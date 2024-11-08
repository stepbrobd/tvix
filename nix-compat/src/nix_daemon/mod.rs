pub mod worker_protocol;

use std::io::Result;

use types::UnkeyedValidPathInfo;

use crate::store_path::StorePath;

pub mod handler;
pub mod types;

/// Represents all possible operations over the nix-daemon protocol.
pub trait NixDaemonIO {
    fn query_path_info(
        &self,
        path: &StorePath<String>,
    ) -> impl std::future::Future<Output = Result<Option<UnkeyedValidPathInfo>>> + Send;
}
