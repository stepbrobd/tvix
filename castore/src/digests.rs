use bytes::Bytes;
use data_encoding::BASE64;
use thiserror::Error;

pub const B3_LEN: usize = blake3::OUT_LEN;

#[derive(PartialEq, Eq, Hash)]
pub struct B3Digest([u8; B3_LEN]);

// TODO: allow converting these errors to crate::Error
#[derive(Error, Debug, PartialEq)]
pub enum Error {
    #[error("invalid digest length: {0}")]
    InvalidDigestLen(usize),
}

impl B3Digest {
    pub fn as_slice(&self) -> &[u8] {
        &self.0[..]
    }
}

impl From<B3Digest> for bytes::Bytes {
    fn from(val: B3Digest) -> Self {
        Bytes::copy_from_slice(&val.0)
    }
}

impl From<blake3::Hash> for B3Digest {
    fn from(value: blake3::Hash) -> Self {
        Self(*value.as_bytes())
    }
}
impl From<digest::Output<blake3::Hasher>> for B3Digest {
    fn from(value: digest::Output<blake3::Hasher>) -> Self {
        Self(value.into())
    }
}

impl TryFrom<&[u8]> for B3Digest {
    type Error = Error;

    // constructs a [B3Digest] from a &[u8].
    // Returns an error if the digest has the wrong length.
    fn try_from(value: &[u8]) -> Result<Self, Self::Error> {
        Ok(Self(
            value
                .try_into()
                .map_err(|_e| Error::InvalidDigestLen(value.len()))?,
        ))
    }
}

impl TryFrom<bytes::Bytes> for B3Digest {
    type Error = Error;

    fn try_from(value: bytes::Bytes) -> Result<Self, Self::Error> {
        value[..].try_into()
    }
}

impl TryFrom<Vec<u8>> for B3Digest {
    type Error = Error;

    fn try_from(value: Vec<u8>) -> Result<Self, Self::Error> {
        value[..].try_into()
    }
}

impl From<&[u8; B3_LEN]> for B3Digest {
    fn from(value: &[u8; B3_LEN]) -> Self {
        Self(*value)
    }
}

impl From<B3Digest> for [u8; B3_LEN] {
    fn from(value: B3Digest) -> Self {
        value.0
    }
}

impl Clone for B3Digest {
    fn clone(&self) -> Self {
        Self(self.0.to_owned())
    }
}

impl std::fmt::Display for B3Digest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "b3:{}", BASE64.encode(&self.0))
    }
}

impl std::fmt::Debug for B3Digest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "b3:{}", BASE64.encode(&self.0))
    }
}
