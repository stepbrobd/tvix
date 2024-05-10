use std::num::NonZeroUsize;
use std::sync::Arc;
use tokio::sync::RwLock;

use async_stream::try_stream;
use futures::stream::BoxStream;
use lru::LruCache;
use tonic::async_trait;

use crate::proto::PathInfo;
use tvix_castore::Error;

use super::PathInfoService;

pub struct LruPathInfoService {
    lru: Arc<RwLock<LruCache<[u8; 20], PathInfo>>>,
}

impl LruPathInfoService {
    pub fn with_capacity(capacity: NonZeroUsize) -> Self {
        Self {
            lru: Arc::new(RwLock::new(LruCache::new(capacity))),
        }
    }
}

#[async_trait]
impl PathInfoService for LruPathInfoService {
    async fn get(&self, digest: [u8; 20]) -> Result<Option<PathInfo>, Error> {
        Ok(self.lru.write().await.get(&digest).cloned())
    }

    async fn put(&self, path_info: PathInfo) -> Result<PathInfo, Error> {
        // call validate
        let store_path = path_info
            .validate()
            .map_err(|e| Error::InvalidRequest(format!("invalid PathInfo: {}", e)))?;

        self.lru
            .write()
            .await
            .put(*store_path.digest(), path_info.clone());

        Ok(path_info)
    }

    fn list(&self) -> BoxStream<'static, Result<PathInfo, Error>> {
        let lru = self.lru.clone();
        Box::pin(try_stream! {
            let lru = lru.read().await;
            let it = lru.iter();

            for (_k,v) in it {
                yield v.clone()
            }
        })
    }
}

#[cfg(test)]
mod test {
    use std::num::NonZeroUsize;

    use crate::{
        pathinfoservice::{LruPathInfoService, PathInfoService},
        proto::PathInfo,
        tests::fixtures::PATH_INFO_WITH_NARINFO,
    };
    use lazy_static::lazy_static;
    use tvix_castore::proto as castorepb;

    lazy_static! {
        static ref PATHINFO_1: PathInfo = PATH_INFO_WITH_NARINFO.clone();
        static ref PATHINFO_1_DIGEST: [u8; 20] = [0; 20];
        static ref PATHINFO_2: PathInfo = {
            let mut p = PATHINFO_1.clone();
            let root_node = p.node.as_mut().unwrap();
            if let castorepb::Node { node: Some(node) } = root_node {
                let n = node.to_owned();
                *node = n.rename("11111111111111111111111111111111-dummy2".into());
            } else {
                unreachable!()
            }
            p
        };
        static ref PATHINFO_2_DIGEST: [u8; 20] = *(PATHINFO_2.validate().unwrap()).digest();
    }

    #[tokio::test]
    async fn evict() {
        let svc = LruPathInfoService::with_capacity(NonZeroUsize::new(1).unwrap());

        // pathinfo_1 should not be there
        assert!(svc
            .get(*PATHINFO_1_DIGEST)
            .await
            .expect("no error")
            .is_none());

        // insert it
        svc.put(PATHINFO_1.clone()).await.expect("no error");

        // now it should be there.
        assert_eq!(
            Some(PATHINFO_1.clone()),
            svc.get(*PATHINFO_1_DIGEST).await.expect("no error")
        );

        // insert pathinfo_2. This will evict pathinfo 1
        svc.put(PATHINFO_2.clone()).await.expect("no error");

        // now pathinfo 2 should be there.
        assert_eq!(
            Some(PATHINFO_2.clone()),
            svc.get(*PATHINFO_2_DIGEST).await.expect("no error")
        );

        // … but pathinfo 1 not anymore.
        assert!(svc
            .get(*PATHINFO_1_DIGEST)
            .await
            .expect("no error")
            .is_none());
    }
}
