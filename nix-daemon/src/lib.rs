use std::{io::Result, sync::Arc};

use nix_compat::{
    nix_daemon::{types::UnkeyedValidPathInfo, NixDaemonIO},
    nixbase32,
    store_path::StorePath,
};
use tvix_store::{path_info::PathInfo, pathinfoservice::PathInfoService};

#[allow(dead_code)]
pub struct TvixDaemon {
    path_info_service: Arc<dyn PathInfoService>,
}

impl TvixDaemon {
    pub fn new(path_info_service: Arc<dyn PathInfoService>) -> Self {
        Self { path_info_service }
    }
}

/// Implements [NixDaemonIO] backed by tvix services.
impl NixDaemonIO for TvixDaemon {
    async fn query_path_info(
        &self,
        path: &StorePath<String>,
    ) -> Result<Option<UnkeyedValidPathInfo>> {
        match self.path_info_service.get(*path.digest()).await? {
            Some(path_info) => Ok(Some(into_unkeyed_path_info(path_info))),
            None => Ok(None),
        }
    }
}

// PathInfo lives in the tvix-store crate, but does not depend on nix-compat's wire feature,
// while UnkeyedValidPathInfo is only available if that feature is enabled. To avoid complexity
// we manually convert as opposed to creating a From<PathInfo>.
fn into_unkeyed_path_info(info: PathInfo) -> UnkeyedValidPathInfo {
    UnkeyedValidPathInfo {
        deriver: info.deriver,
        nar_hash: nixbase32::encode(&info.nar_sha256),
        references: info.references,
        registration_time: 0,
        nar_size: info.nar_size,
        ultimate: false,
        signatures: info.signatures,
        ca: info.ca,
    }
}
