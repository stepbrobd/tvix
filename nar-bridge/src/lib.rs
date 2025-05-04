use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{head, put};
use axum::{routing::get, Router};
use lru::LruCache;
use nix_compat::nix_http;
use parking_lot::RwLock;
use std::num::NonZeroUsize;
use std::sync::Arc;
use tvix_castore::blobservice::BlobService;
use tvix_castore::directoryservice::DirectoryService;
use tvix_castore::Node;
use tvix_store::pathinfoservice::PathInfoService;

mod nar;
mod narinfo;

#[derive(Clone)]
pub struct AppState {
    blob_service: Arc<dyn BlobService>,
    directory_service: Arc<dyn DirectoryService>,
    path_info_service: Arc<dyn PathInfoService>,

    /// Lookup table from NarHash to [Node], necessary to populate the root_node
    /// field of the PathInfo when processing the narinfo upload.
    root_nodes: Arc<RwLock<LruCache<[u8; 32], Node>>>,
}

impl AppState {
    pub fn new(
        blob_service: Arc<dyn BlobService>,
        directory_service: Arc<dyn DirectoryService>,
        path_info_service: Arc<dyn PathInfoService>,
        root_nodes_cache_capacity: NonZeroUsize,
    ) -> Self {
        Self {
            blob_service,
            directory_service,
            path_info_service,
            root_nodes: Arc::new(RwLock::new(LruCache::new(root_nodes_cache_capacity))),
        }
    }
}

pub fn gen_router(priority: u64) -> Router<AppState> {
    let router = Router::new()
        .route("/", get(root))
        .route("/nar/:nar_str", get(four_o_four))
        .route("/nar/:nar_str", head(nar::head_root_nodes))
        .route("/nar/:nar_str", put(nar::put))
        .route("/nar/tvix-castore/:root_node_enc", get(nar::get_head))
        .route("/nar/tvix-castore/:root_node_enc", head(nar::get_head))
        .route("/:narinfo_str", get(narinfo::get))
        .route("/:narinfo_str", head(narinfo::head))
        .route("/:narinfo_str", put(narinfo::put))
        .route("/nix-cache-info", get(move || nix_cache_info(priority)));

    router.layer(tower_http::compression::CompressionLayer::new())
}

async fn root() -> &'static str {
    "Hello from nar-bridge"
}

async fn four_o_four() -> Result<(), StatusCode> {
    Err(StatusCode::NOT_FOUND)
}

async fn nix_cache_info(priority: u64) -> impl IntoResponse {
    (
        [("Content-Type", nix_http::MIME_TYPE_CACHE_INFO)],
        format!(
            "StoreDir: /nix/store\nWantMassQuery: 1\nPriority: {}\n",
            priority
        ),
    )
}
