//! `tvix-simstore` implements a simulated Nix store implementation that can be “interacted” with
//! from `tvix-eval`. This allows evaluating Nix expressions that use store dependent features
//! like path interpolation, `builtins.derivation` etc. without even having the ability to write
//! to a store let alone build a derivation. This is similar to the `dummy://` store implemented
//! by C++ Nix (>= 2.4).
//!
//! Nix expressions that do need a functioning store, e.g. for import from derivation (IFD),
//! will not work. To ensure purity, all reads from the store directory will result in
//! [`SimulatedStoreError::StorePathRead`], i.e. `tvix-simstore` won't access store paths
//! (i.e. paths below the configured `store_dir`) since they'd exist only by chance.
//!
//! Since no uniform store interface has been defined by `tvix-eval` yet, `tvix-simstore` consists
//! of the following components:
//!
//! - [`SimulatedStoreIO`] implements the `EvalIO` trait and handles calculation of the store
//!   paths for files that would need to be imported into the store.
//! - The necessary additional builtins haven't been implemented yet.
use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::io::{BufReader, Error, Read, Result};
use std::os::unix::ffi::OsStringExt;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use nix_compat::{
    nar,
    nixhash::{CAHash, NixHash},
    store_path::{build_ca_path, StorePath},
};
use sha2::{Digest, Sha256};
use tvix_eval::{EvalIO, FileType, StdIO};

pub struct SimulatedStoreIO {
    store_dir: String,
    passthru_paths: RefCell<HashMap<[u8; 20], PathBuf>>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum SimulatedStoreError {
    StorePathRead,
    NixCompatError(nix_compat::store_path::Error),
}

impl fmt::Display for SimulatedStoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SimulatedStoreError::StorePathRead => {
                write!(f, "simstore would need to read from a realised store path")
            }

            SimulatedStoreError::NixCompatError(cause) => {
                write!(f, "invalid Nix store path: ")?;
                cause.fmt(f)
            }
        }
    }
}

impl std::error::Error for SimulatedStoreError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        None
    }
}

impl From<nix_compat::store_path::Error> for SimulatedStoreError {
    fn from(cause: nix_compat::store_path::Error) -> Self {
        Self::NixCompatError(cause)
    }
}

impl Default for SimulatedStoreIO {
    fn default() -> Self {
        Self {
            store_dir: "/nix/store".to_owned(),
            passthru_paths: Default::default(),
        }
    }
}

// TODO(sterni): creation with configurable store dir
impl SimulatedStoreIO {
    /// Returns a path from which StdIO can read, unless realisation is required
    /// (which the simulated store does not support).
    fn to_readable_path<'a>(&self, path: &'a Path) -> Result<Cow<'a, Path>> {
        if !path.starts_with(Path::new(&self.store_dir)) {
            return Ok(Cow::Borrowed(path));
        }

        let (store_path, relative) =
            StorePath::<&str>::from_absolute_path_full(path).map_err(Error::other)?;

        if let Some(base) = self.passthru_paths.borrow().get(store_path.digest()) {
            return Ok(Cow::Owned(if relative.as_os_str().is_empty() {
                base.into()
            } else {
                base.join(relative)
            }));
        }

        Err(Error::other(SimulatedStoreError::StorePathRead))
    }
}

impl EvalIO for SimulatedStoreIO {
    fn store_dir(&self) -> Option<String> {
        Some(self.store_dir.clone())
    }

