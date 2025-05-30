//! This module contains glue code translating from
//! [nix_compat::derivation::Derivation] to [tvix_build::buildservice::BuildRequest].

use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;

use bytes::Bytes;
use nix_compat::{derivation::Derivation, nixbase32, store_path::StorePath};
use sha2::{Digest, Sha256};
use tvix_build::buildservice::{AdditionalFile, BuildConstraints, BuildRequest, EnvVar};
use tvix_castore::Node;

/// These are the environment variables that Nix sets in its sandbox for every
/// build.
#[allow(dead_code)] // TODO(tazjin): keeping this, might become useful?
const NIX_ENVIRONMENT_VARS: [(&str, &str); 12] = [
    ("HOME", "/homeless-shelter"),
    ("NIX_BUILD_CORES", "0"), // TODO: make this configurable?
    ("NIX_BUILD_TOP", "/build"),
    ("NIX_LOG_FD", "2"),
    ("NIX_STORE", "/nix/store"),
    ("PATH", "/path-not-set"),
    ("PWD", "/build"),
    ("TEMP", "/build"),
    ("TEMPDIR", "/build"),
    ("TERM", "xterm-256color"),
    ("TMP", "/build"),
    ("TMPDIR", "/build"),
];

/// Get an iterator of store paths whose nixbase32 hashes will be the needles for refscanning
/// Importantly, the returned order will match the one used by [derivation_to_build_request]
/// so users may use this function to map back from the found needles to a store path
pub(crate) fn get_refscan_needles(
    derivation: &Derivation,
) -> impl Iterator<Item = &StorePath<String>> {
    derivation
        .outputs
        .values()
        .filter_map(|output| output.path.as_ref())
        .chain(derivation.input_sources.iter())
        .chain(derivation.input_derivations.keys())
}

/// Takes a [Derivation] and turns it into a [buildservice::BuildRequest].
/// It assumes the Derivation has been validated.
/// It needs two lookup functions:
/// - one translating input sources to a castore node
///   (`fn_input_sources_to_node`)
/// - one translating a tuple of drv path and (a subset of their) output names to
///   castore nodes of the selected outpus (`fn_input_drvs_to_output_nodes`).
#[allow(dead_code)]
pub(crate) fn derivation_to_build_request(
    derivation: &Derivation,
    inputs: BTreeMap<StorePath<String>, Node>,
) -> std::io::Result<BuildRequest> {
    debug_assert!(derivation.validate(true).is_ok(), "drv must validate");

    // produce command_args, which is builder and arguments in a Vec.
    let mut command_args: Vec<String> = Vec::with_capacity(derivation.arguments.len() + 1);
    command_args.push(derivation.builder.clone());
    command_args.extend_from_slice(&derivation.arguments);

    // Produce environment_vars and additional files.
    // We use a BTreeMap while producing, and only realize the resulting Vec
    // while populating BuildRequest, so we don't need to worry about ordering.
    let mut environment_vars: BTreeMap<String, Bytes> = BTreeMap::new();
    let mut additional_files: BTreeMap<String, Bytes> = BTreeMap::new();

    // Start with some the ones that nix magically sets:
    environment_vars.extend(
        NIX_ENVIRONMENT_VARS
            .iter()
            .map(|(k, v)| (k.to_string(), Bytes::from_static(v.as_bytes()))),
    );

    // extend / overwrite with the keys set in the derivation environment itself.
    // TODO: check if this order is correct, and environment vars set in the
    // *Derivation actually* have priority.
    environment_vars.extend(
        derivation
            .environment
            .iter()
            .map(|(k, v)| (k.clone(), Bytes::from(v.to_vec()))),
    );

    handle_pass_as_file(&mut environment_vars, &mut additional_files)?;

    // TODO: handle __json (structured attrs, provide JSON file and source-able bash script)

    // Produce constraints.
    let mut constraints = HashSet::from([
        BuildConstraints::System(derivation.system.clone()),
        BuildConstraints::ProvideBinSh,
    ]);

    if derivation.outputs.len() == 1
        && derivation
            .outputs
            .get("out")
            .expect("Tvix bug: Derivation has no out output")
            .is_fixed()
    {
        constraints.insert(BuildConstraints::NetworkAccess);
    }

    Ok(BuildRequest {
        // Importantly, this must match the order of get_refscan_needles, since users may use that
        // function to map back from the found needles to a store path
        refscan_needles: get_refscan_needles(derivation)
            .map(|path| nixbase32::encode(path.digest()))
            .collect(),
        command_args,

        outputs: {
            // produce output_paths, which is the absolute path of each output (sorted)
            let mut output_paths: Vec<PathBuf> = derivation
                .outputs
                .values()
                .map(|e| PathBuf::from(e.path_str()[1..].to_owned()))
                .collect();

            // Sort the outputs. We can use sort_unstable, as these are unique strings.
            output_paths.sort_unstable();
            output_paths
        },

        // Turn this into a sorted-by-key Vec<EnvVar>.
        environment_vars: environment_vars
            .into_iter()
            .map(|(key, value)| EnvVar { key, value })
            .collect(),
        inputs: inputs
            .into_iter()
            .map(|(path, node)| {
                (
                    path.to_string()
                        .as_str()
                        .try_into()
                        .expect("Tvix bug: unable to convert store path basename to PathComponent"),
                    node,
                )
            })
            .collect(),
        inputs_dir: nix_compat::store_path::STORE_DIR[1..].into(),
        constraints,
        working_dir: "build".into(),
        scratch_paths: vec!["build".into(), "nix/store".into()],
        additional_files: additional_files
            .into_iter()
            .map(|(path, contents)| AdditionalFile {
                path: PathBuf::from(path),
                contents,
            })
            .collect(),
    })
}

