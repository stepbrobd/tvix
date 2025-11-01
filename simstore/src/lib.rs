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
use std::iter::Peekable;
use std::os::unix::ffi::OsStringExt;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use nix_compat::{
    nar,
    nixhash::{CAHash, NixHash},
    store_path::{build_ca_path, StorePath},
};
use sha2::{Digest, Sha256};
use tvix_eval::{builtin_macros::builtins, ErrorKind, EvalIO, FileType, StdIO, Value};

pub struct SimulatedStoreIO {
    store_dir: String,
    passthru_paths: RefCell<HashMap<[u8; 20], PathBuf>>,
}

// TODO: copied from glue/import.rs; where should this live?
fn path_to_name(path: &Path) -> std::io::Result<&str> {
    path.file_name()
        .and_then(|file_name| file_name.to_str())
        .ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "path must not be .. and the basename valid unicode",
            )
        })
}

impl SimulatedStoreIO {
    /// Adds a passthru path, mapping the given path to the given location on the
    /// filesystem.
    ///
    /// Using this incorrectly can lead to incomprehensible breakage.
    pub fn add_passthru(&mut self, path: &str, loc: PathBuf) -> Result<()> {
        let (store_path, _) =
            StorePath::<&str>::from_absolute_path_full(path).map_err(Error::other)?;

        self.passthru_paths
            .borrow_mut()
            .insert(*store_path.digest(), loc);
        Ok(())
    }
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

        // Pass known imported paths through to their original locations.
        if let Some(base) = self.passthru_paths.borrow().get(store_path.digest()) {
            return Ok(Cow::Owned(if relative.as_os_str().is_empty() {
                base.into()
            } else {
                base.join(relative)
            }));
        }

        // Allow reads from the "real" Nix store locally.
        if StdIO.path_exists(path)? {
            return Ok(Cow::Borrowed(path));
        }

        Err(Error::other(SimulatedStoreError::StorePathRead))
    }

    pub fn import_path_by_entries<I, E>(
        &self,
        name: &str,
        entries: I,
        expected_sha256: Option<[u8; 32]>,
    ) -> Result<StorePath<String>>
    where
        Error: From<E>,
        I: Iterator<Item = std::result::Result<walkdir::DirEntry, E>>,
    {
        let mut hash = Sha256::new();
        let nar = nar::writer::open(&mut hash)?;

        pack_entries(nar, &mut entries.peekable())?;

        let nar_hash = NixHash::Sha256(hash.finalize().into());

        if let Some(expected) = expected_sha256 {
            if nar_hash != NixHash::Sha256(expected) {
                // TODO: this error is really bad; needs both hashes etc.
                // It doesn't feel like this is the right place.
                return Err(Error::other("expected hash does not match"));
            }
        }

        let hash = CAHash::Nar(nar_hash);
        build_ca_path(name, &hash, Option::<String>::default(), false).map_err(Error::other)
    }
}

fn pack_entries_dir<W, E, I>(
    mut dir: nar::writer::Directory<'_, W>,
    depth: usize,
    walker: &mut Peekable<I>,
) -> Result<()>
where
    W: std::io::Write,
    Error: From<E>,
    I: Iterator<Item = std::result::Result<walkdir::DirEntry, E>>,
{
    loop {
        let peeked = match walker.peek() {
            None => break,
            Some(e) => e,
        };

        // `peeked` borrows the next result, if it is an error we need to
        // "actually" take it to be able to propagate the error.
        let entry = match peeked {
            Ok(e) => e,
            Err(_) => {
                walker.next().expect("is present")?;
                unreachable!("above `?` always exits");
            }
        };

        if entry.depth() < depth {
            break;
        }

        let nar = dir.entry(entry.file_name().to_owned().into_vec().as_slice())?;
        pack_entries(nar, walker)?;
    }

    dir.close()?;

    Ok(())
}

fn pack_entries<W, E, I>(nar: nar::writer::Node<'_, W>, walker: &mut Peekable<I>) -> Result<()>
where
    W: std::io::Write,
    Error: From<E>,
    I: Iterator<Item = std::result::Result<walkdir::DirEntry, E>>,
{
    let entry = if let Some(entry) = walker.next() {
        entry?
    } else {
        return Ok(());
    };

    let ft = entry.file_type();
    if ft.is_symlink() {
        let target = fs::read_link(entry.path())?.into_os_string();
        nar.symlink(target.into_vec().as_slice())?;
    } else if ft.is_file() {
        let meta = entry.metadata()?;
        let executable = (meta.mode() & 0o100) != 0;
        let file = fs::File::open(entry.path())?;
        let mut reader = BufReader::new(file);
        nar.file(executable, meta.size(), &mut reader)?;
    } else if ft.is_dir() {
        let inner_depth = entry.depth() + 1;
        let dir = nar.directory()?;
        pack_entries_dir(dir, inner_depth, walker)?;
    } else {
        return Err(Error::new(
            std::io::ErrorKind::InvalidData,
            "invalid file type for store ingestion",
        ));
    }

    Ok(())
}

impl EvalIO for SimulatedStoreIO {
    fn store_dir(&self) -> Option<String> {
        Some(self.store_dir.clone())
    }

    fn import_path(&self, path: &Path) -> Result<PathBuf> {
        let path = path.canonicalize()?;
        let mut hash = Sha256::new();
        let nar = nar::writer::open(&mut hash)?;

        let walker = walkdir::WalkDir::new(path.clone())
            .follow_links(false)
            .follow_root_links(false)
            .contents_first(false)
            .sort_by(|a, b| a.file_name().cmp(b.file_name()))
            .into_iter();

        pack_entries(nar, &mut walker.peekable())?;

        let name = path_to_name(&path)?;
        let hash = CAHash::Nar(NixHash::Sha256(hash.finalize().into()));
        let store_path: StorePath<&str> =
            build_ca_path(name, &hash, Option::<&str>::default(), false).map_err(Error::other)?;

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

// TODO(sterni): implement simulation, parse args
// TODO(sterni): move derivationStrict simulation here
#[builtins]
mod builtins {
    use super::*;
    use tvix_eval::generators::{Gen, GenCo};

    #[builtin("fetchGit")]
    async fn builtin_fetch_git(co: GenCo, args: Value) -> std::result::Result<Value, ErrorKind> {
        Err(ErrorKind::NotImplemented("fetchGit"))
    }

    #[builtin("fetchMercurial")]
    async fn builtin_fetch_mercurial(
        co: GenCo,
        args: Value,
    ) -> std::result::Result<Value, ErrorKind> {
        Err(ErrorKind::NotImplemented("fetchMercurial"))
    }

    #[builtin("fetchTarball")]
    async fn builtin_fetch_tarball(
        co: GenCo,
        args: Value,
    ) -> std::result::Result<Value, ErrorKind> {
        Err(ErrorKind::NotImplemented("fetchTarball"))
    }
}

pub fn simulated_store_builtins() -> Vec<(&'static str, Value)> {
    builtins::builtins()
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

    #[test]
    fn added_passthru_path() {
        let mut store_io = SimulatedStoreIO::default();
        let example = "/nix/store/a396z42saqql55cp5n1vrb2j0siq86k1-nixpkgs-src";
        let example_path = PathBuf::from(example);

        store_io
            .add_passthru(example, example_path.clone())
            .expect("adding passthru should work");

        store_io
            .path_exists(&example_path)
            .expect("path access should not fail");
    }
}