    fn import_path(&self, path: &Path) -> Result<PathBuf> {
        let path = path.canonicalize()?;
        let mut hash = Sha256::new();
        let nar = nar::writer::open(&mut hash)?;

        fn walk_path<T>(nar: nar::writer::Node<'_, T>, path: &Path) -> Result<()>
        where
            T: std::io::Write,
        {
            let meta = fs::symlink_metadata(path)?;

            if meta.is_symlink() {
                let target = fs::read_link(path)?.into_os_string().into_vec();
                nar.symlink(target.as_slice())?;
            } else if meta.is_file() {
                let executable = (meta.mode() & 0o100) != 0;

                let file = fs::File::open(path)?;
                let mut reader = BufReader::new(file);

                nar.file(executable, meta.size(), &mut reader)?;
            } else if meta.is_dir() {
                let mut dir = nar.directory()?;
                let mut entries = fs::read_dir(path)?.collect::<Result<Vec<_>>>()?;
                // TODO(sterni): confirm this is the precise sort ordering we need
                entries.sort_by_key(|e| e.file_name());
                for fs_entry in entries {
                    let node = dir.entry(fs_entry.file_name().into_vec().as_slice())?;
                    walk_path(node, fs_entry.path().as_path())?;
                }
                dir.close()?;
            }

            Ok(())
        }

        walk_path(nar, &path)?;

        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or(Error::other("Could not determine Basename for path"))?;
        let hash = CAHash::Nar(NixHash::Sha256(hash.finalize().into()));
        // TODO(sterni): Vec::new is ugly, copied from //tvix/glue/src/builtins/import.rs
        let store_path: StorePath<&str> = build_ca_path(name, &hash, Vec::<&str>::new(), false)
            .map_err(|_| Error::other("Failed to construct store path"))?;

        self.passthru_paths
            .borrow_mut()
            .insert(*store_path.digest(), path.to_owned());

        Ok(PathBuf::from(store_path.to_absolute_path()))
    }

    // TODO(sterni): proc macro for dispatching methods
    fn path_exists(&self, path: &Path) -> Result<bool> {
        StdIO.path_exists(self.to_readable_path(path)?.as_ref())
    }

    fn open(&self, path: &Path) -> Result<Box<dyn Read>> {
        StdIO.open(self.to_readable_path(path)?.as_ref())
    }

    fn file_type(&self, path: &Path) -> Result<FileType> {
        StdIO.file_type(self.to_readable_path(path)?.as_ref())
    }

    fn read_dir(&self, path: &Path) -> Result<Vec<(bytes::Bytes, FileType)>> {
        StdIO.read_dir(self.to_readable_path(path)?.as_ref())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn io_err_to_simstore_err<T>(res: Result<T>) -> SimulatedStoreError {
        res.err()
            .expect("Store Path Read should fail")
            .downcast::<SimulatedStoreError>()
            .expect("Should be SimulatedStoreError")
    }

    #[test]
    fn store_read_disallowed() {
        let paths = vec![
            "0a00kbgj7n5s2ds6r2ffsmbz8rkg3hdh-charset-0.3.10-r1.cabal.drv",
            "bz02y8zr6wp4yli9bqadjmf0biqinw6i-website/index.html",
            "n2v8qwc85kl4nk6ipfpaxs2pkjzka4v8-serve-examples",
        ];
        let store_io = SimulatedStoreIO::default();

        for path in paths {
            let mut abs = PathBuf::from(
                store_io
                    .store_dir()
                    .expect("SimulatedStoreIO should have a store_dir"),
            );
            abs.push(path);

            assert!(store_io.to_readable_path(&abs).is_err());

            assert_eq!(
                io_err_to_simstore_err(store_io.path_exists(&abs)),
                SimulatedStoreError::StorePathRead
            );
            assert_eq!(
                io_err_to_simstore_err(store_io.file_type(&abs)),
                SimulatedStoreError::StorePathRead
            );
            assert_eq!(
                io_err_to_simstore_err(store_io.open(&abs)),
                SimulatedStoreError::StorePathRead
            );
            assert_eq!(
                io_err_to_simstore_err(store_io.read_dir(&abs)),
                SimulatedStoreError::StorePathRead
            );
        }
    }

    #[test]
    fn imported_paths() {
        let store_io = SimulatedStoreIO::default();
        assert_eq!(
            store_io
                .import_path(Path::new("./test-data/q.txt"))
                .expect("importing test data should succeed"),
            Path::new("/nix/store/6w97x3p5yw17nwvqn3s6mrhdlznmzmiv-q.txt")
        );
        assert_eq!(
            store_io
                .import_path(Path::new("./test-data"))
                .expect("importing test data should succeed"),
            Path::new("/nix/store/ljqm0pf4b43bk53lymzrbljvdxi5vkcm-test-data")
        );
    }

    #[test]
    fn passthru_paths_file() {
        let store_io = SimulatedStoreIO::default();
        let imported = store_io
            .import_path(Path::new("./test-data/q.txt"))
            .expect("importing test data should succeed");
        assert!(store_io
            .path_exists(&imported)
            .expect("imported path should be forwarded"));
    }

    #[test]
    fn passthru_paths_folder() {
        let store_io = SimulatedStoreIO::default();
        let imported = store_io
            .import_path(Path::new("./test-data"))
            .expect("importing test data should succeed");
        assert!(store_io
            .path_exists(&imported.join("q.txt"))
            .expect("imported path should be forwarded"));
    }
}
