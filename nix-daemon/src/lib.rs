use std::sync::Arc;

use nix_compat::nix_daemon::NixDaemonIO;
use tvix_store::pathinfoservice::PathInfoService;

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
impl NixDaemonIO for TvixDaemon {}
