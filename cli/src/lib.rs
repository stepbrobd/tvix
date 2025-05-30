use std::path::PathBuf;
use std::rc::Rc;
use std::str::FromStr;

use rustc_hash::FxHashMap;
use smol_str::SmolStr;
use std::fmt::Write;
use tracing::instrument;
use tvix_eval::{
    builtins::impure_builtins,
    observer::{DisassemblingObserver, TracingObserver},
    ErrorKind, EvalIO, EvalMode, GlobalsMap, SourceCode, Value,
};
use tvix_glue::{
    builtins::{add_derivation_builtins, add_import_builtins},
    configure_nix_path,
    tvix_io::TvixIO,
    tvix_store_io::TvixStoreIO,
};

pub mod args;
pub mod assignment;
pub mod repl;

pub use args::Args;
pub use repl::Repl;

pub fn init_io_handle(args: &Args) -> Rc<TvixStoreIO> {
    // TODO(tazjin): ugly for now, but this is temporary while we drop the old
    // store, this whole function will go away probably.
    let mut simstore = tvix_simstore::SimulatedStoreIO::default();
    if let (Some(nix_path), Some(store_dir)) = (args.nix_path(), simstore.store_dir()) {
        let search_path =
            tvix_eval::NixSearchPath::from_str(&nix_path).expect("NIX_PATH was invalid");
        for entry in search_path.get_entries() {
            let path = entry.get_path();
            if !path.starts_with(&store_dir) {
                continue;
            }

            simstore
                .add_passthru(&path.to_string_lossy(), path.to_path_buf())
                .expect("setting passthru failed");
        }
    }

    Rc::new(TvixStoreIO::new(simstore))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AllowIncomplete {
    Allow,
    #[default]
    RequireComplete,
}

impl AllowIncomplete {
    fn allow(&self) -> bool {
        matches!(self, Self::Allow)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IncompleteInput;

pub struct EvalResult {
    value: Option<Value>,
    globals: Rc<GlobalsMap>,
}

/// Interprets the given code snippet, printing out warnings and errors and returning the result
#[allow(clippy::too_many_arguments)]
pub fn evaluate(
    tvix_store_io: Rc<TvixStoreIO>,
    code: &str,
    path: Option<PathBuf>,
    args: &Args,
    allow_incomplete: AllowIncomplete,
    env: Option<&FxHashMap<SmolStr, Value>>,
    globals: Option<Rc<GlobalsMap>>,
    source_map: Option<SourceCode>,
) -> Result<EvalResult, IncompleteInput> {
    let mut eval_builder = tvix_eval::Evaluation::builder(Rc::new(TvixIO::new(
        tvix_store_io.clone() as Rc<dyn EvalIO>,
    )) as Rc<dyn EvalIO>)
    .enable_import()
    .env(env);

    if args.strict {
        eval_builder = eval_builder.mode(EvalMode::Strict);
    }

    match globals {
        Some(globals) => {
            eval_builder = eval_builder.with_globals(globals);
        }
        None => {
            eval_builder = eval_builder.add_builtins(impure_builtins());
            eval_builder = add_derivation_builtins(eval_builder, Rc::clone(&tvix_store_io));
            // eval_builder = add_fetcher_builtins(eval_builder, Rc::clone(&tvix_store_io));
            eval_builder = add_import_builtins(eval_builder, Rc::clone(&tvix_store_io));
        }
    };
    eval_builder = configure_nix_path(eval_builder, &args.nix_path());

    if let Some(source_map) = source_map {
        eval_builder = eval_builder.with_source_map(source_map);
    }

    let source_map = eval_builder.source_map().clone();
    let (result, globals) = {
        let mut compiler_observer =
            DisassemblingObserver::new(source_map.clone(), std::io::stderr());
        if args.dump_bytecode {
            eval_builder.set_compiler_observer(Some(&mut compiler_observer));
        }

        let mut runtime_observer = TracingObserver::new(std::io::stderr());
        if args.trace_runtime {
            if args.trace_runtime_timing {
                runtime_observer.enable_timing()
            }
            eval_builder.set_runtime_observer(Some(&mut runtime_observer));
        }

        let eval = eval_builder.build();
        let globals = eval.globals();
        let result = eval.evaluate(code, path);
        (result, globals)
    };

    if allow_incomplete.allow()
        && result.errors.iter().any(|err| {
            matches!(
                &err.kind,
                ErrorKind::ParseErrors(pes)
                    if pes.iter().any(|pe| matches!(pe, rnix::parser::ParseError::UnexpectedEOF))
            )
        })
    {
        return Err(IncompleteInput);
    }

    if args.display_ast {
        if let Some(ref expr) = result.expr {
            eprintln!("AST: {}", tvix_eval::pretty_print_expr(expr));
        }
    }

    for error in &result.errors {
        error.fancy_format_stderr();
    }

    if !args.no_warnings {
        for warning in &result.warnings {
            warning.fancy_format_stderr(&source_map);
        }
    }

    if let Some(dumpdir) = &args.drv_dumpdir {
        // Dump all known derivations files to `dumpdir`.
        std::fs::create_dir_all(dumpdir).expect("failed to create drv dumpdir");
        tvix_store_io
            .known_paths
            .borrow()
            .get_derivations()
            // Skip already dumped derivations.
            .filter(|(drv_path, _)| !dumpdir.join(drv_path.to_string()).exists())
            .for_each(|(drv_path, drv)| {
                std::fs::write(dumpdir.join(drv_path.to_string()), drv.to_aterm_bytes())
                    .expect("failed to write drv to dumpdir");
            })
    }

    Ok(EvalResult {
        globals,
        value: result.value,
    })
}

pub struct InterpretResult {
    output: String,
    success: bool,
    pub(crate) globals: Option<Rc<GlobalsMap>>,
}

impl InterpretResult {
    pub fn empty_success(globals: Option<Rc<GlobalsMap>>) -> Self {
        Self {
            output: String::new(),
            success: true,
            globals,
        }
    }

    pub fn finalize(self) -> bool {
        print!("{}", self.output);
        self.success
    }

    pub fn output(&self) -> &str {
        &self.output
    }

    pub fn success(&self) -> bool {
        self.success
    }
}

/// Interprets the given code snippet, printing out warnings, errors
/// and the result itself. The return value indicates whether
/// evaluation succeeded.
#[instrument(skip_all, fields(indicatif.pb_show=tracing::field::Empty))]
#[allow(clippy::too_many_arguments)]
pub fn interpret(
    tvix_store_io: Rc<TvixStoreIO>,
    code: &str,
    path: Option<PathBuf>,
    args: &Args,
    explain: bool,
    allow_incomplete: AllowIncomplete,
    env: Option<&FxHashMap<SmolStr, Value>>,
    globals: Option<Rc<GlobalsMap>>,
    source_map: Option<SourceCode>,
) -> Result<InterpretResult, IncompleteInput> {
    let mut output = String::new();
    let result = evaluate(
        tvix_store_io,
        code,
        path,
        args,
        allow_incomplete,
        env,
        globals,
        source_map,
    )?;

    if let Some(value) = result.value.as_ref() {
        if explain {
            writeln!(&mut output, "=> {}", value.explain()).unwrap();
        } else if args.raw {
            writeln!(&mut output, "{}", value.to_contextful_str().unwrap()).unwrap();
        } else {
            writeln!(&mut output, "=> {} :: {}", value, value.type_of()).unwrap();
        }
    }

    // inform the caller about any errors
    Ok(InterpretResult {
        output,
        success: result.value.is_some(),
        globals: Some(result.globals),
    })
}
