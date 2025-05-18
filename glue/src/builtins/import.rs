//! Implements builtins used to import paths in the store.

use std::path::Path;
use std::rc::Rc;

use crate::tvix_store_io::TvixStoreIO;
use nix_compat::store_path::{build_ca_path, StorePath, StorePathRef};
use tvix_eval::{
    builtin_macros::builtins,
    generators::{self, GenCo},
    ErrorKind, EvalIO, Value,
};

/// Transform a path into its base name and returns an [`std::io::Error`] if it is `..` or if the
/// basename is not valid unicode.
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

async fn filtered_ingest(
    state: Rc<TvixStoreIO>,
    co: GenCo,
    path: &Path,
    name: Option<String>,
    filter: Option<&Value>,
) -> Result<StorePath<String>, ErrorKind> {
    let mut entries: Vec<walkdir::DirEntry> = vec![];
    let mut it = walkdir::WalkDir::new(path)
        .follow_links(false)
        .follow_root_links(false)
        .contents_first(false)
        .sort_by(|a, b| a.file_name().cmp(b.file_name()))
        .into_iter();

    // Always add root node.
    entries.push(
        it.next()
            .ok_or_else(|| ErrorKind::IO {
                path: Some(path.to_path_buf()),
                error: std::io::Error::new(std::io::ErrorKind::NotFound, "No root node emitted")
                    .into(),
            })?
            .map_err(|err| ErrorKind::IO {
                path: Some(path.to_path_buf()),
                error: std::io::Error::from(err).into(),
            })?,
    );

    while let Some(entry) = it.next() {
        // Entry could be a NotFound, if the root path specified does not exist.
        let entry = entry.map_err(|err| ErrorKind::IO {
            path: err.path().map(|p| p.to_path_buf()),
            error: std::io::Error::from(err).into(),
        })?;

        // As per Nix documentation `:doc builtins.filterSource`.
        let file_type = if entry.file_type().is_dir() {
            "directory"
        } else if entry.file_type().is_file() {
            "regular"
        } else if entry.file_type().is_symlink() {
            "symlink"
        } else {
            "unknown"
        };

        let should_keep: bool = if let Some(filter) = filter {
            generators::request_force(
                &co,
                generators::request_call_with(
                    &co,
                    filter.clone(),
                    [
                        Value::String(entry.path().as_os_str().as_encoded_bytes().into()),
                        Value::String(file_type.into()),
                    ],
                )
                .await,
            )
            .await
            .as_bool()?
        } else {
            true
        };

        if !should_keep {
            if file_type == "directory" {
                it.skip_current_dir();
            }
            continue;
        }

        entries.push(entry);
    }

    let dir_entries = entries
        .into_iter()
        .map(Ok::<walkdir::DirEntry, std::io::Error>);

    let name = match name {
        Some(name) => name,
        None => path_to_name(path)
            .expect("failed to derive the default name out of the path")
            .to_string(),
    };

    Ok(state
        .simulated_store
        .import_path_by_entries(&name, dir_entries, None)?)
}

#[builtins(state = "Rc<TvixStoreIO>")]
mod import_builtins {
    use super::*;

    use crate::builtins::ImportError;
    use crate::tvix_store_io::TvixStoreIO;
    use bstr::ByteSlice;
    use nix_compat::nixhash::{CAHash, NixHash};
    use sha2::Digest;
    use std::rc::Rc;
    use tvix_eval::builtins::coerce_value_to_path;
    use tvix_eval::generators::Gen;
    use tvix_eval::{generators::GenCo, ErrorKind, Value};
    use tvix_eval::{AddContext, FileType, NixContext, NixContextElement, NixString};

    // This is a helper used by both builtins.path and builtins.filterSource.
    async fn import_helper(
        state: Rc<TvixStoreIO>,
        co: GenCo,
        path: std::path::PathBuf,
        name: Option<&Value>,
        filter: Option<&Value>,
        recursive_ingestion: bool,
        expected_sha256: Option<[u8; 32]>,
    ) -> Result<Value, ErrorKind> {
        let name: String = match name {
            Some(name) => generators::request_force(&co, name.clone())
                .await
                .to_str()?
                .as_bstr()
                .to_string(),

            None => path_to_name(&path)
                .expect("Failed to derive the default name out of the path")
                .to_string(),
        };

        let store_path = match std::fs::metadata(&path)?.file_type().into() {
            // Regular file, non-recursive -> ingest with plain SHA256 content hash
            FileType::Regular if !recursive_ingestion => {
                let mut file = state.open(&path)?;
                let mut hasher = sha2::Sha256::new();
                let mut buffer = [0; 8192]; // 8KB buffer is a reasonable size \/(O.o)\/

                loop {
                    let bytes_read = file.read(&mut buffer)?;
                    if bytes_read == 0 {
                        break;
                    }
                    hasher.update(&buffer[..bytes_read]);
                }

                let actual_sha256 = hasher.finalize().into();

                // If an expected hash was provided upfront, compare and bail out.
                if let Some(expected_sha256) = expected_sha256 {
                    if actual_sha256 != expected_sha256 {
                        return Err(ImportError::HashMismatch(
                            path.clone(),
                            NixHash::Sha256(expected_sha256),
                            NixHash::Sha256(actual_sha256),
                        )
                        .into());
                    }
                }

                let ca = CAHash::Flat(NixHash::Sha256(actual_sha256));
                build_ca_path(&name, &ca, Vec::<&str>::new(), false)
                    .map_err(|e| tvix_eval::ErrorKind::TvixError(Rc::new(e)))?
            }

            FileType::Regular => {
                let dir_entry = walkdir::WalkDir::new(path)
                    .follow_root_links(false)
                    .into_iter();

                state
                    .simulated_store
                    .import_path_by_entries(&name, dir_entry, expected_sha256)?
            }

            FileType::Directory if !recursive_ingestion => {
                return Err(ImportError::FlatImportOfNonFile(path))?
            }

            // do the filtered ingest
            FileType::Directory => {
                filtered_ingest(state.clone(), co, path.as_ref(), Some(name), filter).await?
            }

            FileType::Symlink => {
                // FUTUREWORK: Nix follows a symlink if it's at the root,
                // except if it's not resolve-able (NixOS/nix#7761).i
                return Err(tvix_eval::ErrorKind::IO {
                    path: Some(path),
                    error: Rc::new(std::io::Error::new(
                        std::io::ErrorKind::Unsupported,
                        "builtins.path pointing to a symlink is ill-defined.",
                    )),
                });
            }
            FileType::Unknown => {
                return Err(tvix_eval::ErrorKind::IO {
                    path: Some(path),
                    error: Rc::new(std::io::Error::new(
                        std::io::ErrorKind::Unsupported,
                        "unsupported file type",
                    )),
                })
            }
        };

        let outpath = store_path.to_absolute_path();
        let ctx: NixContext = NixContextElement::Plain(outpath.to_string()).into();
        Ok(NixString::new_context_from(ctx, outpath.to_string()).into())
    }

