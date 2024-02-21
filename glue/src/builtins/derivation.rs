//! Implements `builtins.derivation`, the core of what makes Nix build packages.
use crate::builtins::DerivationError;
use crate::tvix_store_io::TvixStoreIO;
use bstr::BString;
use nix_compat::derivation::{Derivation, Output};
use nix_compat::nixhash;
use nix_compat::store_path::{StorePath, StorePathRef};
use std::collections::{btree_map, BTreeSet};
use std::rc::Rc;
use tvix_eval::builtin_macros::builtins;
use tvix_eval::generators::{self, emit_warning_kind, GenCo};
use tvix_eval::{
    AddContext, CatchableErrorKind, CoercionKind, ErrorKind, NixAttrs, NixContext,
    NixContextElement, Value, WarningKind,
};

// Constants used for strangely named fields in derivation inputs.
const STRUCTURED_ATTRS: &str = "__structuredAttrs";
const IGNORE_NULLS: &str = "__ignoreNulls";

/// Populate the inputs of a derivation from the build references
/// found when scanning the derivation's parameters and extracting their contexts.
fn populate_inputs(drv: &mut Derivation, full_context: NixContext) {
    for element in full_context.iter() {
        match element {
            NixContextElement::Plain(source) => {
                let sp = StorePathRef::from_absolute_path(source.as_bytes())
                    .expect("invalid store path")
                    .to_owned();
                drv.input_sources.insert(sp);
            }

            NixContextElement::Single {
                name,
                derivation: derivation_str,
            } => {
                // TODO: b/264
                // We assume derivations to be passed validated, so ignoring rest
                // and expecting parsing is ok.
                let (derivation, _rest) =
                    StorePath::from_absolute_path_full(derivation_str).expect("valid store path");

                #[cfg(debug_assertions)]
                assert!(
                    _rest.iter().next().is_none(),
                    "Extra path not empty for {}",
                    derivation_str
                );

                match drv.input_derivations.entry(derivation.clone()) {
                    btree_map::Entry::Vacant(entry) => {
                        entry.insert(BTreeSet::from([name.clone()]));
                    }

                    btree_map::Entry::Occupied(mut entry) => {
                        entry.get_mut().insert(name.clone());
                    }
                }
            }

            NixContextElement::Derivation(_drv_path) => {
                // This is a hard one, it means that
                // we are depending on a drvPath of ourselves
                // *or* another derivation's drvPath.
                // What to do here?
                panic!("please do not depend on drvPath, I have 2 hours of sleep in blood");
            }
        }
    }
}

/// Populate the output configuration of a derivation based on the
/// parameters passed to the call, configuring a fixed-output derivation output
/// if necessary.
///
/// This function handles all possible combinations of the
/// parameters, including invalid ones.
///
/// Due to the support for SRI hashes, and how these are passed along to
/// builtins.derivation, outputHash and outputHashAlgo can have values which
/// need to be further modified before constructing the Derivation struct.
///
/// If outputHashAlgo is an SRI hash, outputHashAlgo must either be an empty
/// string, or the hash algorithm as specified in the (single) SRI (entry).
/// SRI strings with multiple hash algorithms are not supported.
///
/// In case an SRI string was used, the (single) fixed output is populated
/// with the hash algo name, and the hash digest is populated with the
/// (lowercase) hex encoding of the digest.
///
/// These values are only rewritten for the outputs, not what's passed to env.
///
/// The return value may optionally contain a warning.
fn handle_fixed_output(
    drv: &mut Derivation,
    hash_str: Option<String>,      // in nix: outputHash
    hash_algo_str: Option<String>, // in nix: outputHashAlgo
    hash_mode_str: Option<String>, // in nix: outputHashmode
) -> Result<Option<WarningKind>, ErrorKind> {
    // If outputHash is provided, ensure hash_algo_str is compatible.
    // If outputHash is not provided, do nothing.
    if let Some(hash_str) = hash_str {
        // treat an empty algo as None
        let hash_algo_str = match hash_algo_str {
            Some(s) if s.is_empty() => None,
            Some(s) => Some(s),
            None => None,
        };

        // construct a NixHash.
        let nixhash = nixhash::from_str(&hash_str, hash_algo_str.as_deref())
            .map_err(DerivationError::InvalidOutputHash)?;
        let algo = nixhash.algo();

        // construct the fixed output.
        drv.outputs.insert(
            "out".to_string(),
            Output {
                path: None,
                ca_hash: match hash_mode_str.as_deref() {
                    None | Some("flat") => Some(nixhash::CAHash::Flat(nixhash)),
                    Some("recursive") => Some(nixhash::CAHash::Nar(nixhash)),
                    Some(other) => {
                        return Err(DerivationError::InvalidOutputHashMode(other.to_string()))?
                    }
                },
            },
        );

        // Peek at hash_str once more.
        // If it was a SRI hash, but is not using the correct length, this means
        // the padding was wrong. Emit a warning in that case.
        let sri_prefix = format!("{}-", algo);
        if let Some(rest) = hash_str.strip_prefix(&sri_prefix) {
            if data_encoding::BASE64.encode_len(algo.digest_length()) != rest.len() {
                return Ok(Some(WarningKind::SRIHashWrongPadding));
            }
        }
    }
    Ok(None)
}

