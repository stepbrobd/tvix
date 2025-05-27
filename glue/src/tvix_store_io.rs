//! This module provides an implementation of EvalIO talking to tvix-store.
use std::{
    cell::RefCell,
    io,
    path::{Path, PathBuf},
    sync::Arc,
};
use tracing::{instrument, Level};
use tvix_build::buildservice::BuildService;
use tvix_eval::{EvalIO, FileType};
use tvix_simstore::SimulatedStoreIO;
use tvix_store::nar::NarCalculationService;

use tvix_castore::{blobservice::BlobService, directoryservice::DirectoryService};
use tvix_store::pathinfoservice::PathInfoService;

// use crate::fetchers::Fetcher;
use crate::known_paths::KnownPaths;

/// Implements [EvalIO], asking given [PathInfoService], [DirectoryService]
/// and [BlobService].
///
/// In case the given path does not exist in these stores, we ask StdIO.
/// This is to both cover cases of syntactically valid store paths, that exist
/// on the filesystem (still managed by Nix), as well as being able to read
/// files outside store paths.
///
/// This structure is also directly used by the derivation builtins
/// and tightly coupled to it.
///
/// In the future, we may revisit that coupling and figure out how to generalize this interface and
/// hide this implementation detail of the glue itself so that glue can be used with more than one
/// implementation of "Tvix Store IO" which does not necessarily bring the concept of blob service,
/// directory service or path info service.
pub struct TvixStoreIO {
    // Field for in-progress switch to simulated store:
    pub(crate) simulated_store: SimulatedStoreIO,

    #[allow(dead_code)]
    build_service: Arc<dyn BuildService>,
    #[allow(dead_code)]
    pub(crate) tokio_handle: tokio::runtime::Handle,

    // #[allow(clippy::type_complexity)]
    // pub(crate) fetcher: Fetcher<
    //     Arc<dyn BlobService>,
    //     Arc<dyn DirectoryService>,
    //     Arc<dyn PathInfoService>,
    //     Arc<dyn NarCalculationService>,
    // >,

    // Paths known how to produce, by building or fetching.
    pub known_paths: RefCell<KnownPaths>,
}

impl TvixStoreIO {
    pub fn new(
        simulated_store: SimulatedStoreIO,
        _blob_service: Arc<dyn BlobService>,
        _directory_service: Arc<dyn DirectoryService>,
        _path_info_service: Arc<dyn PathInfoService>,
        _nar_calculation_service: Arc<dyn NarCalculationService>,
        build_service: Arc<dyn BuildService>,
        tokio_handle: tokio::runtime::Handle,
    ) -> Self {
        Self {
            simulated_store,
            build_service,
            tokio_handle,
            // fetcher: Fetcher::new(
            //     blob_service,
            //     directory_service,
            //     path_info_service,
            //     nar_calculation_service,
            // ),
            known_paths: Default::default(),
        }
    }
}

impl EvalIO for TvixStoreIO {
    #[instrument(skip(self), ret(level = Level::TRACE), err)]
    fn path_exists(&self, path: &Path) -> io::Result<bool> {
        self.simulated_store.path_exists(path)
    }

    #[instrument(skip(self), err)]
    fn open(&self, path: &Path) -> io::Result<Box<dyn io::Read>> {
        self.simulated_store.open(path)
    }

    #[instrument(skip(self), ret(level = Level::TRACE), err)]
    fn file_type(&self, path: &Path) -> io::Result<FileType> {
        self.simulated_store.file_type(path)
    }

    #[instrument(skip(self), ret(level = Level::TRACE), err)]
    fn read_dir(&self, path: &Path) -> io::Result<Vec<(bytes::Bytes, FileType)>> {
        self.simulated_store.read_dir(path)
    }

    fn import_path(&self, path: &Path) -> io::Result<PathBuf> {
        self.simulated_store.import_path(path)
    }

    fn store_dir(&self) -> Option<String> {
        self.simulated_store.store_dir()
    }
}

#[cfg(test)]
mod tests {
    use std::{path::Path, rc::Rc, sync::Arc};

    use bstr::ByteSlice;
    use clap::Parser;
    use tempfile::TempDir;
    use tvix_build::buildservice::DummyBuildService;
    use tvix_eval::{EvalIO, EvaluationResult};
    use tvix_store::utils::{construct_services, ServiceUrlsMemory};

    use super::TvixStoreIO;
    use crate::builtins::{add_derivation_builtins, add_import_builtins};

