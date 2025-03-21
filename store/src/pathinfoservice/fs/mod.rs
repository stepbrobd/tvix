use futures::stream::BoxStream;
use futures::StreamExt;
use nix_compat::store_path::StorePathRef;
use tonic::async_trait;
use tvix_castore::fs::{RootNodes, TvixStoreFs};
use tvix_castore::{blobservice::BlobService, directoryservice::DirectoryService};
use tvix_castore::{Error, Node, PathComponent};

use super::PathInfoService;

/// Helper to construct a [TvixStoreFs] from a [BlobService], [DirectoryService]
/// and [PathInfoService].
/// This avoids users to have to interact with the wrapper struct directly, as
/// it leaks into the type signature of TvixStoreFS.
pub fn make_fs<BS, DS, PS>(
    blob_service: BS,
    directory_service: DS,
    path_info_service: PS,
    list_root: bool,
    show_xattr: bool,
) -> TvixStoreFs<BS, DS, RootNodesWrapper<PS>>
where
    BS: BlobService + Send + Clone + 'static,
    DS: DirectoryService + Send + Clone + 'static,
    PS: PathInfoService + Send + Sync + Clone + 'static,
{
    TvixStoreFs::new(
        blob_service,
        directory_service,
        RootNodesWrapper(path_info_service),
        list_root,
        show_xattr,
    )
}

/// Wrapper to satisfy Rust's orphan rules for trait implementations, as
/// RootNodes is coming from the [tvix-castore] crate.
#[doc(hidden)]
#[derive(Clone, Debug)]
pub struct RootNodesWrapper<T>(pub(crate) T);

/// Implements root node lookup for any [PathInfoService]. This represents a flat
/// directory structure like /nix/store where each entry in the root filesystem
/// directory corresponds to a CA node.
#[cfg(any(feature = "fuse", feature = "virtiofs"))]
#[async_trait]
impl<T> RootNodes for RootNodesWrapper<T>
where
    T: PathInfoService + Send + Sync,
{
    async fn get_by_basename(&self, name: &PathComponent) -> Result<Option<Node>, Error> {
        let Ok(store_path) = StorePathRef::from_bytes(name.as_ref()) else {
            return Ok(None);
        };

        Ok(self
            .0
            .get(*store_path.digest())
            .await?
            .map(|path_info| path_info.node))
    }

    fn list(&self) -> BoxStream<Result<(PathComponent, Node), Error>> {
        Box::pin(self.0.list().map(|result| {
            result.map(|path_info| {
                let basename = path_info.store_path.to_string();
                (
                    basename
                        .as_str()
                        .try_into()
                        .expect("Tvix bug: StorePath must be PathComponent"),
                    path_info.node,
                )
            })
        }))
    }
}
