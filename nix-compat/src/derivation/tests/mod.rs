use super::parse_error::ErrorKind;
use crate::derivation::output::Output;
use crate::derivation::parse_error::NomError;
use crate::derivation::parser::Error;
use crate::derivation::Derivation;
use crate::store_path::StorePath;
use bstr::{BStr, BString};
use hex_literal::hex;
use rstest::rstest;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr;

const RESOURCES_PATHS: &str = "src/derivation/tests/derivation_tests";

#[rstest]
fn check_serialization(
    #[files("src/derivation/tests/derivation_tests/ok/*.drv")]
    #[exclude("(cp1252)|(latin1)")] // skip JSON files known to fail parsing
    path_to_drv_file: PathBuf,
) {
    let json_bytes =
        fs::read(path_to_drv_file.with_extension("drv.json")).expect("unable to read JSON");
    let derivation: Derivation =
        serde_json::from_slice(&json_bytes).expect("JSON was not well-formatted");

    let mut serialized_derivation = Vec::new();
    derivation.serialize(&mut serialized_derivation).unwrap();

    let expected = fs::read(&path_to_drv_file).expect("unable to read .drv");

    assert_eq!(expected, BStr::new(&serialized_derivation));
}

#[rstest]
fn validate(
    #[files("src/derivation/tests/derivation_tests/ok/*.drv")]
    #[exclude("(cp1252)|(latin1)")] // skip JSON files known to fail parsing
    path_to_drv_file: PathBuf,
) {
    let json_bytes =
        fs::read(path_to_drv_file.with_extension("drv.json")).expect("unable to read JSON");
    let derivation: Derivation =
        serde_json::from_slice(&json_bytes).expect("JSON was not well-formatted");

    derivation
        .validate(true)
        .expect("derivation failed to validate")
}

#[rstest]
fn check_to_aterm_bytes(
    #[files("src/derivation/tests/derivation_tests/ok/*.drv")]
    #[exclude("(cp1252)|(latin1)")] // skip JSON files known to fail parsing
    path_to_drv_file: PathBuf,
) {
    let json_bytes =
        fs::read(path_to_drv_file.with_extension("drv.json")).expect("unable to read JSON");
    let derivation: Derivation =
        serde_json::from_slice(&json_bytes).expect("JSON was not well-formatted");

    let expected = fs::read(&path_to_drv_file).expect("unable to read .drv");

    assert_eq!(expected, BStr::new(&derivation.to_aterm_bytes()));
}

/// Reads in derivations in ATerm representation, parses with that parser,
/// then compares the structs with the ones obtained by parsing the JSON
/// representations.
#[rstest]
fn from_aterm_bytes(
    #[files("src/derivation/tests/derivation_tests/ok/*.drv")] path_to_drv_file: PathBuf,
) {
    // Read in ATerm representation.
    let aterm_bytes = fs::read(&path_to_drv_file).expect("unable to read .drv");
    let parsed_drv = Derivation::from_aterm_bytes(&aterm_bytes).expect("must succeed");

    // For where we're able to load JSON fixtures, parse them and compare the structs.
    // For where we're not, compare the bytes manually.
    if path_to_drv_file.file_name().is_some_and(|s| {
        s.as_encoded_bytes().ends_with(b"cp1252.drv")
            || s.as_encoded_bytes().ends_with(b"latin1.drv")
    }) {
        assert_eq!(
            &[0xc5, 0xc4, 0xd6][..],
            parsed_drv.environment.get("chars").unwrap(),
            "expected bytes to match",
        );
    } else {
        let json_bytes =
            fs::read(path_to_drv_file.with_extension("drv.json")).expect("unable to read JSON");
        let fixture_derivation: Derivation =
            serde_json::from_slice(&json_bytes).expect("JSON was not well-formatted");

        assert_eq!(fixture_derivation, parsed_drv);
    }

    // Finally, write the ATerm serialization to another buffer, ensuring it's
    // stable (and we compare all fields we couldn't compare in the non-utf8
    // derivations)

    assert_eq!(
        &aterm_bytes,
        &BString::new(parsed_drv.to_aterm_bytes()),
        "expected serialized ATerm to match initial input"
    );
}

#[test]
fn from_aterm_bytes_duplicate_map_key() {
    let buf: Vec<u8> =
        fs::read(format!("{}/{}", RESOURCES_PATHS, "duplicate.drv")).expect("unable to read .drv");

    let err = Derivation::from_aterm_bytes(&buf).expect_err("must fail");

    match err {
        Error::Parser(NomError { input: _, code }) => {
            assert_eq!(code, ErrorKind::DuplicateMapKey("name".to_string()));
        }
        _ => {
            panic!("unexpected error");
        }
    }
}