    /// evaluates a given nix expression and returns the result.
    /// Takes care of setting up the evaluator so it knows about the
    /// `derivation` builtin.
    fn eval(str: &str) -> EvaluationResult {
        let tokio_runtime = tokio::runtime::Runtime::new().unwrap();
        let (blob_service, directory_service, path_info_service, nar_calculation_service) =
            tokio_runtime
                .block_on(async {
                    construct_services(ServiceUrlsMemory::parse_from(std::iter::empty::<&str>()))
                        .await
                })
                .unwrap();

        let io = Rc::new(TvixStoreIO::new(
            Default::default(),
            blob_service,
            directory_service,
            path_info_service,
            nar_calculation_service.into(),
            Arc::<DummyBuildService>::default(),
            tokio_runtime.handle().clone(),
        ));

        let mut eval_builder =
            tvix_eval::Evaluation::builder(io.clone() as Rc<dyn EvalIO>).enable_import();
        eval_builder = add_derivation_builtins(eval_builder, Rc::clone(&io));
        // eval_builder = add_fetcher_builtins(eval_builder, Rc::clone(&io));
        eval_builder = add_import_builtins(eval_builder, io);
        let eval = eval_builder.build();

        // run the evaluation itself.
        eval.evaluate(str, None)
    }

    /// Helper function that takes a &Path, and invokes a tvix evaluator coercing that path to a string
    /// (via "${/this/path}"). The path can be both absolute or not.
    /// It returns Option<String>, depending on whether the evaluation succeeded or not.
    fn import_path_and_compare<P: AsRef<Path>>(p: P) -> Option<String> {
        // Try to import the path using "${/tmp/path/to/test}".
        // The format string looks funny, the {} passed to Nix needs to be
        // escaped.
        let code = format!(r#""${{{}}}""#, p.as_ref().display());
        let result = eval(&code);

        if !result.errors.is_empty() {
            return None;
        }

        let value = result.value.expect("must be some");
        match value {
            tvix_eval::Value::String(s) => Some(s.to_str_lossy().into_owned()),
            _ => panic!("unexpected value type: {:?}", value),
        }
    }

    /// Import a directory with a zero-sized ".keep" regular file.
    /// Ensure it matches the (pre-recorded) store path that Nix would produce.
    #[test]
    fn import_directory() {
        let tmpdir = TempDir::new().unwrap();

        // create a directory named "test"
        let src_path = tmpdir.path().join("test");
        std::fs::create_dir(&src_path).unwrap();

        // write a regular file `.keep`.
        std::fs::write(src_path.join(".keep"), vec![]).unwrap();

        // importing the path with .../test at the end.
        assert_eq!(
            Some("/nix/store/gq3xcv4xrj4yr64dflyr38acbibv3rm9-test".to_string()),
            import_path_and_compare(&src_path)
        );

        // importing the path with .../test/. at the end.
        assert_eq!(
            Some("/nix/store/gq3xcv4xrj4yr64dflyr38acbibv3rm9-test".to_string()),
            import_path_and_compare(src_path.join("."))
        );
    }

    /// Import a file into the store. Nix uses the "recursive"/NAR-based hashing
    /// scheme for these.
    #[test]
    fn import_file() {
        let tmpdir = TempDir::new().unwrap();

        // write a regular file `empty`.
        std::fs::write(tmpdir.path().join("empty"), vec![]).unwrap();

        assert_eq!(
            Some("/nix/store/lx5i78a4izwk2qj1nq8rdc07y8zrwy90-empty".to_string()),
            import_path_and_compare(tmpdir.path().join("empty"))
        );

        // write a regular file `hello.txt`.
        std::fs::write(tmpdir.path().join("hello.txt"), b"Hello World!").unwrap();

        assert_eq!(
            Some("/nix/store/925f1jb1ajrypjbyq7rylwryqwizvhp0-hello.txt".to_string()),
            import_path_and_compare(tmpdir.path().join("hello.txt"))
        );
    }

    /// Invoke toString on a nonexisting file, and access the .file attribute.
    /// This should not cause an error, because it shouldn't trigger an import,
    /// and leave the path as-is.
    #[test]
    fn nonexisting_path_without_import() {
        let result = eval("toString ({ line = 42; col = 42; file = /deep/thought; }.file)");

        assert!(result.errors.is_empty(), "expect evaluation to succeed");
        let value = result.value.expect("must be some");

        match value {
            tvix_eval::Value::String(s) => {
                assert_eq!(*s, "/deep/thought");
            }
            _ => panic!("unexpected value type: {:?}", value),
        }
    }
}