    #[builtin("path")]
    async fn builtin_path(
        state: Rc<TvixStoreIO>,
        co: GenCo,
        args: Value,
    ) -> Result<Value, ErrorKind> {
        let args = args.to_attrs()?;

        let path = match coerce_value_to_path(
            &co,
            generators::request_force(&co, args.select_required("path")?.clone()).await,
        )
        .await?
        {
            Ok(path) => path,
            Err(cek) => return Ok(cek.into()),
        };

        let filter = args.select("filter");

        // Construct a sha256 hasher, which is needed for flat ingestion.
        let recursive_ingestion = args
            .select("recursive")
            .map(|r| r.as_bool())
            .transpose()?
            .unwrap_or(true); // Yes, yes, Nix, by default, sets `recursive = true;`.

        let expected_sha256 = args
            .select("sha256")
            .map(|h| {
                h.to_str().and_then(|expected| {
                    match nix_compat::nixhash::from_str(expected.to_str()?, Some("sha256")) {
                        Ok(NixHash::Sha256(digest)) => Ok(digest),
                        Ok(_) => unreachable!(),
                        Err(e) => Err(ErrorKind::InvalidHash(e.to_string())),
                    }
                })
            })
            .transpose()?;

        import_helper(
            state,
            co,
            path,
            args.select("name"),
            filter,
            recursive_ingestion,
            expected_sha256,
        )
        .await
    }

    #[builtin("filterSource")]
    async fn builtin_filter_source(
        state: Rc<TvixStoreIO>,
        co: GenCo,
        #[lazy] filter: Value,
        path: Value,
    ) -> Result<Value, ErrorKind> {
        let path =
            match coerce_value_to_path(&co, generators::request_force(&co, path).await).await? {
                Ok(path) => path,
                Err(cek) => return Ok(cek.into()),
            };

        import_helper(state, co, path, None, Some(&filter), true, None).await
    }

    #[builtin("storePath")]
    async fn builtin_store_path(
        state: Rc<TvixStoreIO>,
        co: GenCo,
        path: Value,
    ) -> Result<Value, ErrorKind> {
        let p = match &path {
            Value::String(s) => Path::new(s.as_bytes().to_os_str()?),
            Value::Path(p) => p.as_path(),
            _ => {
                return Err(ErrorKind::TypeError {
                    expected: "string or path",
                    actual: path.type_of(),
                })
            }
        };

        // For this builtin, the path needs to start with an absolute store path.
        let (store_path, _sub_path) = StorePathRef::from_absolute_path_full(p)
            .map_err(|_e| ImportError::PathNotAbsoluteOrInvalid(p.to_path_buf()))?;

        if state.path_exists(p)? {
            Ok(Value::String(NixString::new_context_from(
                [NixContextElement::Plain(store_path.to_absolute_path())].into(),
                p.as_os_str().as_encoded_bytes(),
            )))
        } else {
            Err(ErrorKind::IO {
                path: Some(p.to_path_buf()),
                error: Rc::new(std::io::ErrorKind::NotFound.into()),
            })
        }
    }

    #[builtin("toFile")]
    async fn builtin_to_file(co: GenCo, name: Value, content: Value) -> Result<Value, ErrorKind> {
        if name.is_catchable() {
            return Ok(name);
        }

        if content.is_catchable() {
            return Ok(content);
        }

        let name = name
            .to_str()
            .context("evaluating the `name` parameter of builtins.toFile")?;
        let content = content
            .to_contextful_str()
            .context("evaluating the `content` parameter of builtins.toFile")?;

        if content.iter_ctx_derivation().count() > 0
            || content.iter_ctx_single_outputs().count() > 0
        {
            return Err(ErrorKind::UnexpectedContext);
        }

        let name_str = std::str::from_utf8(name.as_bytes())?;
        let mut hasher = sha2::Sha256::new();
        hasher.update(&content);
        let ca_hash = CAHash::Text(hasher.finalize().into());
        let store_path: StorePath<&str> =
            build_ca_path(name_str, &ca_hash, content.iter_ctx_plain(), false)
                .map_err(|e| tvix_eval::ErrorKind::TvixError(Rc::new(e)))?;

        let abs_path = store_path.to_absolute_path();
        let context: NixContext = NixContextElement::Plain(abs_path.clone()).into();

        Ok(Value::from(NixString::new_context_from(context, abs_path)))
    }
}

pub use import_builtins::builtins as import_builtins;
