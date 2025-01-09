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
}

#[derive(Debug, PartialEq, Eq)]
pub enum SimulatedStoreError {
    StorePathRead,
}

impl fmt::Display for SimulatedStoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SimulatedStoreError::StorePathRead => {
                write!(f, "simstore would need to read from a realised store path")
            }
        }
    }
}

impl std::error::Error for SimulatedStoreError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        None
    }
}

impl Default for SimulatedStoreIO {
    fn default() -> Self {
        Self {
            store_dir: "/nix/store".to_owned(),
        }
    }
}

// TODO(sterni): creation with configurable store dir
impl SimulatedStoreIO {
    fn check_below_store_dir(&self, path: &Path) -> Result<()> {
        if !path.starts_with(Path::new(&self.store_dir)) {
            Ok(())
        } else {
            Err(Error::other(SimulatedStoreError::StorePathRead))
        }
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

        Ok(PathBuf::from(store_path.to_absolute_path()))
    }

    // TODO(sterni): proc macro for dispatching methods
    fn path_exists(&self, path: &Path) -> Result<bool> {
        self.check_below_store_dir(path)?;
        StdIO.path_exists(path)
    }

    fn open(&self, path: &Path) -> Result<Box<dyn Read>> {
        self.check_below_store_dir(path)?;
        StdIO.open(path)
    }

    fn file_type(&self, path: &Path) -> Result<FileType> {
        self.check_below_store_dir(path)?;
        StdIO.file_type(path)
    }

    fn read_dir(&self, path: &Path) -> Result<Vec<(bytes::Bytes, FileType)>> {
        self.check_below_store_dir(path)?;
        StdIO.read_dir(path)
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

            assert!(store_io.check_below_store_dir(&abs).is_err());

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
}
