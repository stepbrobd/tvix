use super::{dumb_seeker::DumbSeeker, BlobReader, BlobService, BlobWriter};
use crate::{proto, B3Digest};
use futures::sink::{SinkExt, SinkMapErr};
use std::{collections::VecDeque, io};
use tokio::{net::UnixStream, task::JoinHandle};
use tokio_stream::{wrappers::ReceiverStream, StreamExt};
use tokio_util::{
    io::{CopyToBytes, SinkWriter, SyncIoBridge},
    sync::{PollSendError, PollSender},
};
use tonic::{transport::Channel, Code, Status, Streaming};
use tracing::instrument;

/// Connects to a (remote) tvix-store BlobService over gRPC.
#[derive(Clone)]
pub struct GRPCBlobService {
    /// A handle into the active tokio runtime. Necessary to spawn tasks.
    tokio_handle: tokio::runtime::Handle,

    /// The internal reference to a gRPC client.
    /// Cloning it is cheap, and it internally handles concurrent requests.
    grpc_client: proto::blob_service_client::BlobServiceClient<Channel>,
}

impl GRPCBlobService {
    /// construct a [GRPCBlobService] from a [proto::blob_service_client::BlobServiceClient].
    /// panics if called outside the context of a tokio runtime.
    pub fn from_client(
        grpc_client: proto::blob_service_client::BlobServiceClient<Channel>,
    ) -> Self {
        Self {
            tokio_handle: tokio::runtime::Handle::current(),
            grpc_client,
        }
    }
}

impl BlobService for GRPCBlobService {
    /// Constructs a [GRPCBlobService] from the passed [url::Url]:
    /// - scheme has to match `grpc+*://`.
    ///   That's normally grpc+unix for unix sockets, and grpc+http(s) for the HTTP counterparts.
    /// - In the case of unix sockets, there must be a path, but may not be a host.
    /// - In the case of non-unix sockets, there must be a host, but no path.
    fn from_url(url: &url::Url) -> Result<Self, crate::Error> {
        // Start checking for the scheme to start with grpc+.
        match url.scheme().strip_prefix("grpc+") {
            None => Err(crate::Error::StorageError("invalid scheme".to_string())),
            Some(rest) => {
                if rest == "unix" {
                    if url.host_str().is_some() {
                        return Err(crate::Error::StorageError(
                            "host may not be set".to_string(),
                        ));
                    }
                    let path = url.path().to_string();
                    let channel = tonic::transport::Endpoint::try_from("http://[::]:50051") // doesn't matter
                        .unwrap()
                        .connect_with_connector_lazy(tower::service_fn(
                            move |_: tonic::transport::Uri| UnixStream::connect(path.clone()),
                        ));
                    let grpc_client = proto::blob_service_client::BlobServiceClient::new(channel);
                    Ok(Self::from_client(grpc_client))
                } else {
                    // ensure path is empty, not supported with gRPC.
                    if !url.path().is_empty() {
                        return Err(crate::Error::StorageError(
                            "path may not be set".to_string(),
                        ));
                    }

                    // clone the uri, and drop the grpc+ from the scheme.
                    // Recreate a new uri with the `grpc+` prefix dropped from the scheme.
                    // We can't use `url.set_scheme(rest)`, as it disallows
                    // setting something http(s) that previously wasn't.
                    let url = {
                        let url_str = url.to_string();
                        let s_stripped = url_str.strip_prefix("grpc+").unwrap();
                        url::Url::parse(s_stripped).unwrap()
                    };
                    let channel = tonic::transport::Endpoint::try_from(url.to_string())
                        .unwrap()
                        .connect_lazy();

                    let grpc_client = proto::blob_service_client::BlobServiceClient::new(channel);
                    Ok(Self::from_client(grpc_client))
                }
            }
        }
    }

    #[instrument(skip(self, digest), fields(blob.digest=%digest))]
    fn has(&self, digest: &B3Digest) -> Result<bool, crate::Error> {
        // Get a new handle to the gRPC client, and copy the digest.
        let mut grpc_client = self.grpc_client.clone();
        let digest = digest.clone();

        let task: JoinHandle<Result<_, Status>> = self.tokio_handle.spawn(async move {
            Ok(grpc_client
                .stat(proto::StatBlobRequest {
                    digest: digest.into(),
                    ..Default::default()
                })
                .await?
                .into_inner())
        });

        match self.tokio_handle.block_on(task)? {
            Ok(_blob_meta) => Ok(true),
            Err(e) if e.code() == Code::NotFound => Ok(false),
            Err(e) => Err(crate::Error::StorageError(e.to_string())),
        }
    }