/// handle passAsFile, if set.
/// For each env $x in that list, the original env is removed, and a $xPath
/// environment var added instead, referring to a path inside the build with
/// the contents from the original env var.
fn handle_pass_as_file(
    environment_vars: &mut BTreeMap<String, Bytes>,
    additional_files: &mut BTreeMap<String, Bytes>,
) -> std::io::Result<()> {
    let pass_as_file = environment_vars.get("passAsFile").map(|v| {
        // Convert pass_as_file to string.
        // When it gets here, it contains a space-separated list of env var
        // keys, which must be strings.
        String::from_utf8(v.to_vec())
    });

    if let Some(pass_as_file) = pass_as_file {
        let pass_as_file = pass_as_file.map_err(|_| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "passAsFile elements are no valid utf8 strings",
            )
        })?;

        for x in pass_as_file.split(' ') {
            match environment_vars.remove_entry(x) {
                Some((k, contents)) => {
                    let (new_k, path) = calculate_pass_as_file_env(&k);

                    additional_files.insert(path[1..].to_string(), contents);
                    environment_vars.insert(new_k, Bytes::from(path));
                }
                None => {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "passAsFile refers to non-existent env key",
                    ));
                }
            }
        }
    }

    Ok(())
}

/// For a given key k in a derivation environment that's supposed to be passed as file,
/// calculate the ${k}Path key and filepath value that it's being replaced with
/// while preparing the build.
/// The filepath is `/build/.attrs-${nixbase32(sha256(key))`.
fn calculate_pass_as_file_env(k: &str) -> (String, String) {
    (
        format!("{}Path", k),
        format!(
            "/build/.attr-{}",
            nixbase32::encode(&Sha256::new_with_prefix(k).finalize())
        ),
    )
}

#[cfg(test)]
mod test {
    use bytes::Bytes;
    use nix_compat::{derivation::Derivation, store_path::StorePath};
    use std::collections::{BTreeMap, HashSet};
    use std::sync::LazyLock;
    use tvix_castore::fixtures::DUMMY_DIGEST;
    use tvix_castore::{Node, PathComponent};

    use tvix_build::buildservice::{AdditionalFile, BuildConstraints, BuildRequest, EnvVar};

    use crate::tvix_build::NIX_ENVIRONMENT_VARS;

    use super::derivation_to_build_request;

    static INPUT_NODE_FOO_NAME: LazyLock<Bytes> =
        LazyLock::new(|| "mp57d33657rf34lzvlbpfa1gjfv5gmpg-bar".into());

    static INPUT_NODE_FOO: LazyLock<Node> = LazyLock::new(|| Node::Directory {
        digest: DUMMY_DIGEST.clone(),
        size: 42,
    });

