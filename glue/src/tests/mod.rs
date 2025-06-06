use std::rc::Rc;

use pretty_assertions::assert_eq;
use std::path::PathBuf;
use tvix_eval::{EvalIO, EvalMode, Value};

use rstest::rstest;

use crate::{
    builtins::{add_derivation_builtins, add_import_builtins},
    configure_nix_path,
    tvix_io::TvixIO,
    tvix_store_io::TvixStoreIO,
};

fn eval_test(code_path: PathBuf, expect_success: bool) {
    assert_eq!(
        code_path.extension().unwrap(),
        "nix",
        "test files always end in .nix"
    );
    let exp_path = code_path.with_extension("exp");
    let exp_xml_path = code_path.with_extension("exp.xml");

    let code = std::fs::read_to_string(&code_path).expect("should be able to read test code");

    if exp_xml_path.exists() {
        // We can't test them at the moment because we don't have XML output yet.
        // Checking for success / failure only is a bit disingenious.
        return;
    }

    let tvix_store_io = Rc::new(TvixStoreIO::new(Default::default()));
    // Wrap with TvixIO, so <nix/fetchurl.nix can be imported.
    let mut eval_builder = tvix_eval::Evaluation::builder(Rc::new(TvixIO::new(
        tvix_store_io.clone() as Rc<dyn EvalIO>,
    )) as Rc<dyn EvalIO>)
    .enable_import()
    .mode(EvalMode::Strict);

    eval_builder = add_derivation_builtins(eval_builder, Rc::clone(&tvix_store_io));
    // eval_builder = add_fetcher_builtins(eval_builder, Rc::clone(&tvix_store_io));
    eval_builder = add_import_builtins(eval_builder, tvix_store_io);
    eval_builder = configure_nix_path(eval_builder, &None);

    let eval = eval_builder.build();

    let result = eval.evaluate(code, Some(code_path.clone()));
    let failed = match result.value {
        Some(Value::Catchable(_)) => true,
        _ => !result.errors.is_empty(),
    };
    if expect_success && failed {
        panic!(
            "{}: evaluation of eval-okay test should succeed, but failed with {:?}",
            code_path.display(),
            result.errors,
        );
    }

    if !expect_success && failed {
        return;
    }

    let value = result.value.unwrap();
    let result_str = value.to_string();

    if let Ok(exp) = std::fs::read_to_string(exp_path) {
        if expect_success {
            assert_eq!(
                result_str,
                exp.trim(),
                "{}: result value representation (left) must match expectation (right)",
                code_path.display()
            );
        } else {
            assert_ne!(
                result_str,
                exp.trim(),
                "{}: test passed unexpectedly!  consider moving it out of notyetpassing",
                code_path.display()
            );
        }
    } else if expect_success {
        panic!(
            "{}: should be able to read test expectation",
            code_path.display()
        );
    } else {
        panic!(
            "{}: test should have failed, but succeeded with output {}",
            code_path.display(),
            result_str
        );
    }
}

// eval-okay-* tests contain a snippet of Nix code, and an expectation
// of the produced string output of the evaluator.
//
// These evaluations are always supposed to succeed, i.e. all snippets
// are guaranteed to be valid Nix code.
#[rstest]
fn eval_okay(#[files("src/tests/tvix_tests/eval-okay-*.nix")] code_path: PathBuf) {
    eval_test(code_path, true)
}

// eval-okay-* tests from the original Nix test suite.
#[cfg(feature = "nix_tests")]
#[rstest]
fn nix_eval_okay(#[files("src/tests/nix_tests/eval-okay-*.nix")] code_path: PathBuf) {
    eval_test(code_path, true)
}

// eval-okay-* tests from the original Nix test suite which do not yet pass for tvix
//
// Eventually there will be none of these left, and this function
// will disappear :) Until then, to run these tests, use `cargo test
// --features expected_failures`.
//
// Please don't submit failing tests unless they're in
// notyetpassing; this makes the test suite much more useful for
// regression testing, since there should always be zero non-ignored
// failing tests.
//
// NOTE: There's no such test anymore. `rstest` does not handle empty directories, so, we
// just comment it for now.
//
// #[rstest]
// fn nix_eval_okay_currently_failing(
//     #[files("src/tests/nix_tests/notyetpassing/eval-okay-*.nix")] code_path: PathBuf,
// ) {
//     eval_test(code_path, false)
// }

// eval-fail-* tests contain a snippet of Nix code, which is
// expected to fail evaluation.  The exact type of failure
// (assertion, parse error, etc) is not currently checked.
#[rstest]
fn eval_fail(#[files("src/tests/tvix_tests/eval-fail-*.nix")] code_path: PathBuf) {
    eval_test(code_path, false)
}
