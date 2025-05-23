use crate::composition::{Registry, ServiceBuilder};
use crate::{B3Digest, Directory, Error};

use auto_impl::auto_impl;
use futures::stream::BoxStream;
use tonic::async_trait;
mod combinators;
mod directory_graph;
mod from_addr;
mod grpc;
mod memory;
mod object_store;
mod order_validator;
mod redb;
mod simple_putter;
#[cfg(test)]
pub mod tests;
mod traverse;
mod utils;

pub use self::combinators::{Cache, CacheConfig};
pub use self::directory_graph::{DirectoryGraph, ValidatedDirectoryGraph};
pub use self::from_addr::from_addr;
pub use self::grpc::{GRPCDirectoryService, GRPCDirectoryServiceConfig};
pub use self::memory::{MemoryDirectoryService, MemoryDirectoryServiceConfig};
pub use self::object_store::{ObjectStoreDirectoryService, ObjectStoreDirectoryServiceConfig};
pub use self::order_validator::{LeavesToRootValidator, OrderValidator, RootToLeavesValidator};
pub use self::redb::{RedbDirectoryService, RedbDirectoryServiceConfig};
pub use self::simple_putter::SimplePutter;
pub use self::traverse::descend_to;
pub use self::utils::traverse_directory;

/// The base trait all Directory services need to implement.
/// This is a simple get and put of [Directory], returning their
/// digest.
#[async_trait]
#[auto_impl(&, &mut, Arc, Box)]
pub trait DirectoryService: Send + Sync {
    /// Looks up a single Directory message by its digest.
    /// The returned Directory message *must* be valid.
    /// In case the directory is not found, Ok(None) is returned.
    ///
    /// It is okay for certain implementations to only allow retrieval of
    /// Directory digests that are at the "root", aka the last element that's
    /// sent to a DirectoryPutter. This makes sense for implementations bundling
    /// closures of directories together in batches.
    async fn get(&self, digest: &B3Digest) -> Result<Option<Directory>, Error>;
    /// Uploads a single Directory message, and returns the calculated
    /// digest, or an error. An error *must* also be returned if the message is
    /// not valid.
    async fn put(&self, directory: Directory) -> Result<B3Digest, Error>;

    /// Looks up a closure of [Directory].
    /// Ideally this would be a `impl Stream<Item = Result<Directory, Error>>`,
    /// and we'd be able to add a default implementation for it here, but
    /// we can't have that yet.
    ///
    /// This returns a pinned, boxed stream. The pinning allows for it to be polled easily,
    /// and the box allows different underlying stream implementations to be returned since
    /// Rust doesn't support this as a generic in traits yet. This is the same thing that
    /// [async_trait] generates, but for streams instead of futures.
    ///
    /// The individually returned Directory messages *must* be valid.
    /// Directories are sent in an order from the root to the leaves, so that
    /// the receiving side can validate each message to be a connected to the root
    /// that has initially been requested.
    ///
    /// In case the directory can not be found, this should return an empty stream.
    fn get_recursive(
        &self,
        root_directory_digest: &B3Digest,
    ) -> BoxStream<'static, Result<Directory, Error>>;

    /// Allows persisting a closure of [Directory], which is a graph of
    /// connected Directory messages.
    fn put_multiple_start(&self) -> Box<dyn DirectoryPutter + '_>;
}

/// Provides a handle to put a closure of connected [Directory] elements.
///
/// The consumer can periodically call [DirectoryPutter::put], starting from the
/// leaves. Once the root is reached, [DirectoryPutter::close] can be called to
/// retrieve the root digest (or an error).
///
/// DirectoryPutters might be created without a single [DirectoryPutter::put],
/// and then dropped without calling [DirectoryPutter::close],
/// for example when ingesting a path that ends up not pointing to a directory,
/// but a single file or symlink.
#[async_trait]
pub trait DirectoryPutter: Send {
    /// Put a individual [Directory] into the store.
    /// Error semantics and behaviour is up to the specific implementation of
    /// this trait.
    /// Due to bursting, the returned error might refer to an object previously
    /// sent via `put`.
    async fn put(&mut self, directory: Directory) -> Result<(), Error>;

    /// Close the stream, and wait for any errors.
    /// If there's been any invalid Directory message uploaded, and error *must*
    /// be returned.
    async fn close(&mut self) -> Result<B3Digest, Error>;
}

/// Registers the builtin DirectoryService implementations with the registry
pub(crate) fn register_directory_services(reg: &mut Registry) {
    reg.register::<Box<dyn ServiceBuilder<Output = dyn DirectoryService>>, super::directoryservice::ObjectStoreDirectoryServiceConfig>("objectstore");
    reg.register::<Box<dyn ServiceBuilder<Output = dyn DirectoryService>>, super::directoryservice::MemoryDirectoryServiceConfig>("memory");
    reg.register::<Box<dyn ServiceBuilder<Output = dyn DirectoryService>>, super::directoryservice::CacheConfig>("cache");
    reg.register::<Box<dyn ServiceBuilder<Output = dyn DirectoryService>>, super::directoryservice::GRPCDirectoryServiceConfig>("grpc");
    reg.register::<Box<dyn ServiceBuilder<Output = dyn DirectoryService>>, super::directoryservice::RedbDirectoryServiceConfig>("redb");
}