    #[test]
    fn test_derivation_to_build_request() {
        let aterm_bytes = include_bytes!("tests/ch49594n9avinrf8ip0aslidkc4lxkqv-foo.drv");

        let derivation = Derivation::from_aterm_bytes(aterm_bytes).expect("must parse");

        let build_request = derivation_to_build_request(
            &derivation,
            BTreeMap::from([(
                StorePath::<String>::from_bytes(&INPUT_NODE_FOO_NAME.clone()).unwrap(),
                INPUT_NODE_FOO.clone(),
            )]),
        )
        .expect("must succeed");

        let mut expected_environment_vars = vec![
            EnvVar {
                key: "bar".into(),
                value: "/nix/store/mp57d33657rf34lzvlbpfa1gjfv5gmpg-bar".into(),
            },
            EnvVar {
                key: "builder".into(),
                value: ":".into(),
            },
            EnvVar {
                key: "name".into(),
                value: "foo".into(),
            },
            EnvVar {
                key: "out".into(),
                value: "/nix/store/fhaj6gmwns62s6ypkcldbaj2ybvkhx3p-foo".into(),
            },
            EnvVar {
                key: "system".into(),
                value: ":".into(),
            },
        ];

        expected_environment_vars.extend(NIX_ENVIRONMENT_VARS.iter().map(|(k, v)| EnvVar {
            key: k.to_string(),
            value: Bytes::from_static(v.as_bytes()),
        }));

        expected_environment_vars.sort_unstable_by_key(|e| e.key.to_owned());

        assert_eq!(
            BuildRequest {
                command_args: vec![":".into()],
                outputs: vec!["nix/store/fhaj6gmwns62s6ypkcldbaj2ybvkhx3p-foo".into()],
                environment_vars: expected_environment_vars,
                inputs: BTreeMap::from([(
                    PathComponent::try_from(INPUT_NODE_FOO_NAME.clone()).unwrap(),
                    INPUT_NODE_FOO.clone()
                )]),
                inputs_dir: "nix/store".into(),
                constraints: HashSet::from([
                    BuildConstraints::System(derivation.system.clone()),
                    BuildConstraints::ProvideBinSh
                ]),
                additional_files: vec![],
                working_dir: "build".into(),
                scratch_paths: vec!["build".into(), "nix/store".into()],
                refscan_needles: vec![
                    "fhaj6gmwns62s6ypkcldbaj2ybvkhx3p".into(),
                    "ss2p4wmxijn652haqyd7dckxwl4c7hxx".into()
                ],
            },
            build_request
        );
    }

    #[test]
    fn test_fod_to_build_request() {
        let aterm_bytes = include_bytes!("tests/0hm2f1psjpcwg8fijsmr4wwxrx59s092-bar.drv");

        let derivation = Derivation::from_aterm_bytes(aterm_bytes).expect("must parse");

        let build_request =
            derivation_to_build_request(&derivation, BTreeMap::from([])).expect("must succeed");

        let mut expected_environment_vars = vec![
            EnvVar {
                key: "builder".into(),
                value: ":".into(),
            },
            EnvVar {
                key: "name".into(),
                value: "bar".into(),
            },
            EnvVar {
                key: "out".into(),
                value: "/nix/store/4q0pg5zpfmznxscq3avycvf9xdvx50n3-bar".into(),
            },
            EnvVar {
                key: "outputHash".into(),
                value: "08813cbee9903c62be4c5027726a418a300da4500b2d369d3af9286f4815ceba".into(),
            },
            EnvVar {
                key: "outputHashAlgo".into(),
                value: "sha256".into(),
            },
            EnvVar {
                key: "outputHashMode".into(),
                value: "recursive".into(),
            },
            EnvVar {
                key: "system".into(),
                value: ":".into(),
            },
        ];

        expected_environment_vars.extend(NIX_ENVIRONMENT_VARS.iter().map(|(k, v)| EnvVar {
            key: k.to_string(),
            value: Bytes::from_static(v.as_bytes()),
        }));

        expected_environment_vars.sort_unstable_by_key(|e| e.key.to_owned());

        assert_eq!(
            BuildRequest {
                command_args: vec![":".to_string()],
                outputs: vec!["nix/store/4q0pg5zpfmznxscq3avycvf9xdvx50n3-bar".into()],
                environment_vars: expected_environment_vars,
                inputs: BTreeMap::new(),
                inputs_dir: "nix/store".into(),
                constraints: HashSet::from([
                    BuildConstraints::System(derivation.system.clone()),
                    BuildConstraints::NetworkAccess,
                    BuildConstraints::ProvideBinSh
                ]),
                additional_files: vec![],
                working_dir: "build".into(),
                scratch_paths: vec!["build".into(), "nix/store".into()],
                refscan_needles: vec!["4q0pg5zpfmznxscq3avycvf9xdvx50n3".into()],
            },
            build_request
        );
    }

