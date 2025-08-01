use crate::store_path::{
    self, build_ca_path, build_output_path, build_text_path, StorePath, StorePathRef,
};
use bstr::BString;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::io;

mod errors;
mod output;
mod parse_error;
mod parser;
mod validate;
mod write;

#[cfg(test)]
mod tests;

// Public API of the crate.
pub use crate::nixhash::{CAHash, NixHash};
pub use errors::{DerivationError, OutputError};
pub use output::Output;
pub use parser::Error as ParserError;

use self::write::AtermWriteable;

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct Derivation {
    #[serde(rename = "args")]
    pub arguments: Vec<String>,

    pub builder: String,

    #[serde(rename = "env")]
    pub environment: BTreeMap<String, BString>,

    /// Map from drv path to output names used from this derivation.
    #[serde(rename = "inputDrvs")]
    pub input_derivations: BTreeMap<StorePath<String>, BTreeSet<String>>,

    /// Plain store paths of additional inputs.
    #[serde(rename = "inputSrcs")]
    pub input_sources: BTreeSet<StorePath<String>>,

    /// Maps output names to Output.
    pub outputs: BTreeMap<String, Output>,

    pub system: String,
}

impl Derivation {
    /// write the Derivation to the given [std::io::Write], in ATerm format.
    ///
    /// The only errors returns are these when writing to the passed writer.
    pub fn serialize(&self, writer: &mut impl std::io::Write) -> Result<(), io::Error> {
        self.serialize_with_replacements(writer, &self.input_derivations)
    }

    /// Like `serialize` but allow replacing the input_derivations for hash calculations.
    fn serialize_with_replacements(
        &self,
        writer: &mut impl std::io::Write,
        input_derivations: &BTreeMap<impl AtermWriteable, BTreeSet<String>>,
    ) -> Result<(), io::Error> {
        use write::*;

        writer.write_all(write::DERIVATION_PREFIX.as_bytes())?;
        write_char(writer, write::PAREN_OPEN)?;

        write_outputs(writer, &self.outputs)?;
        write_char(writer, COMMA)?;

        write_input_derivations(writer, input_derivations)?;
        write_char(writer, COMMA)?;

        write_input_sources(writer, &self.input_sources)?;
        write_char(writer, COMMA)?;

        write_system(writer, &self.system)?;
        write_char(writer, COMMA)?;

        write_builder(writer, &self.builder)?;
        write_char(writer, COMMA)?;

        write_arguments(writer, &self.arguments)?;
        write_char(writer, COMMA)?;

        write_environment(writer, &self.environment)?;

        write_char(writer, PAREN_CLOSE)?;

        Ok(())
    }

    /// return the ATerm serialization.
    pub fn to_aterm_bytes(&self) -> Vec<u8> {
        self.to_aterm_bytes_with_replacements(&self.input_derivations)
    }

    /// Like `to_aterm_bytes`, but accept a different BTreeMap for input_derivations.
    /// This is used to render the ATerm representation of a Derivation "modulo
    /// fixed-output derivations".
    fn to_aterm_bytes_with_replacements(
        &self,
        input_derivations: &BTreeMap<impl AtermWriteable, BTreeSet<String>>,
    ) -> Vec<u8> {
        let mut buffer: Vec<u8> = Vec::new();

        // invoke serialize and write to the buffer.
        // Note we only propagate errors writing to the writer in serialize,
        // which won't panic for the string we write to.
        self.serialize_with_replacements(&mut buffer, input_derivations)
            .unwrap();

        buffer
    }

    /// Parse an Derivation in ATerm serialization, and validate it passes our
    /// set of validations.
    pub fn from_aterm_bytes(b: &[u8]) -> Result<Derivation, parser::Error<&[u8]>> {
        parser::parse(b)
    }

    /// Returns the drv path of a [Derivation] struct.
    ///
    /// The drv path is calculated by invoking [build_text_path], using
    /// the `name` with a `.drv` suffix as name, all [Derivation::input_sources] and
    /// keys of [Derivation::input_derivations] as references, and the ATerm string of
    /// the [Derivation] as content.
    pub fn calculate_derivation_path(
        &self,
        name: &str,
    ) -> Result<StorePath<String>, DerivationError> {
        // append .drv to the name
        let name = &format!("{name}.drv");

        // collect the list of paths from input_sources and input_derivations
        // into a (sorted, guaranteed by BTreeSet) list of references
        let references: BTreeSet<String> = self
            .input_sources
            .iter()
            .chain(self.input_derivations.keys())
            .map(StorePath::to_absolute_path)
            .collect();

        build_text_path(name, self.to_aterm_bytes(), references)
            .map_err(|_e| DerivationError::InvalidOutputName(name.to_string()))
    }

    /// Returns the FOD digest, if the derivation is fixed-output, or None if
    /// it's not.
    /// TODO: this is kinda the string from [build_ca_path] with a
    /// [CAHash::Flat], what's fed to `build_store_path_from_fingerprint_parts`
    /// (except the out_output.path being an empty string)
    pub fn fod_digest(&self) -> Option<[u8; 32]> {
        if self.outputs.len() != 1 {
            return None;
        }

        let out_output = self.outputs.get("out")?;
        let ca_hash = &out_output.ca_hash.as_ref()?;

        Some(
            Sha256::new_with_prefix(format!(
                "fixed:out:{}{}:{}",
                ca_kind_prefix(ca_hash),
                ca_hash.hash().to_nix_hex_string(),
                out_output
                    .path
                    .as_ref()
                    .map(StorePath::to_absolute_path)
                    .as_ref()
                    .map(|s| s as &str)
                    .unwrap_or(""),
            ))
            .finalize()
            .into(),
        )
    }

