use crate::pathinfoservice::PathInfo;
use md5::Digest;
use nix_compat::nixhash::{CAHash, NixHash};
use nix_compat::store_path::StorePath;
use std::sync::LazyLock;
use tvix_castore::fixtures::{
    DIRECTORY_COMPLICATED, DUMMY_DIGEST, HELLOWORLD_BLOB_CONTENTS, HELLOWORLD_BLOB_DIGEST,
};
use tvix_castore::Node;

pub const DUMMY_PATH_STR: &str = "00000000000000000000000000000000-dummy";
pub const DUMMY_PATH_DIGEST: [u8; 20] = [0; 20];

pub static DUMMY_PATH: LazyLock<StorePath<String>> =
    LazyLock::new(|| StorePath::from_name_and_digest_fixed("dummy", DUMMY_PATH_DIGEST).unwrap());

pub static CASTORE_NODE_SYMLINK: LazyLock<Node> = LazyLock::new(|| Node::Symlink {
    target: "/nix/store/somewhereelse".try_into().unwrap(),
});

/// The NAR representation of a symlink pointing to `/nix/store/somewhereelse`
pub const NAR_CONTENTS_SYMLINK: [u8; 136] = [
    13, 0, 0, 0, 0, 0, 0, 0, b'n', b'i', b'x', b'-', b'a', b'r', b'c', b'h', b'i', b'v', b'e',
    b'-', b'1', 0, 0, 0, // "nix-archive-1"
    1, 0, 0, 0, 0, 0, 0, 0, b'(', 0, 0, 0, 0, 0, 0, 0, // "("
    4, 0, 0, 0, 0, 0, 0, 0, b't', b'y', b'p', b'e', 0, 0, 0, 0, // "type"
    7, 0, 0, 0, 0, 0, 0, 0, b's', b'y', b'm', b'l', b'i', b'n', b'k', 0, // "symlink"
    6, 0, 0, 0, 0, 0, 0, 0, b't', b'a', b'r', b'g', b'e', b't', 0, 0, // target
    24, 0, 0, 0, 0, 0, 0, 0, b'/', b'n', b'i', b'x', b'/', b's', b't', b'o', b'r', b'e', b'/',
    b's', b'o', b'm', b'e', b'w', b'h', b'e', b'r', b'e', b'e', b'l', b's',
    b'e', // "/nix/store/somewhereelse"
    1, 0, 0, 0, 0, 0, 0, 0, b')', 0, 0, 0, 0, 0, 0, 0, // ")"
];

pub static CASTORE_NODE_HELLOWORLD: LazyLock<Node> = LazyLock::new(|| Node::File {
    digest: HELLOWORLD_BLOB_DIGEST.clone(),
    size: HELLOWORLD_BLOB_CONTENTS.len() as u64,
    executable: false,
});

/// The NAR representation of a regular file with the contents "Hello World!"
pub const NAR_CONTENTS_HELLOWORLD: [u8; 128] = [
    13, 0, 0, 0, 0, 0, 0, 0, b'n', b'i', b'x', b'-', b'a', b'r', b'c', b'h', b'i', b'v', b'e',
    b'-', b'1', 0, 0, 0, // "nix-archive-1"
    1, 0, 0, 0, 0, 0, 0, 0, b'(', 0, 0, 0, 0, 0, 0, 0, // "("
    4, 0, 0, 0, 0, 0, 0, 0, b't', b'y', b'p', b'e', 0, 0, 0, 0, // "type"
    7, 0, 0, 0, 0, 0, 0, 0, b'r', b'e', b'g', b'u', b'l', b'a', b'r', 0, // "regular"
    8, 0, 0, 0, 0, 0, 0, 0, b'c', b'o', b'n', b't', b'e', b'n', b't', b's', // "contents"
    12, 0, 0, 0, 0, 0, 0, 0, b'H', b'e', b'l', b'l', b'o', b' ', b'W', b'o', b'r', b'l', b'd',
    b'!', 0, 0, 0, 0, // "Hello World!"
    1, 0, 0, 0, 0, 0, 0, 0, b')', 0, 0, 0, 0, 0, 0, 0, // ")"
];

pub static CASTORE_NODE_TOO_BIG: LazyLock<Node> = LazyLock::new(|| Node::File {
    digest: HELLOWORLD_BLOB_DIGEST.clone(),
    size: 42, // <- note the wrong size here!
    executable: false,
});
pub static CASTORE_NODE_TOO_SMALL: LazyLock<Node> = LazyLock::new(|| Node::File {
    digest: HELLOWORLD_BLOB_DIGEST.clone(),
    size: 2, // <- note the wrong size here!
    executable: false,
});