/// Read in a derivation in ATerm, but add some garbage at the end.
/// Ensure the parser detects and fails in this case.
#[test]
fn from_aterm_bytes_trailer() {
    let mut buf: Vec<u8> = fs::read(format!(
        "{}/ok/{}",
        RESOURCES_PATHS, "0hm2f1psjpcwg8fijsmr4wwxrx59s092-bar.drv"
    ))
    .expect("unable to read .drv");

    buf.push(0x00);

    Derivation::from_aterm_bytes(&buf).expect_err("must fail");
}

#[rstest]
#[case::fixed_sha256("bar", "0hm2f1psjpcwg8fijsmr4wwxrx59s092-bar.drv")]
#[case::simple_sha256("foo", "4wvvbi4jwn0prsdxb7vs673qa5h9gr7x-foo.drv")]
#[case::fixed_sha1("bar", "ss2p4wmxijn652haqyd7dckxwl4c7hxx-bar.drv")]
#[case::simple_sha1("foo", "ch49594n9avinrf8ip0aslidkc4lxkqv-foo.drv")]
#[case::multiple_outputs("has-multi-out", "h32dahq0bx5rp1krcdx3a53asj21jvhk-has-multi-out.drv")]
#[case::structured_attrs(
    "structured-attrs",
    "9lj1lkjm2ag622mh4h9rpy6j607an8g2-structured-attrs.drv"
)]
#[case::unicode("unicode", "52a9id8hx688hvlnz4d1n25ml1jdykz0-unicode.drv")]
fn derivation_path(#[case] name: &str, #[case] expected_path: &str) {
    let json_bytes = fs::read(format!("{RESOURCES_PATHS}/ok/{expected_path}.json"))
        .expect("unable to read JSON");
    let derivation: Derivation =
        serde_json::from_slice(&json_bytes).expect("JSON was not well-formatted");

    assert_eq!(
        derivation.calculate_derivation_path(name).unwrap(),
        StorePath::from_str(expected_path).unwrap()
    );
}

/// This trims all output paths from a Derivation struct,
/// by setting outputs[$outputName].path and environment[$outputName] to the empty string.
fn derivation_without_output_paths(derivation: &Derivation) -> Derivation {
    let mut trimmed_env = derivation.environment.clone();
    let mut trimmed_outputs = derivation.outputs.clone();

    for (output_name, output) in &derivation.outputs {
        trimmed_env.insert(output_name.clone(), "".into());
        assert!(trimmed_outputs.contains_key(output_name));
        trimmed_outputs.insert(
            output_name.to_string(),
            Output {
                path: None,
                ..output.clone()
            },
        );
    }

    // replace environment and outputs with the trimmed variants
    Derivation {
        environment: trimmed_env,
        outputs: trimmed_outputs,
        ..derivation.clone()
    }
}

#[rstest]
#[case::fixed_sha256("0hm2f1psjpcwg8fijsmr4wwxrx59s092-bar.drv", hex!("724f3e3634fce4cbbbd3483287b8798588e80280660b9a63fd13a1bc90485b33"))]
#[case::fixed_sha1("ss2p4wmxijn652haqyd7dckxwl4c7hxx-bar.drv", hex!("c79aebd0ce3269393d4a1fde2cbd1d975d879b40f0bf40a48f550edc107fd5df"))]
fn hash_derivation_modulo_fixed(#[case] drv_path: &str, #[case] expected_digest: [u8; 32]) {
    // read in the fixture
    let json_bytes =
        fs::read(format!("{RESOURCES_PATHS}/ok/{drv_path}.json")).expect("unable to read JSON");
    let drv: Derivation = serde_json::from_slice(&json_bytes).expect("must deserialize");

    let actual = drv.hash_derivation_modulo(|_| panic!("must not be called"));
    assert_eq!(expected_digest, actual);
}

