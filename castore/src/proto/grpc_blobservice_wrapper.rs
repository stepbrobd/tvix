use crate::{blobservice::BlobService, B3Digest};
use core::pin::pin;
use futures::{stream::BoxStream, TryFutureExt};
use std::{
    collections::VecDeque,
    ops::{Deref, DerefMut},
};
use tokio_stream::StreamExt;
use tokio_util::io::ReaderStream;
use tonic::{async_trait, Request, Response, Status, Streaming};
use tracing::{instrument, warn, Span};

pub struct GRPCBlobServiceWrapper<T> {
    blob_service: T,
}

impl<T> GRPCBlobServiceWrapper<T> {
    pub fn new(blob_service: T) -> Self {
        Self { blob_service }
    }
}

// This is necessary because bytes::BytesMut comes up with
// a default 64 bytes capacity that cannot be changed
// easily if you assume a bytes::BufMut trait implementation
// Therefore, we override the Default implementation here
// TODO(raitobezarius?): upstream me properly
struct BytesMutWithDefaultCapacity<const N: usize> {
    inner: bytes::BytesMut,
}

impl<const N: usize> Deref for BytesMutWithDefaultCapacity<N> {
    type Target = bytes::BytesMut;
    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl<const N: usize> DerefMut for BytesMutWithDefaultCapacity<N> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

impl<const N: usize> Default for BytesMutWithDefaultCapacity<N> {
    fn default() -> Self {
        BytesMutWithDefaultCapacity {
            inner: bytes::BytesMut::with_capacity(N),
        }
    }
}

impl<const N: usize> bytes::Buf for BytesMutWithDefaultCapacity<N> {
    fn remaining(&self) -> usize {
        self.inner.remaining()
    }

    fn chunk(&self) -> &[u8] {
        self.inner.chunk()
    }

    fn advance(&mut self, cnt: usize) {
        self.inner.advance(cnt);
    }
}

unsafe impl<const N: usize> bytes::BufMut for BytesMutWithDefaultCapacity<N> {
    fn remaining_mut(&self) -> usize {
        self.inner.remaining_mut()
    }

    unsafe fn advance_mut(&mut self, cnt: usize) {
        self.inner.advance_mut(cnt);
    }

    fn chunk_mut(&mut self) -> &mut bytes::buf::UninitSlice {
        self.inner.chunk_mut()
    }
}

#[async_trait]
impl<T> super::blob_service_server::BlobService for GRPCBlobServiceWrapper<T>
where
    T: Deref<Target = dyn BlobService> + Send + Sync + 'static,
{
    // https://github.com/tokio-rs/tokio/issues/2723#issuecomment-1534723933
    type ReadStream = BoxStream<'static, Result<super::BlobChunk, Status>>;

    #[instrument(skip_all)]
    async fn stat(
        &self,
        request: Request<super::StatBlobRequest>,
    ) -> Result<Response<super::StatBlobResponse>, Status> {
        let rq = request.into_inner();
        let req_digest: B3Digest = rq
            .digest
            .try_into()
            .map_err(|_e| Status::invalid_argument("invalid digest length"))?;

        let span = Span::current();
        span.record("blob.digest", req_digest.to_string());

        match self.blob_service.chunks(&req_digest).await {
            Ok(None) => Err(Status::not_found(format!("blob {} not found", &req_digest))),
            Ok(Some(chunk_metas)) => Ok(Response::new(super::StatBlobResponse {
                chunks: chunk_metas,
                ..Default::default()
            })),
            Err(e) => {
                warn!(err=%e, "failed to request chunks");
                Err(e.into())
            }
        }
    }

    #[instrument(skip_all)]
    async fn read(
        &self,
        request: Request<super::ReadBlobRequest>,
    ) -> Result<Response<Self::ReadStream>, Status> {
        let rq = request.into_inner();

        let req_digest: B3Digest = rq
            .digest
            .try_into()
            .map_err(|_e| Status::invalid_argument("invalid digest length"))?;

        let span = Span::current();
        span.record("blob.digest", req_digest.to_string());

        match self.blob_service.open_read(&req_digest).await {
            Ok(Some(r)) => {
                let chunks_stream =
                    ReaderStream::new(r).map(|chunk| Ok(super::BlobChunk { data: chunk? }));
                Ok(Response::new(Box::pin(chunks_stream)))
            }
            Ok(None) => Err(Status::not_found(format!("blob {} not found", &req_digest))),
            Err(e) => {
                warn!(err=%e, "failed to call open_read");
                Err(e.into())
            }
        }
    }

    #[instrument(skip_all)]
    async fn put(
        &self,
        request: Request<Streaming<super::BlobChunk>>,
    ) -> Result<Response<super::PutBlobResponse>, Status> {
        let req_inner = request.into_inner();

        let data_stream = req_inner.map(|x| {
            x.map(|x| VecDeque::from(x.data.to_vec()))
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))
        });

        let mut data_reader = tokio_util::io::StreamReader::new(data_stream);

        let mut blob_writer = pin!(self.blob_service.open_write().await);

        tokio::io::copy(&mut data_reader, &mut blob_writer)
            .await
            .map_err(|e| {
                warn!("error copying: {}", e);
                Status::internal("error copying")
            })?;

        let digest = blob_writer
            .close()
            .map_err(|e| {
                warn!("error closing stream: {}", e);
                Status::internal("error closing stream")
            })
            .await?;

        Ok(Response::new(super::PutBlobResponse {
            digest: digest.into(),
        }))
    }
}