    #[test]
    fn test_pass_as_file() {
        // (builtins.derivation { "name" = "foo"; passAsFile = ["bar" "baz"]; bar = "baz"; baz = "bar"; system = ":"; builder = ":";}).drvPath
        let aterm_bytes = r#"Derive([("out","/nix/store/pp17lwra2jkx8rha15qabg2q3wij72lj-foo","","")],[],[],":",":",[],[("bar","baz"),("baz","bar"),("builder",":"),("name","foo"),("out","/nix/store/pp17lwra2jkx8rha15qabg2q3wij72lj-foo"),("passAsFile","bar baz"),("system",":")])"#.as_bytes();

        let derivation = Derivation::from_aterm_bytes(aterm_bytes).expect("must parse");

        let build_request =
            derivation_to_build_request(&derivation, BTreeMap::from([])).expect("must succeed");

        let mut expected_environment_vars = vec![
            // Note how bar and baz are not present in the env anymore,
            // but replaced with barPath, bazPath respectively.
            EnvVar {
                key: "barPath".into(),
                value: "/build/.attr-1fcgpy7vc4ammr7s17j2xq88scswkgz23dqzc04g8sx5vcp2pppw".into(),
            },
            EnvVar {
                key: "bazPath".into(),
                value: "/build/.attr-15l04iksj1280dvhbzdq9ai3wlf8ac2188m9qv0gn81k9nba19ds".into(),
            },
            EnvVar {
                key: "builder".into(),
                value: ":".into(),
            },
            EnvVar {
                key: "name".into(),
                value: "foo".into(),
            },
            EnvVar {
                key: "out".into(),
                value: "/nix/store/pp17lwra2jkx8rha15qabg2q3wij72lj-foo".into(),
            },
            // passAsFile stays around
            EnvVar {
                key: "passAsFile".into(),
                value: "bar baz".into(),
            },
            EnvVar {
                key: "system".into(),
                value: ":".into(),
            },
        ];

        expected_environment_vars.extend(NIX_ENVIRONMENT_VARS.iter().map(|(k, v)| EnvVar {
            key: k.to_string(),
            value: Bytes::from_static(v.as_bytes()),
        }));

        expected_environment_vars.sort_unstable_by_key(|e| e.key.to_owned());

        assert_eq!(
            BuildRequest {
                command_args: vec![":".to_string()],
                outputs: vec!["nix/store/pp17lwra2jkx8rha15qabg2q3wij72lj-foo".into()],
                environment_vars: expected_environment_vars,
                inputs: BTreeMap::new(),
                inputs_dir: "nix/store".into(),
                constraints: HashSet::from([
                    BuildConstraints::System(derivation.system.clone()),
                    BuildConstraints::ProvideBinSh,
                ]),
                additional_files: vec![
                    // baz env
                    AdditionalFile {
                        path: "build/.attr-15l04iksj1280dvhbzdq9ai3wlf8ac2188m9qv0gn81k9nba19ds"
                            .into(),
                        contents: "bar".into()
                    },
                    // bar env
                    AdditionalFile {
                        path: "build/.attr-1fcgpy7vc4ammr7s17j2xq88scswkgz23dqzc04g8sx5vcp2pppw"
                            .into(),
                        contents: "baz".into(),
                    },
                ],
                working_dir: "build".into(),
                scratch_paths: vec!["build".into(), "nix/store".into()],
                refscan_needles: vec!["pp17lwra2jkx8rha15qabg2q3wij72lj".into()],
            },
            build_request
        );
    }
}
