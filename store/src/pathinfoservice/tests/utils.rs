use std::sync::Arc;

use tonic::transport::{Endpoint, Server, Uri};

use crate::{
    pathinfoservice::{GRPCPathInfoService, MemoryPathInfoService, PathInfoService},
    proto::{
        path_info_service_client::PathInfoServiceClient,
        path_info_service_server::PathInfoServiceServer, GRPCPathInfoServiceWrapper,
    },
    tests::fixtures::{blob_service, directory_service},
};

/// Constructs and returns a gRPC PathInfoService.
/// We also return memory-based {Blob,Directory}Service,
/// as the consumer of this function accepts a 3-tuple.
pub async fn make_grpc_path_info_service_client() -> super::BSDSPS {
    let (left, right) = tokio::io::duplex(64);

    let blob_service = blob_service();
    let directory_service = directory_service();

    // spin up a server, which will only connect once, to the left side.
    tokio::spawn({
        let blob_service = blob_service.clone();
        let directory_service = directory_service.clone();
        async move {
            let path_info_service: Arc<dyn PathInfoService> =
                Arc::from(MemoryPathInfoService::new(blob_service, directory_service));

            // spin up a new DirectoryService
            let mut server = Server::builder();
            let router = server.add_service(PathInfoServiceServer::new(
                GRPCPathInfoServiceWrapper::new(path_info_service),
            ));

            router
                .serve_with_incoming(tokio_stream::once(Ok::<_, std::io::Error>(left)))
                .await
        }
    });

    // Create a client, connecting to the right side. The URI is unused.
    let mut maybe_right = Some(right);

    let path_info_service = Box::new(GRPCPathInfoService::from_client(
        PathInfoServiceClient::new(
            Endpoint::try_from("http://[::]:50051")
                .unwrap()
                .connect_with_connector(tower::service_fn(move |_: Uri| {
                    let right = maybe_right.take().unwrap();
                    async move { Ok::<_, std::io::Error>(right) }
                }))
                .await
                .unwrap(),
        ),
    ));

    (blob_service, directory_service, path_info_service)
}