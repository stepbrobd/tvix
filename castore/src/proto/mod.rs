#![allow(clippy::derive_partial_eq_without_eq, non_snake_case)]
// https://github.com/hyperium/tonic/issues/1056
use data_encoding::BASE64;
use std::{collections::HashSet, iter::Peekable};
use thiserror::Error;

use prost::Message;

mod grpc_blobservice_wrapper;
mod grpc_directoryservice_wrapper;

pub use grpc_blobservice_wrapper::GRPCBlobServiceWrapper;
pub use grpc_directoryservice_wrapper::GRPCDirectoryServiceWrapper;

use crate::B3Digest;

tonic::include_proto!("tvix.castore.v1");

#[cfg(feature = "tonic-reflection")]
/// Compiled file descriptors for implementing [gRPC
/// reflection](https://github.com/grpc/grpc/blob/master/doc/server-reflection.md) with e.g.
/// [`tonic_reflection`](https://docs.rs/tonic-reflection).
pub const FILE_DESCRIPTOR_SET: &[u8] = tonic::include_file_descriptor_set!("tvix.castore.v1");

#[cfg(test)]
mod tests;

/// Errors that can occur during the validation of Directory messages.
#[derive(Debug, PartialEq, Eq, Error)]
pub enum ValidateDirectoryError {
    /// Elements are not in sorted order
    #[error("{} is not sorted", std::str::from_utf8(.0).unwrap_or(&BASE64.encode(.0)))]
    WrongSorting(Vec<u8>),
    /// Multiple elements with the same name encountered
    #[error("{0:?} is a duplicate name")]
    DuplicateName(Vec<u8>),
    /// Invalid name encountered
    #[error("Invalid name in {0:?}")]
    InvalidName(Vec<u8>),
    /// Invalid digest length encountered
    #[error("Invalid Digest length: {0}")]
    InvalidDigestLen(usize),
}

/// Checks a Node name for validity as an intermediate node, and returns an
/// error that's generated from the supplied constructor.
///
/// We disallow slashes, null bytes, '.', '..' and the empty string.
fn validate_node_name<E>(name: &[u8], err: fn(Vec<u8>) -> E) -> Result<(), E> {
    if name.is_empty()
        || name == b".."
        || name == b"."
        || name.contains(&0x00)
        || name.contains(&b'/')
    {
        return Err(err(name.to_vec()));
    }
    Ok(())
}

/// NamedNode is implemented for [FileNode], [DirectoryNode] and [SymlinkNode]
/// and [node::Node], so we can ask all of them for the name easily.
pub trait NamedNode {
    fn get_name(&self) -> &[u8];
}

impl NamedNode for &FileNode {
    fn get_name(&self) -> &[u8] {
        &self.name
    }
}

impl NamedNode for &DirectoryNode {
    fn get_name(&self) -> &[u8] {
        &self.name
    }
}

impl NamedNode for &SymlinkNode {
    fn get_name(&self) -> &[u8] {
        &self.name
    }
}

impl NamedNode for node::Node {
    fn get_name(&self) -> &[u8] {
        match self {
            node::Node::File(node_file) => &node_file.name,
            node::Node::Directory(node_directory) => &node_directory.name,
            node::Node::Symlink(node_symlink) => &node_symlink.name,
        }
    }
}

impl node::Node {
    /// Returns the node with a new name.
    pub fn rename(self, name: bytes::Bytes) -> Self {
        match self {
            node::Node::Directory(n) => node::Node::Directory(DirectoryNode { name, ..n }),
            node::Node::File(n) => node::Node::File(FileNode { name, ..n }),
            node::Node::Symlink(n) => node::Node::Symlink(SymlinkNode { name, ..n }),
        }
    }
}

/// Accepts a name, and a mutable reference to the previous name.
/// If the passed name is larger than the previous one, the reference is updated.
/// If it's not, an error is returned.
fn update_if_lt_prev<'n>(
    prev_name: &mut &'n [u8],
    name: &'n [u8],
) -> Result<(), ValidateDirectoryError> {
    if *name < **prev_name {
        return Err(ValidateDirectoryError::WrongSorting(name.to_vec()));
    }
    *prev_name = name;
    Ok(())
}

/// Inserts the given name into a HashSet if it's not already in there.
/// If it is, an error is returned.
fn insert_once<'n>(
    seen_names: &mut HashSet<&'n [u8]>,
    name: &'n [u8],
) -> Result<(), ValidateDirectoryError> {
    if seen_names.get(name).is_some() {
        return Err(ValidateDirectoryError::DuplicateName(name.to_vec()));
    }
    seen_names.insert(name);
    Ok(())
}

impl Directory {
    /// The size of a directory is the number of all regular and symlink elements,
    /// the number of directory elements, and their size fields.
    pub fn size(&self) -> u32 {
        self.files.len() as u32
            + self.symlinks.len() as u32
            + self
                .directories
                .iter()
                .fold(0, |acc: u32, e| (acc + 1 + e.size))
    }

