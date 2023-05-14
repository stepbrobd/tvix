use super::PathInfoService;
use crate::{proto, Error};
use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
};

#[derive(Default)]
pub struct MemoryPathInfoService {
    db: Arc<RwLock<HashMap<[u8; 20], proto::PathInfo>>>,
}

impl PathInfoService for MemoryPathInfoService {
    fn get(&self, digest: [u8; 20]) -> Result<Option<proto::PathInfo>, Error> {
        let db = self.db.read().unwrap();

        match db.get(&digest) {
            None => Ok(None),
            Some(path_info) => Ok(Some(path_info.clone())),
        }
    }

    fn put(&self, path_info: proto::PathInfo) -> Result<proto::PathInfo, Error> {
        // Call validate on the received PathInfo message.
        match path_info.validate() {
            Err(e) => Err(Error::InvalidRequest(format!(
                "failed to validate PathInfo: {}",
                e
            ))),

            // In case the PathInfo is valid, and we were able to extract a NixPath, store it in the database.
            // This overwrites existing PathInfo objects.
            Ok(nix_path) => {
                let mut db = self.db.write().unwrap();
                db.insert(nix_path.digest, path_info.clone());

                Ok(path_info)
            }
        }
    }
}