    // On success, this returns a Ok(Some(io::Read)), which can be used to read
    // the contents of the Blob, identified by the digest.
    fn open_read(&self, digest: &B3Digest) -> Result<Option<Box<dyn BlobReader>>, crate::Error> {
        // Get a new handle to the gRPC client, and copy the digest.
        let mut grpc_client = self.grpc_client.clone();
        let digest = digest.clone();

        // Construct the task that'll send out the request and return the stream
        // the gRPC client should use to send [proto::BlobChunk], or an error if
        // the blob doesn't exist.
        let task: JoinHandle<Result<Streaming<proto::BlobChunk>, Status>> =
            self.tokio_handle.spawn(async move {
                let stream = grpc_client
                    .read(proto::ReadBlobRequest {
                        digest: digest.into(),
                    })
                    .await?
                    .into_inner();

                Ok(stream)
            });

        // This runs the task to completion, which on success will return a stream.
        // On reading from it, we receive individual [proto::BlobChunk], so we
        // massage this to a stream of bytes,
        // then create an [AsyncRead], which we'll turn into a [io::Read],
        // that's returned from the function.
        match self.tokio_handle.block_on(task)? {
            Ok(stream) => {
                // map the stream of proto::BlobChunk to bytes.
                let data_stream = stream.map(|x| {
                    x.map(|x| VecDeque::from(x.data.to_vec()))
                        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))
                });

                // Use StreamReader::new to convert to an AsyncRead.
                let data_reader = tokio_util::io::StreamReader::new(data_stream);

                // Use SyncIoBridge to turn it into a sync Read.
                let sync_reader = tokio_util::io::SyncIoBridge::new(data_reader);
                Ok(Some(Box::new(DumbSeeker::new(sync_reader))))
            }
            Err(e) if e.code() == Code::NotFound => Ok(None),
            Err(e) => Err(crate::Error::StorageError(e.to_string())),
        }
    }

    /// Returns a BlobWriter, that'll internally wrap each write in a
    // [proto::BlobChunk], which is send to the gRPC server.
    fn open_write(&self) -> Box<dyn BlobWriter> {
        let mut grpc_client = self.grpc_client.clone();

        // set up an mpsc channel passing around Bytes.
        let (tx, rx) = tokio::sync::mpsc::channel::<bytes::Bytes>(10);

        // bytes arriving on the RX side are wrapped inside a
        // [proto::BlobChunk], and a [ReceiverStream] is constructed.
        let blobchunk_stream = ReceiverStream::new(rx).map(|x| proto::BlobChunk { data: x });

        // That receiver stream is used as a stream in the gRPC BlobService.put rpc call.
        let task: JoinHandle<Result<_, Status>> = self
            .tokio_handle
            .spawn(async move { Ok(grpc_client.put(blobchunk_stream).await?.into_inner()) });

        // The tx part of the channel is converted to a sink of byte chunks.

        // We need to make this a function pointer, not a closure.
        fn convert_error(_: PollSendError<bytes::Bytes>) -> io::Error {
            io::Error::from(io::ErrorKind::BrokenPipe)
        }

        let sink = PollSender::new(tx)
            .sink_map_err(convert_error as fn(PollSendError<bytes::Bytes>) -> io::Error);
        // We need to explicitly cast here, otherwise rustc does error with "expected fn pointer, found fn item"

        // … which is turned into an [tokio::io::AsyncWrite].
        let async_writer = SinkWriter::new(CopyToBytes::new(sink));
        // … which is then turned into a [io::Write].
        let writer = SyncIoBridge::new(async_writer);

        Box::new(GRPCBlobWriter {
            tokio_handle: self.tokio_handle.clone(),
            task_and_writer: Some((task, writer)),
            digest: None,
        })
    }
}

type BridgedWriter = SyncIoBridge<
    SinkWriter<
        CopyToBytes<
            SinkMapErr<PollSender<bytes::Bytes>, fn(PollSendError<bytes::Bytes>) -> io::Error>,
        >,
    >,
>;

pub struct GRPCBlobWriter {
    /// A handle into the active tokio runtime. Necessary to block on the task
    /// containing the put request.
    tokio_handle: tokio::runtime::Handle,

    /// The task containing the put request, and the inner writer, if we're still writing.
    task_and_writer: Option<(
        JoinHandle<Result<proto::PutBlobResponse, Status>>,
        BridgedWriter,
    )>,

    /// The digest that has been returned, if we successfully closed.
    digest: Option<B3Digest>,
}

impl BlobWriter for GRPCBlobWriter {
    fn close(&mut self) -> Result<B3Digest, crate::Error> {
        if self.task_and_writer.is_none() {
            // if we're already closed, return the b3 digest, which must exist.
            // If it doesn't, we already closed and failed once, and didn't handle the error.
            match &self.digest {
                Some(digest) => Ok(digest.clone()),
                None => Err(crate::Error::StorageError(
                    "previously closed with error".to_string(),
                )),
            }
        } else {
            let (task, mut writer) = self.task_and_writer.take().unwrap();

            // invoke shutdown, so the inner writer closes its internal tx side of
            // the channel.
            writer
                .shutdown()
                .map_err(|e| crate::Error::StorageError(e.to_string()))?;

            // block on the RPC call to return.
            // This ensures all chunks are sent out, and have been received by the
            // backend.
            match self.tokio_handle.block_on(task)? {
                Ok(resp) => {
                    // return the digest from the response, and store it in self.digest for subsequent closes.
                    let digest: B3Digest = resp.digest.try_into().map_err(|_| {
                        crate::Error::StorageError(
                            "invalid root digest length in response".to_string(),
                        )
                    })?;
                    self.digest = Some(digest.clone());
                    Ok(digest)
                }
                Err(e) => Err(crate::Error::StorageError(e.to_string())),
            }
        }
    }
}