    /// Calculates the digest of a Directory, which is the blake3 hash of a
    /// Directory protobuf message, serialized in protobuf canonical form.
    pub fn digest(&self) -> B3Digest {
        let mut hasher = blake3::Hasher::new();

        hasher
            .update(&self.encode_to_vec())
            .finalize()
            .as_bytes()
            .into()
    }

    /// validate checks the directory for invalid data, such as:
    /// - violations of name restrictions
    /// - invalid digest lengths
    /// - not properly sorted lists
    /// - duplicate names in the three lists
    pub fn validate(&self) -> Result<(), ValidateDirectoryError> {
        let mut seen_names: HashSet<&[u8]> = HashSet::new();

        let mut last_directory_name: &[u8] = b"";
        let mut last_file_name: &[u8] = b"";
        let mut last_symlink_name: &[u8] = b"";

        // check directories
        for directory_node in &self.directories {
            validate_node_name(&directory_node.name, ValidateDirectoryError::InvalidName)?;
            // ensure the digest has the appropriate size.
            if TryInto::<B3Digest>::try_into(directory_node.digest.clone()).is_err() {
                return Err(ValidateDirectoryError::InvalidDigestLen(
                    directory_node.digest.len(),
                ));
            }

            update_if_lt_prev(&mut last_directory_name, &directory_node.name)?;
            insert_once(&mut seen_names, &directory_node.name)?;
        }

        // check files
        for file_node in &self.files {
            validate_node_name(&file_node.name, ValidateDirectoryError::InvalidName)?;
            if TryInto::<B3Digest>::try_into(file_node.digest.clone()).is_err() {
                return Err(ValidateDirectoryError::InvalidDigestLen(
                    file_node.digest.len(),
                ));
            }

            update_if_lt_prev(&mut last_file_name, &file_node.name)?;
            insert_once(&mut seen_names, &file_node.name)?;
        }

        // check symlinks
        for symlink_node in &self.symlinks {
            validate_node_name(&symlink_node.name, ValidateDirectoryError::InvalidName)?;

            update_if_lt_prev(&mut last_symlink_name, &symlink_node.name)?;
            insert_once(&mut seen_names, &symlink_node.name)?;
        }

        Ok(())
    }

    /// Allows iterating over all three nodes ([DirectoryNode], [FileNode],
    /// [SymlinkNode]) in an ordered fashion, as long as the individual lists
    /// are sorted (which can be checked by the [Directory::validate]).
    pub fn nodes(&self) -> DirectoryNodesIterator {
        return DirectoryNodesIterator {
            i_directories: self.directories.iter().peekable(),
            i_files: self.files.iter().peekable(),
            i_symlinks: self.symlinks.iter().peekable(),
        };
    }
}

/// Struct to hold the state of an iterator over all nodes of a Directory.
///
/// Internally, this keeps peekable Iterators over all three lists of a
/// Directory message.
pub struct DirectoryNodesIterator<'a> {
    // directory: &Directory,
    i_directories: Peekable<std::slice::Iter<'a, DirectoryNode>>,
    i_files: Peekable<std::slice::Iter<'a, FileNode>>,
    i_symlinks: Peekable<std::slice::Iter<'a, SymlinkNode>>,
}

/// looks at two elements implementing NamedNode, and returns true if "left
/// is smaller / comes first".
///
/// Some(_) is preferred over None.
fn left_name_lt_right<A: NamedNode, B: NamedNode>(left: Option<&A>, right: Option<&B>) -> bool {
    match left {
        // if left is None, right always wins
        None => false,
        Some(left_inner) => {
            // left is Some.
            match right {
                // left is Some, right is None - left wins.
                None => true,
                Some(right_inner) => {
                    // both are Some - compare the name.
                    return left_inner.get_name() < right_inner.get_name();
                }
            }
        }
    }
}

impl Iterator for DirectoryNodesIterator<'_> {
    type Item = node::Node;

    // next returns the next node in the Directory.
    // we peek at all three internal iterators, and pick the one with the
    // smallest name, to ensure lexicographical ordering.
    // The individual lists are already known to be sorted.
    fn next(&mut self) -> Option<Self::Item> {
        if left_name_lt_right(self.i_directories.peek(), self.i_files.peek()) {
            // i_directories is still in the game, compare with symlinks
            if left_name_lt_right(self.i_directories.peek(), self.i_symlinks.peek()) {
                self.i_directories
                    .next()
                    .cloned()
                    .map(node::Node::Directory)
            } else {
                self.i_symlinks.next().cloned().map(node::Node::Symlink)
            }
        } else {
            // i_files is still in the game, compare with symlinks
            if left_name_lt_right(self.i_files.peek(), self.i_symlinks.peek()) {
                self.i_files.next().cloned().map(node::Node::File)
            } else {
                self.i_symlinks.next().cloned().map(node::Node::Symlink)
            }
        }
    }
}