#[builtins(state = "Rc<TvixStoreIO>")]
pub(crate) mod derivation_builtins {
    use std::collections::BTreeMap;

    use super::*;
    use bstr::ByteSlice;
    use nix_compat::store_path::hash_placeholder;
    use tvix_eval::generators::Gen;
    use tvix_eval::{NixContext, NixContextElement, NixString};

    #[builtin("placeholder")]
    async fn builtin_placeholder(co: GenCo, input: Value) -> Result<Value, ErrorKind> {
        if input.is_catchable() {
            return Ok(input);
        }

        let placeholder = hash_placeholder(
            input
                .to_str()
                .context("looking at output name in builtins.placeholder")?
                .to_str()?,
        );

        Ok(placeholder.into())
    }

    /// Strictly construct a Nix derivation from the supplied arguments.
    ///
    /// This is considered an internal function, users usually want to
    /// use the higher-level `builtins.derivation` instead.
    #[builtin("derivationStrict")]
    async fn builtin_derivation_strict(
        state: Rc<TvixStoreIO>,
        co: GenCo,
        input: Value,
    ) -> Result<Value, ErrorKind> {
        if input.is_catchable() {
            return Ok(input);
        }

        let input = input.to_attrs()?;
        let name = generators::request_force(&co, input.select_required("name")?.clone()).await;

        if name.is_catchable() {
            return Ok(name);
        }

        let name = name.to_str().context("determining derivation name")?;
        if name.is_empty() {
            return Err(ErrorKind::Abort("derivation has empty name".to_string()));
        }
        let name = name.to_str()?;

        let mut drv = Derivation::default();
        drv.outputs.insert("out".to_string(), Default::default());
        let mut input_context = NixContext::new();

        #[inline]
        async fn strong_importing_coerce_to_string(
            co: &GenCo,
            val: Value,
        ) -> Result<NixString, CatchableErrorKind> {
            let val = generators::request_force(co, val).await;
            match generators::request_string_coerce(
                co,
                val,
                CoercionKind {
                    strong: true,
                    import_paths: true,
                },
            )
            .await
            {
                Err(cek) => Err(cek),
                Ok(val_str) => Ok(val_str),
            }
        }

        /// Inserts a key and value into the drv.environment BTreeMap, and fails if the
        /// key did already exist before.
        fn insert_env(
            drv: &mut Derivation,
            k: &str, /* TODO: non-utf8 env keys */
            v: BString,
        ) -> Result<(), DerivationError> {
            if drv.environment.insert(k.into(), v).is_some() {
                return Err(DerivationError::DuplicateEnvVar(k.into()));
            }
            Ok(())
        }

        // Check whether null attributes should be ignored or passed through.
        let ignore_nulls = match input.select(IGNORE_NULLS) {
            Some(b) => generators::request_force(&co, b.clone()).await.as_bool()?,
            None => false,
        };

        // peek at the STRUCTURED_ATTRS argument.
        // If it's set and true, provide a BTreeMap that gets populated while looking at the arguments.
        // We need it to be a BTreeMap, so iteration order of keys is reproducible.
        let mut structured_attrs: Option<BTreeMap<String, serde_json::Value>> =
            match input.select(STRUCTURED_ATTRS) {
                Some(b) => generators::request_force(&co, b.clone())
                    .await
                    .as_bool()?
                    .then_some(Default::default()),
                None => None,
            };

        // Look at the arguments passed to builtins.derivationStrict.
        // Some set special fields in the Derivation struct, some change
        // behaviour of other functionality.
        for (arg_name, arg_value) in input.clone().into_iter_sorted() {
            let arg_name = arg_name.to_str()?;
            // force the current value.
            let value = generators::request_force(&co, arg_value).await;

            // filter out nulls if ignore_nulls is set.
            if ignore_nulls && matches!(value, Value::Null) {
                continue;
            }

            match arg_name {
                // Command line arguments to the builder.
                // These are only set in drv.arguments.
                "args" => {
                    for arg in value.to_list()? {
                        match strong_importing_coerce_to_string(&co, arg).await {
                            Err(cek) => return Ok(Value::from(cek)),
                            Ok(s) => {
                                input_context.mimic(&s);
                                drv.arguments.push(s.to_str()?.to_owned())
                            }
                        }
                    }
                }

                // If outputs is set, remove the original default `out` output,
                // and replace it with the list of outputs.
                "outputs" => {
                    let outputs = value
                        .to_list()
                        .context("looking at the `outputs` parameter of the derivation")?;

                    // Remove the original default `out` output.
                    drv.outputs.clear();

                    let mut output_names = vec![];

                    for output in outputs {
                        let output_name = generators::request_force(&co, output)
                            .await
                            .to_str()
                            .context("determining output name")?;

                        input_context.mimic(&output_name);

                        // Populate drv.outputs
                        if drv
                            .outputs
                            .insert(output_name.to_str()?.to_owned(), Default::default())
                            .is_some()
                        {
                            Err(DerivationError::DuplicateOutput(
                                output_name.to_str_lossy().into_owned(),
                            ))?
                        }
                        output_names.push(output_name.to_str()?.to_owned());
                    }

                    // Add drv.environment[outputs] unconditionally.
                    insert_env(&mut drv, arg_name, output_names.join(" ").into())?;
                    // drv.environment[$output_name] is added after the loop,
                    // with whatever is in drv.outputs[$output_name].
                }

                // handle builder and system.
                "builder" | "system" => {
                    match strong_importing_coerce_to_string(&co, value).await {
                        Err(cek) => return Ok(Value::from(cek)),
                        Ok(val_str) => {
                            input_context.mimic(&val_str);

                            if arg_name == "builder" {
                                drv.builder = val_str.to_str()?.to_owned();
                            } else {
                                drv.system = val_str.to_str()?.to_owned();
                            }

                            // Either populate drv.environment or structured_attrs.
                            if let Some(ref mut structured_attrs) = structured_attrs {
                                // No need to check for dups, we only iterate over every attribute name once
                                structured_attrs.insert(
                                    arg_name.to_owned(),
                                    val_str.to_str()?.to_owned().into(),
                                );
                            } else {
                                insert_env(&mut drv, arg_name, val_str.as_bytes().into())?;
                            }
                        }
                    }
                }

                // Don't add STRUCTURED_ATTRS if enabled.
                STRUCTURED_ATTRS if structured_attrs.is_some() => continue,
                // IGNORE_NULLS is always skipped, even if it's not set to true.
                IGNORE_NULLS => continue,

                // all other args.
                _ => {
                    // In SA case, force and add to structured attrs.
                    // In non-SA case, coerce to string and add to env.
                    if let Some(ref mut structured_attrs) = structured_attrs {
                        let val = generators::request_force(&co, value).await;
                        if val.is_catchable() {
                            return Ok(val);
                        }

                        // TODO(raitobezarius): context for json values?
                        // input_context.mimic(&val);

                        let val_json = match val.into_json(&co).await? {
                            Ok(v) => v,
                            Err(cek) => return Ok(Value::from(cek)),
                        };

                        // No need to check for dups, we only iterate over every attribute name once
                        structured_attrs.insert(arg_name.to_owned(), val_json);
                    } else {
                        match strong_importing_coerce_to_string(&co, value).await {
                            Err(cek) => return Ok(Value::from(cek)),
                            Ok(val_str) => {
                                input_context.mimic(&val_str);

                                insert_env(&mut drv, arg_name, val_str.as_bytes().into())?;
                            }
                        }
                    }
                }
            }
        }
        // end of per-argument loop

        // Configure fixed-output derivations if required.
        {
            async fn select_string(
                co: &GenCo,
                attrs: &NixAttrs,
                key: &str,
            ) -> Result<Result<Option<String>, CatchableErrorKind>, ErrorKind> {
                if let Some(attr) = attrs.select(key) {
                    match strong_importing_coerce_to_string(co, attr.clone()).await {
                        Err(cek) => return Ok(Err(cek)),
                        Ok(str) => return Ok(Ok(Some(str.to_str()?.to_owned()))),
                    }
                }

                Ok(Ok(None))
            }

            let output_hash = match select_string(&co, &input, "outputHash")
                .await
                .context("evaluating the `outputHash` parameter")?
            {
                Err(cek) => return Ok(Value::from(cek)),
                Ok(s) => s,
            };
            let output_hash_algo = match select_string(&co, &input, "outputHashAlgo")
                .await
                .context("evaluating the `outputHashAlgo` parameter")?
            {
                Err(cek) => return Ok(Value::from(cek)),
                Ok(s) => s,
            };
            let output_hash_mode = match select_string(&co, &input, "outputHashMode")
                .await
                .context("evaluating the `outputHashMode` parameter")?
            {
                Err(cek) => return Ok(Value::from(cek)),
                Ok(s) => s,
            };

            if let Some(warning) =
                handle_fixed_output(&mut drv, output_hash, output_hash_algo, output_hash_mode)?
            {
                emit_warning_kind(&co, warning).await;
            }
        }

        // Each output name needs to exist in the environment, at this
        // point initialised as an empty string, as the ATerm serialization of that is later
        // used for the output path calculation (which will also update output
        // paths post-calculation, both in drv.environment and drv.outputs)
        for output in drv.outputs.keys() {
            if drv
                .environment
                .insert(output.to_string(), String::new().into())
                .is_some()
            {
                emit_warning_kind(&co, WarningKind::ShadowedOutput(output.to_string())).await;
            }
        }

        if let Some(structured_attrs) = structured_attrs {
            // configure __json
            drv.environment.insert(
                "__json".to_string(),
                BString::from(serde_json::to_string(&structured_attrs)?),
            );
        }

        populate_inputs(&mut drv, input_context);
        let mut known_paths = state.as_ref().known_paths.borrow_mut();

        // At this point, derivation fields are fully populated from
        // eval data structures.
        drv.validate(false)
            .map_err(DerivationError::InvalidDerivation)?;

        // Calculate the derivation_or_fod_hash for the current derivation.
        // This one is still intermediate (so not added to known_paths)
        let derivation_or_fod_hash_tmp = drv.derivation_or_fod_hash(|drv_path| {
            known_paths
                .get_hash_derivation_modulo(&drv_path.to_owned())
                .unwrap_or_else(|| panic!("{} not found", drv_path))
                .to_owned()
        });

        // Mutate the Derivation struct and set output paths
        drv.calculate_output_paths(name, &derivation_or_fod_hash_tmp)
            .map_err(DerivationError::InvalidDerivation)?;

        let drv_path = drv
            .calculate_derivation_path(name)
            .map_err(DerivationError::InvalidDerivation)?;

        // TODO: avoid cloning
        known_paths.add(drv_path.clone(), drv.clone());

        let mut new_attrs: Vec<(String, NixString)> = drv
            .outputs
            .into_iter()
            .map(|(name, output)| {
                (
                    name.clone(),
                    (
                        output.path.unwrap().to_absolute_path(),
                        Some(
                            NixContextElement::Single {
                                name,
                                derivation: drv_path.to_absolute_path(),
                            }
                            .into(),
                        ),
                    )
                        .into(),
                )
            })
            .collect();

        new_attrs.push((
            "drvPath".to_string(),
            (
                drv_path.to_absolute_path(),
                Some(NixContextElement::Derivation(drv_path.to_absolute_path()).into()),
            )
                .into(),
        ));

        Ok(Value::Attrs(Box::new(NixAttrs::from_iter(
            new_attrs.into_iter(),
        ))))
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

        if content.iter_derivation().count() > 0 || content.iter_single_outputs().count() > 0 {
            return Err(ErrorKind::UnexpectedContext);
        }

        let path =
            nix_compat::store_path::build_text_path(name.to_str()?, &content, content.iter_plain())
                .map_err(|_e| {
                    nix_compat::derivation::DerivationError::InvalidOutputName(
                        name.to_str_lossy().into_owned(),
                    )
                })
                .map_err(DerivationError::InvalidDerivation)?
                .to_absolute_path();

        let context: NixContext = NixContextElement::Plain(path.clone()).into();

        // TODO: actually persist the file in the store at that path ...

        Ok(Value::from(NixString::new_context_from(context, path)))
    }
}