/// This reads a Derivation (in A-Term), trims out all fields containing
/// calculated output paths, then triggers the output path calculation and
/// compares the struct to match what was originally read in.
#[rstest]
#[case::fixed_sha256("bar", "0hm2f1psjpcwg8fijsmr4wwxrx59s092-bar.drv")]
#[case::simple_sha256("foo", "4wvvbi4jwn0prsdxb7vs673qa5h9gr7x-foo.drv")]
#[case::fixed_sha1("bar", "ss2p4wmxijn652haqyd7dckxwl4c7hxx-bar.drv")]
#[case::simple_sha1("foo", "ch49594n9avinrf8ip0aslidkc4lxkqv-foo.drv")]
#[case::multiple_outputs("has-multi-out", "h32dahq0bx5rp1krcdx3a53asj21jvhk-has-multi-out.drv")]
#[case::structured_attrs(
    "structured-attrs",
    "9lj1lkjm2ag622mh4h9rpy6j607an8g2-structured-attrs.drv"
)]
#[case::unicode("unicode", "52a9id8hx688hvlnz4d1n25ml1jdykz0-unicode.drv")]
#[case::cp1252("cp1252", "m1vfixn8iprlf0v9abmlrz7mjw1xj8kp-cp1252.drv")]
#[case::latin1("latin1", "x6p0hg79i3wg0kkv7699935f7rrj9jf3-latin1.drv")]
fn output_paths(#[case] name: &str, #[case] drv_path_str: &str) {
    // read in the derivation
    let expected_derivation = Derivation::from_aterm_bytes(
        &fs::read(format!("{RESOURCES_PATHS}/ok/{drv_path_str}")).expect("unable to read .drv"),
    )
    .expect("must succeed");

    // create a version without output paths, simulating we constructed the
    // struct.
    let mut derivation = derivation_without_output_paths(&expected_derivation);

    // calculate the hash_derivation_modulo of Derivation
    // We don't expect the lookup function to be called for most derivations.
    let actual_hash_derivation_modulo = derivation.hash_derivation_modulo(|parent_drv_path| {
        // 4wvvbi4jwn0prsdxb7vs673qa5h9gr7x-foo.drv may lookup /nix/store/0hm2f1psjpcwg8fijsmr4wwxrx59s092-bar.drv
        // ch49594n9avinrf8ip0aslidkc4lxkqv-foo.drv may lookup /nix/store/ss2p4wmxijn652haqyd7dckxwl4c7hxx-bar.drv
        if name == "foo"
            && ((drv_path_str == "4wvvbi4jwn0prsdxb7vs673qa5h9gr7x-foo.drv"
                && parent_drv_path.to_string() == "0hm2f1psjpcwg8fijsmr4wwxrx59s092-bar.drv")
                || (drv_path_str == "ch49594n9avinrf8ip0aslidkc4lxkqv-foo.drv"
                    && parent_drv_path.to_string() == "ss2p4wmxijn652haqyd7dckxwl4c7hxx-bar.drv"))
        {
            // do the lookup, by reading in the fixture of the requested
            // drv_name, and calculating its drv replacement (on the non-stripped version)
            // In a real-world scenario you would have already done this during construction.

            let json_bytes = fs::read(format!(
                "{}/ok/{}.json",
                RESOURCES_PATHS,
                Path::new(&parent_drv_path.to_string())
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
            ))
            .expect("unable to read JSON");

            let drv: Derivation = serde_json::from_slice(&json_bytes).expect("must deserialize");

            // calculate hash_derivation_modulo for each parent.
            // This may not trigger subsequent requests, as both parents are FOD.
            drv.hash_derivation_modulo(|_| panic!("must not lookup"))
        } else {
            // we only expect this to be called in the "foo" testcase, for the "bar derivations"
            panic!("may only be called for foo testcase on bar derivations");
        }
    });

    derivation
        .calculate_output_paths(name, &actual_hash_derivation_modulo)
        .unwrap();

    // The derivation should now look like it was before
    assert_eq!(expected_derivation, derivation);
}