    /// Calculates the hash of a derivation modulo fixed-output subderivations.
    ///
    /// This is called `hashDerivationModulo` in nixcpp.
    ///
    /// It returns the sha256 digest of the derivation ATerm representation,
    /// except that:
    ///  -  any input derivation paths have beed replaced "by the result of a
    ///     recursive call to this function" and that
    ///  - for fixed-output derivations the special
    ///    `fixed:out:${algo}:${digest}:${fodPath}` string is hashed instead of
    ///    the A-Term.
    ///
    /// It's up to the caller of this function to provide a (infallible) lookup
    /// function to query the [Derivation::hash_derivation_modulo] of direct
    /// input derivations, by their [StorePathRef].
    /// It will only be called in case the derivation is not a fixed-output
    /// derivation.
    pub fn hash_derivation_modulo<F>(&self, fn_lookup_hash_derivation_modulo: F) -> [u8; 32]
    where
        F: Fn(&StorePathRef) -> [u8; 32],
    {
        // Fixed-output derivations return a fixed hash.
        // Non-Fixed-output derivations return the sha256 digest of the ATerm
        // notation, but with all input_derivation paths replaced by a recursive
        // call to this function.
        // We call [fn_lookup_hash_derivation_modulo] rather than recursing
        // ourselves, so callers can precompute this.
        self.fod_digest().unwrap_or({
            // For each input_derivation, look up the hash derivation modulo,
            // and replace the derivation path in the aterm with it's HEXLOWER digest.
            let aterm_bytes = self.to_aterm_bytes_with_replacements(&BTreeMap::from_iter(
                self.input_derivations
                    .iter()
                    .map(|(drv_path, output_names)| {
                        let hash = fn_lookup_hash_derivation_modulo(&drv_path.as_ref());

                        (hash, output_names.to_owned())
                    }),
            ));

            // write the ATerm of that to the hash function and return its digest.
            Sha256::new_with_prefix(aterm_bytes).finalize().into()
        })
    }

    /// This calculates all output paths of a Derivation and updates the struct.
    /// It requires the struct to be initially without output paths.
    /// This means, self.outputs[$outputName].path needs to be an empty string,
    /// and self.environment[$outputName] needs to be an empty string.
    ///
    /// Output path calculation requires knowledge of the
    /// [Derivation::hash_derivation_modulo], which (in case of non-fixed-output
    /// derivations) also requires knowledge of the
    /// [Derivation::hash_derivation_modulo] of input derivations (recursively).
    ///
    /// To avoid recursing and doing unnecessary calculation, we simply
    /// ask the caller of this function to provide the result of the
    /// [Derivation::hash_derivation_modulo] call of the current [Derivation],
    /// and leave it up to them to calculate it when needed.
    ///
    /// On completion, `self.environment[$outputName]` and
    /// `self.outputs[$outputName].path` are set to the calculated output path for all
    /// outputs.
    pub fn calculate_output_paths(
        &mut self,
        name: &str,
        hash_derivation_modulo: &[u8; 32],
    ) -> Result<(), DerivationError> {
        // The fingerprint and hash differs per output
        for (output_name, output) in self.outputs.iter_mut() {
            // Assert that outputs are not yet populated, to avoid using this function wrongly.
            // We don't also go over self.environment, but it's a sufficient
            // footgun prevention mechanism.
            assert!(output.path.is_none());

            let path_name = output_path_name(name, output_name);

            // For fixed output derivation we use [build_ca_path], otherwise we
            // use [build_output_path] with [hash_derivation_modulo].
            let store_path = if let Some(ref hwm) = output.ca_hash {
                build_ca_path(&path_name, hwm, Vec::<&str>::new(), false).map_err(|e| {
                    DerivationError::InvalidOutputDerivationPath(output_name.to_string(), e)
                })?
            } else {
                build_output_path(hash_derivation_modulo, output_name, &path_name).map_err(|e| {
                    DerivationError::InvalidOutputDerivationPath(
                        output_name.to_string(),
                        store_path::BuildStorePathError::InvalidStorePath(e),
                    )
                })?
            };

            self.environment.insert(
                output_name.to_string(),
                store_path.to_absolute_path().into(),
            );
            output.path = Some(store_path);
        }

        Ok(())
    }
}

/// Calculate the name part of the store path of a derivation [Output].
///
/// It's the name, and (if it's the non-out output), the output name
/// after a `-`.
fn output_path_name(derivation_name: &str, output_name: &str) -> String {
    let mut output_path_name = derivation_name.to_string();
    if output_name != "out" {
        output_path_name.push('-');
        output_path_name.push_str(output_name);
    }
    output_path_name
}

/// For a [CAHash], return the "prefix" used for NAR purposes.
/// For [CAHash::Flat], this is an empty string, for [CAHash::Nar], it's "r:".
/// Panics for other [CAHash] kinds, as they're not valid in a derivation
/// context.
fn ca_kind_prefix(ca_hash: &CAHash) -> &'static str {
    match ca_hash {
        CAHash::Flat(_) => "",
        CAHash::Nar(_) => "r:",
        _ => panic!("invalid ca hash in derivation context: {ca_hash:?}"),
    }
}