impl io::Write for GRPCBlobWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match &mut self.task_and_writer {
            None => Err(io::Error::new(
                io::ErrorKind::NotConnected,
                "already closed",
            )),
            Some((_, ref mut writer)) => writer.write(buf),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match &mut self.task_and_writer {
            None => Err(io::Error::new(
                io::ErrorKind::NotConnected,
                "already closed",
            )),
            Some((_, ref mut writer)) => writer.flush(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::thread;

    use tempfile::TempDir;
    use tokio::net::UnixListener;
    use tokio::task;
    use tokio::time;
    use tokio_stream::wrappers::UnixListenerStream;

    use crate::blobservice::MemoryBlobService;
    use crate::proto::GRPCBlobServiceWrapper;
    use crate::tests::fixtures;

    use super::BlobService;
    use super::GRPCBlobService;

    /// This uses the wrong scheme
    #[test]
    fn test_invalid_scheme() {
        let url = url::Url::parse("http://foo.example/test").expect("must parse");

        assert!(GRPCBlobService::from_url(&url).is_err());
    }

    /// This uses the correct scheme for a unix socket.
    /// The fact that /path/to/somewhere doesn't exist yet is no problem, because we connect lazily.
    #[tokio::test]
    async fn test_valid_unix_path() {
        let url = url::Url::parse("grpc+unix:///path/to/somewhere").expect("must parse");

        assert!(GRPCBlobService::from_url(&url).is_ok());
    }

    /// This uses the correct scheme for a unix socket,
    /// but sets a host, which is unsupported.
    #[tokio::test]
    async fn test_invalid_unix_path_with_domain() {
        let url =
            url::Url::parse("grpc+unix://host.example/path/to/somewhere").expect("must parse");

        assert!(GRPCBlobService::from_url(&url).is_err());
    }

    /// This uses the correct scheme for a HTTP server.
    /// The fact that nothing is listening there is no problem, because we connect lazily.
    #[tokio::test]
    async fn test_valid_http() {
        let url = url::Url::parse("grpc+http://localhost").expect("must parse");

        assert!(GRPCBlobService::from_url(&url).is_ok());
    }

    /// This uses the correct scheme for a HTTPS server.
    /// The fact that nothing is listening there is no problem, because we connect lazily.
    #[tokio::test]
    async fn test_valid_https() {
        let url = url::Url::parse("grpc+https://localhost").expect("must parse");

        assert!(GRPCBlobService::from_url(&url).is_ok());
    }

    /// This uses the correct scheme, but also specifies
    /// an additional path, which is not supported for gRPC.
    /// The fact that nothing is listening there is no problem, because we connect lazily.
    #[tokio::test]
    async fn test_invalid_http_with_path() {
        let url = url::Url::parse("grpc+https://localhost/some-path").expect("must parse");

        assert!(GRPCBlobService::from_url(&url).is_err());
    }

    /// This uses the correct scheme for a unix socket, and provides a server on the other side.
    #[tokio::test]
    async fn test_valid_unix_path_ping_pong() {
        let tmpdir = TempDir::new().unwrap();
        let path = tmpdir.path().join("daemon");

        // let mut join_set = JoinSet::new();

        // prepare a client
        let client = {
            let mut url = url::Url::parse("grpc+unix:///path/to/somewhere").expect("must parse");
            url.set_path(path.to_str().unwrap());
            GRPCBlobService::from_url(&url).expect("must succeed")
        };

        let path_copy = path.clone();

        // Spin up a server, in a thread far away, which spawns its own tokio runtime,
        // and blocks on the task.
        thread::spawn(move || {
            // Create the runtime
            let rt = tokio::runtime::Runtime::new().unwrap();
            // Get a handle from this runtime
            let handle = rt.handle();

            let task = handle.spawn(async {
                let uds = UnixListener::bind(path_copy).unwrap();
                let uds_stream = UnixListenerStream::new(uds);

                // spin up a new server
                let mut server = tonic::transport::Server::builder();
                let router =
                    server.add_service(crate::proto::blob_service_server::BlobServiceServer::new(
                        GRPCBlobServiceWrapper::from(
                            Arc::new(MemoryBlobService::default()) as Arc<dyn BlobService>
                        ),
                    ));
                router.serve_with_incoming(uds_stream).await
            });

            handle.block_on(task)
        });

        // wait for the socket to be created
        {
            let mut socket_created = false;
            for _try in 1..20 {
                if path.exists() {
                    socket_created = true;
                    break;
                }
                tokio::time::sleep(time::Duration::from_millis(20)).await;
            }

            assert!(
                socket_created,
                "expected socket path to eventually get created, but never happened"
            );
        }

        let has = task::spawn_blocking(move || {
            client
                .has(&fixtures::BLOB_A_DIGEST)
                .expect("must not be err")
        })
        .await
        .expect("must not be err");
        assert!(!has);
    }
}