/// Exercises the output path calculation functions like a constructing client
/// (an implementation of builtins.derivation) would do:
///
/// ```nix
/// rec {
///   bar = builtins.derivation {
///     name = "bar";
///     builder = ":";
///     system = ":";
///     outputHash = "08813cbee9903c62be4c5027726a418a300da4500b2d369d3af9286f4815ceba";
///     outputHashAlgo = "sha256";
///     outputHashMode = "recursive";
///   };
///
///   foo = builtins.derivation {
///     name = "foo";
///     builder = ":";
///     system = ":";
///     inherit bar;
///   };
/// }
/// ```
/// It first assembles the bar derivation, does the output path calculation on
/// it, then continues with the foo derivation.
///
/// The code ensures the resulting Derivations match our fixtures.
#[test]
fn output_path_construction() {
    // create the bar derivation
    let mut bar_drv = Derivation {
        builder: ":".to_string(),
        system: ":".to_string(),
        ..Default::default()
    };

    // assemble bar env
    let bar_env = &mut bar_drv.environment;
    bar_env.insert("builder".to_string(), ":".into());
    bar_env.insert("name".to_string(), "bar".into());
    bar_env.insert("out".to_string(), "".into()); // will be calculated
    bar_env.insert(
        "outputHash".to_string(),
        "08813cbee9903c62be4c5027726a418a300da4500b2d369d3af9286f4815ceba".into(),
    );
    bar_env.insert("outputHashAlgo".to_string(), "sha256".into());
    bar_env.insert("outputHashMode".to_string(), "recursive".into());
    bar_env.insert("system".to_string(), ":".into());

    // assemble bar outputs
    bar_drv.outputs.insert(
        "out".to_string(),
        Output {
            path: None, // will be calculated
            ca_hash: Some(crate::nixhash::CAHash::Nar(
                crate::nixhash::from_algo_and_digest(
                    crate::nixhash::HashAlgo::Sha256,
                    &data_encoding::HEXLOWER
                        .decode(
                            "08813cbee9903c62be4c5027726a418a300da4500b2d369d3af9286f4815ceba"
                                .as_bytes(),
                        )
                        .unwrap(),
                )
                .unwrap(),
            )),
        },
    );

    // calculate bar output paths
    let bar_calc_result = bar_drv.calculate_output_paths(
        "bar",
        &bar_drv.hash_derivation_modulo(|_| panic!("is FOD, should not lookup")),
    );
    assert!(bar_calc_result.is_ok());

    // ensure it matches our bar fixture
    let bar_json_bytes = fs::read(format!(
        "{}/ok/{}.json",
        RESOURCES_PATHS, "0hm2f1psjpcwg8fijsmr4wwxrx59s092-bar.drv"
    ))
    .expect("unable to read JSON");
    let bar_drv_expected: Derivation =
        serde_json::from_slice(&bar_json_bytes).expect("must deserialize");
    assert_eq!(bar_drv_expected, bar_drv);

    // now construct foo, which requires bar_drv
    // Note how we refer to the output path, drv name and replacement_str (with calculated output paths) of bar.
    let bar_output_path = &bar_drv.outputs.get("out").expect("must exist").path;
    let bar_drv_hash_derivation_modulo =
        bar_drv.hash_derivation_modulo(|_| panic!("is FOD, should not lookup"));

    let bar_drv_path = bar_drv
        .calculate_derivation_path("bar")
        .expect("must succeed");

    // create foo derivation
    let mut foo_drv = Derivation {
        builder: ":".to_string(),
        system: ":".to_string(),
        ..Default::default()
    };

    // assemble foo env
    let foo_env = &mut foo_drv.environment;
    // foo_env.insert("bar".to_string(), StorePathRef:: bar_output_path.to_owned().try_into().unwrap());
    foo_env.insert(
        "bar".to_string(),
        bar_output_path
            .as_ref()
            .unwrap()
            .to_absolute_path()
            .as_bytes()
            .into(),
    );
    foo_env.insert("builder".to_string(), ":".into());
    foo_env.insert("name".to_string(), "foo".into());
    foo_env.insert("out".to_string(), "".into()); // will be calculated
    foo_env.insert("system".to_string(), ":".into());

    // asssemble foo outputs
    foo_drv.outputs.insert(
        "out".to_string(),
        Output {
            path: None, // will be calculated
            ca_hash: None,
        },
    );

    // assemble foo input_derivations
    foo_drv
        .input_derivations
        .insert(bar_drv_path, BTreeSet::from(["out".to_string()]));

    // calculate foo output paths
    let foo_calc_result = foo_drv.calculate_output_paths(
        "foo",
        &foo_drv.hash_derivation_modulo(|drv_path| {
            if drv_path.to_string() != "0hm2f1psjpcwg8fijsmr4wwxrx59s092-bar.drv" {
                panic!("lookup called with unexpected drv_path: {drv_path}");
            }
            bar_drv_hash_derivation_modulo
        }),
    );
    assert!(foo_calc_result.is_ok());

    // ensure it matches our foo fixture
    let foo_json_bytes = fs::read(format!(
        "{}/ok/{}.json",
        RESOURCES_PATHS, "4wvvbi4jwn0prsdxb7vs673qa5h9gr7x-foo.drv",
    ))
    .expect("unable to read JSON");
    let foo_drv_expected: Derivation =
        serde_json::from_slice(&foo_json_bytes).expect("must deserialize");
    assert_eq!(foo_drv_expected, foo_drv);

    assert_eq!(
        StorePath::from_str("4wvvbi4jwn0prsdxb7vs673qa5h9gr7x-foo.drv").expect("must succeed"),
        foo_drv
            .calculate_derivation_path("foo")
            .expect("must succeed")
    );
}