pub static CASTORE_NODE_COMPLICATED: LazyLock<Node> = LazyLock::new(|| Node::Directory {
    digest: DIRECTORY_COMPLICATED.digest(),
    size: DIRECTORY_COMPLICATED.size(),
});

/// The NAR representation of a more complicated directory structure.
pub const NAR_CONTENTS_COMPLICATED: [u8; 840] = [
    13, 0, 0, 0, 0, 0, 0, 0, b'n', b'i', b'x', b'-', b'a', b'r', b'c', b'h', b'i', b'v', b'e',
    b'-', b'1', 0, 0, 0, // "nix-archive-1"
    1, 0, 0, 0, 0, 0, 0, 0, b'(', 0, 0, 0, 0, 0, 0, 0, // "("
    4, 0, 0, 0, 0, 0, 0, 0, b't', b'y', b'p', b'e', 0, 0, 0, 0, // "type"
    9, 0, 0, 0, 0, 0, 0, 0, b'd', b'i', b'r', b'e', b'c', b't', b'o', b'r', b'y', 0, 0, 0, 0, 0, 0,
    0, // "directory"
    5, 0, 0, 0, 0, 0, 0, 0, b'e', b'n', b't', b'r', b'y', 0, 0, 0, // "entry"
    1, 0, 0, 0, 0, 0, 0, 0, b'(', 0, 0, 0, 0, 0, 0, 0, // "("
    4, 0, 0, 0, 0, 0, 0, 0, b'n', b'a', b'm', b'e', 0, 0, 0, 0, // "name"
    5, 0, 0, 0, 0, 0, 0, 0, b'.', b'k', b'e', b'e', b'p', 0, 0, 0, // ".keep"
    4, 0, 0, 0, 0, 0, 0, 0, b'n', b'o', b'd', b'e', 0, 0, 0, 0, // "node"
    1, 0, 0, 0, 0, 0, 0, 0, b'(', 0, 0, 0, 0, 0, 0, 0, // "("
    4, 0, 0, 0, 0, 0, 0, 0, b't', b'y', b'p', b'e', 0, 0, 0, 0, // "type"
    7, 0, 0, 0, 0, 0, 0, 0, b'r', b'e', b'g', b'u', b'l', b'a', b'r', 0, // "regular"
    8, 0, 0, 0, 0, 0, 0, 0, b'c', b'o', b'n', b't', b'e', b'n', b't', b's', // "contents"
    0, 0, 0, 0, 0, 0, 0, 0, // ""
    1, 0, 0, 0, 0, 0, 0, 0, b')', 0, 0, 0, 0, 0, 0, 0, // ")"
    1, 0, 0, 0, 0, 0, 0, 0, b')', 0, 0, 0, 0, 0, 0, 0, // ")"
    5, 0, 0, 0, 0, 0, 0, 0, b'e', b'n', b't', b'r', b'y', 0, 0, 0, // "entry"
    1, 0, 0, 0, 0, 0, 0, 0, b'(', 0, 0, 0, 0, 0, 0, 0, // "("
    4, 0, 0, 0, 0, 0, 0, 0, b'n', b'a', b'm', b'e', 0, 0, 0, 0, // "name"
    2, 0, 0, 0, 0, 0, 0, 0, b'a', b'a', 0, 0, 0, 0, 0, 0, // "aa"
    4, 0, 0, 0, 0, 0, 0, 0, b'n', b'o', b'd', b'e', 0, 0, 0, 0, // "node"
    1, 0, 0, 0, 0, 0, 0, 0, b'(', 0, 0, 0, 0, 0, 0, 0, // "("
    4, 0, 0, 0, 0, 0, 0, 0, b't', b'y', b'p', b'e', 0, 0, 0, 0, // "type"
    7, 0, 0, 0, 0, 0, 0, 0, b's', b'y', b'm', b'l', b'i', b'n', b'k', 0, // "symlink"
    6, 0, 0, 0, 0, 0, 0, 0, b't', b'a', b'r', b'g', b'e', b't', 0, 0, // target
    24, 0, 0, 0, 0, 0, 0, 0, b'/', b'n', b'i', b'x', b'/', b's', b't', b'o', b'r', b'e', b'/',
    b's', b'o', b'm', b'e', b'w', b'h', b'e', b'r', b'e', b'e', b'l', b's',
    b'e', // "/nix/store/somewhereelse"
    1, 0, 0, 0, 0, 0, 0, 0, b')', 0, 0, 0, 0, 0, 0, 0, // ")"
    1, 0, 0, 0, 0, 0, 0, 0, b')', 0, 0, 0, 0, 0, 0, 0, // ")"
    5, 0, 0, 0, 0, 0, 0, 0, b'e', b'n', b't', b'r', b'y', 0, 0, 0, // "entry"
    1, 0, 0, 0, 0, 0, 0, 0, b'(', 0, 0, 0, 0, 0, 0, 0, // "("
    4, 0, 0, 0, 0, 0, 0, 0, b'n', b'a', b'm', b'e', 0, 0, 0, 0, // "name"
    4, 0, 0, 0, 0, 0, 0, 0, b'k', b'e', b'e', b'p', 0, 0, 0, 0, // "keep"
    4, 0, 0, 0, 0, 0, 0, 0, b'n', b'o', b'd', b'e', 0, 0, 0, 0, // "node"
    1, 0, 0, 0, 0, 0, 0, 0, b'(', 0, 0, 0, 0, 0, 0, 0, // "("
    4, 0, 0, 0, 0, 0, 0, 0, b't', b'y', b'p', b'e', 0, 0, 0, 0, // "type"
    9, 0, 0, 0, 0, 0, 0, 0, b'd', b'i', b'r', b'e', b'c', b't', b'o', b'r', b'y', 0, 0, 0, 0, 0, 0,
    0, // "directory"
    5, 0, 0, 0, 0, 0, 0, 0, b'e', b'n', b't', b'r', b'y', 0, 0, 0, // "entry"
    1, 0, 0, 0, 0, 0, 0, 0, b'(', 0, 0, 0, 0, 0, 0, 0, // "("
    4, 0, 0, 0, 0, 0, 0, 0, b'n', b'a', b'm', b'e', 0, 0, 0, 0, // "name"
    5, 0, 0, 0, 0, 0, 0, 0, 46, 107, 101, 101, 112, 0, 0, 0, // ".keep"
    4, 0, 0, 0, 0, 0, 0, 0, 110, 111, 100, 101, 0, 0, 0, 0, // "node"
    1, 0, 0, 0, 0, 0, 0, 0, b'(', 0, 0, 0, 0, 0, 0, 0, // "("
    4, 0, 0, 0, 0, 0, 0, 0, b't', b'y', b'p', b'e', 0, 0, 0, 0, // "type"
    7, 0, 0, 0, 0, 0, 0, 0, b'r', b'e', b'g', b'u', b'l', b'a', b'r', 0, // "regular"
    8, 0, 0, 0, 0, 0, 0, 0, b'c', b'o', b'n', b't', b'e', b'n', b't', b's', // "contents"
    0, 0, 0, 0, 0, 0, 0, 0, // ""
    1, 0, 0, 0, 0, 0, 0, 0, b')', 0, 0, 0, 0, 0, 0, 0, // ")"
    1, 0, 0, 0, 0, 0, 0, 0, b')', 0, 0, 0, 0, 0, 0, 0, // ")"
    1, 0, 0, 0, 0, 0, 0, 0, b')', 0, 0, 0, 0, 0, 0, 0, // ")"
    1, 0, 0, 0, 0, 0, 0, 0, b')', 0, 0, 0, 0, 0, 0, 0, // ")"
    1, 0, 0, 0, 0, 0, 0, 0, b')', 0, 0, 0, 0, 0, 0, 0, // ")"
];

/// A PathInfo message
pub static PATH_INFO: LazyLock<PathInfo> = LazyLock::new(|| PathInfo {
    store_path: DUMMY_PATH.clone(),
    node: tvix_castore::Node::Directory {
        digest: DUMMY_DIGEST.clone(),
        size: 0,
    },
    references: vec![DUMMY_PATH.clone()],
    nar_sha256: [0; 32],
    nar_size: 0,
    signatures: vec![],
    deriver: None,
    ca: Some(CAHash::Nar(NixHash::Sha256([0; 32]))),
});

/// A PathInfo message for the store path with CASTORE_NODE_SYMLINK as root node.
pub static PATH_INFO_SYMLINK: LazyLock<PathInfo> = LazyLock::new(|| PathInfo {
    store_path: DUMMY_PATH.clone(),
    node: CASTORE_NODE_SYMLINK.clone(),
    references: vec![],
    nar_size: NAR_CONTENTS_SYMLINK.len() as u64,
    nar_sha256: sha2::Sha256::new_with_prefix(NAR_CONTENTS_SYMLINK.as_slice())
        .finalize()
        .into(),
    signatures: vec![],
    deriver: None,
    ca: None,
});
