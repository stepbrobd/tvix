//! This module implements logic required for persisting known paths
//! during an evaluation.
//!
//! Tvix needs to be able to keep track of each Nix store path that it
//! knows about during the scope of a single evaluation and its
//! related builds.
//!
//! This data is required to find the derivation needed to actually trigger the
//! build, if necessary.

use nix_compat::{derivation::Derivation, store_path::StorePath};
use std::collections::HashMap;

// use crate::fetchers::Fetch;

/// Struct keeping track of all known Derivations in the current evaluation.
/// This keeps both the Derivation struct, as well as the "Hash derivation
/// modulo".
#[derive(Debug, Default)]
pub struct KnownPaths {
    /// All known derivation or FOD hashes.
    ///
    /// Keys are derivation paths, values are a tuple of the "hash derivation
    /// modulo" and the Derivation struct itself.
    derivations: HashMap<StorePath<String>, ([u8; 32], Derivation)>,

    /// A map from output path to (one) drv path.
    /// Note that in the case of FODs, multiple drvs can produce the same output
    /// path. We use one of them.
    outputs_to_drvpath: HashMap<StorePath<String>, StorePath<String>>,
    /*
    /// A map from output path to fetches (and their names).
    outputs_to_fetches: HashMap<StorePath<String>, (String, Fetch)>,
    */
}

impl KnownPaths {
    /// Fetch the opaque "hash derivation modulo" for a given derivation path.
    pub fn get_hash_derivation_modulo(&self, drv_path: &StorePath<String>) -> Option<&[u8; 32]> {
        self.derivations
            .get(drv_path)
            .map(|(hash_derivation_modulo, _derivation)| hash_derivation_modulo)
    }

    /// Return a reference to the Derivation for a given drv path.
    pub fn get_drv_by_drvpath(&self, drv_path: &StorePath<String>) -> Option<&Derivation> {
        self.derivations
            .get(drv_path)
            .map(|(_hash_derivation_modulo, derivation)| derivation)
    }

    /// Return the drv path of the derivation producing the passed output path.
    /// Note there can be multiple Derivations producing the same output path in
    /// flight; this function will only return one of them.
    pub fn get_drv_path_for_output_path(
        &self,
        output_path: &StorePath<String>,
    ) -> Option<&StorePath<String>> {
        self.outputs_to_drvpath.get(output_path)
    }

    /// Insert a new [Derivation] into this struct.
    /// The Derivation struct must pass validation, and its output paths need to
    /// be fully calculated.
    /// All input derivations this refers to must also be inserted to this
    /// struct.
    pub fn add_derivation(&mut self, drv_path: StorePath<String>, drv: Derivation) {
        // check input derivations to have been inserted.
        #[cfg(debug_assertions)]
        {
            for input_drv_path in drv.input_derivations.keys() {
                debug_assert!(self.derivations.contains_key(input_drv_path));
            }
        }

        // compute the hash derivation modulo
        let hash_derivation_modulo = drv.hash_derivation_modulo(|drv_path| {
            self.get_hash_derivation_modulo(&drv_path.to_owned())
                .unwrap_or_else(|| panic!("{drv_path} not found"))
                .to_owned()
        });

        // For all output paths, update our lookup table.
        // We only write into the lookup table once.
        for output in drv.outputs.values() {
            self.outputs_to_drvpath
                .entry(output.path.as_ref().expect("missing store path").clone())
                .or_insert(drv_path.to_owned());
        }

        // insert the derivation itself
        #[allow(unused_variables)] // assertions on this only compiled in debug builds
        let old = self
            .derivations
            .insert(drv_path.to_owned(), (hash_derivation_modulo, drv));

        #[cfg(debug_assertions)]
        {
            if let Some(old) = old {
                debug_assert!(
                    old.0 == hash_derivation_modulo,
                    "hash derivation modulo for a given derivation should always be calculated the same"
                );
            }
        }
    }

    /*
    /// Insert a new [Fetch] into this struct, which *must* have an expected
    /// hash (otherwise we wouldn't be able to calculate the store path).
    /// Fetches without a known hash need to be fetched inside builtins.
    pub fn add_fetch<'a>(
        &mut self,
        fetch: Fetch,
        name: &'a str,
    ) -> Result<StorePathRef<'a>, BuildStorePathError> {
        let store_path = fetch
            .store_path(name)?
            .expect("Tvix bug: fetch must have an expected hash");
        // insert the fetch.
        self.outputs_to_fetches
            .insert(store_path.to_owned(), (name.to_owned(), fetch));

        Ok(store_path)
    }

    /// Return the name and fetch producing the passed output path.
    /// Note there can also be (multiple) Derivations producing the same output path.
    pub fn get_fetch_for_output_path(
        &self,
        output_path: &StorePath<String>,
    ) -> Option<(String, Fetch)> {
        self.outputs_to_fetches
            .get(output_path)
            .map(|(name, fetch)| (name.to_owned(), fetch.to_owned()))
    }
    */

    /// Returns an iterator over all known derivations and their store path.
    pub fn get_derivations(&self) -> impl Iterator<Item = (&StorePath<String>, &Derivation)> {
        self.derivations.iter().map(|(k, v)| (k, &v.1))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::LazyLock;

    use super::KnownPaths;
    use hex_literal::hex;
    use nix_compat::{derivation::Derivation, store_path::StorePath};

    // use url::Url;
    // use crate::fetchers::Fetch;

    static BAR_DRV: LazyLock<Derivation> = LazyLock::new(|| {
        Derivation::from_aterm_bytes(include_bytes!(
            "tests/ss2p4wmxijn652haqyd7dckxwl4c7hxx-bar.drv"
        ))
        .expect("must parse")
    });

    static FOO_DRV: LazyLock<Derivation> = LazyLock::new(|| {
        Derivation::from_aterm_bytes(include_bytes!(
            "tests/ch49594n9avinrf8ip0aslidkc4lxkqv-foo.drv"
        ))
        .expect("must parse")
    });

    static BAR_DRV_PATH: LazyLock<StorePath<String>> = LazyLock::new(|| {
        StorePath::from_bytes(b"ss2p4wmxijn652haqyd7dckxwl4c7hxx-bar.drv").expect("must parse")
    });

    static FOO_DRV_PATH: LazyLock<StorePath<String>> = LazyLock::new(|| {
        StorePath::from_bytes(b"ch49594n9avinrf8ip0aslidkc4lxkqv-foo.drv").expect("must parse")
    });

    static BAR_OUT_PATH: LazyLock<StorePath<String>> = LazyLock::new(|| {
        StorePath::from_bytes(b"mp57d33657rf34lzvlbpfa1gjfv5gmpg-bar").expect("must parse")
    });

    static FOO_OUT_PATH: LazyLock<StorePath<String>> = LazyLock::new(|| {
        StorePath::from_bytes(b"fhaj6gmwns62s6ypkcldbaj2ybvkhx3p-foo").expect("must parse")
    });

    /*
    static FETCH_URL: LazyLock<Fetch> = LazyLock::new(|| {
        Fetch::URL {
        url: Url::parse("https://raw.githubusercontent.com/aaptel/notmuch-extract-patch/f732a53e12a7c91a06755ebfab2007adc9b3063b/notmuch-extract-patch").unwrap(),
        exp_hash: Some(nixhash::from_sri_str("sha256-Xa1Jbl2Eq5+L0ww+Ph1osA3Z/Dxe/RkN1/dITQCdXFk=").unwrap())
    }
    });

    static FETCH_URL_OUT_PATH: LazyLock<StorePath<String>> = LazyLock::new(|| {
        StorePath::from_bytes(b"06qi00hylriyfm0nl827crgjvbax84mz-notmuch-extract-patch").unwrap()
    });

    static FETCH_TARBALL: LazyLock<Fetch> = LazyLock::new(|| {
        Fetch::Tarball {
        url: Url::parse("https://github.com/NixOS/nixpkgs/archive/91050ea1e57e50388fa87a3302ba12d188ef723a.tar.gz").unwrap(),
        exp_nar_sha256: Some(nixbase32::decode_fixed("1hf6cgaci1n186kkkjq106ryf8mmlq9vnwgfwh625wa8hfgdn4dm").unwrap())
    }
    });

    static FETCH_TARBALL_OUT_PATH: LazyLock<StorePath<String>> = LazyLock::new(|| {
        StorePath::from_bytes(b"7adgvk5zdfq4pwrhsm3n9lzypb12gw0g-source").unwrap()
    });
    */

    /// Ensure that we don't allow adding a derivation that depends on another,
    /// not-yet-added derivation.
    #[test]
    #[should_panic]
    fn drv_reject_if_missing_input_drv() {
        let mut known_paths = KnownPaths::default();

        // FOO_DRV depends on BAR_DRV, which wasn't added.
        known_paths.add_derivation(FOO_DRV_PATH.clone(), FOO_DRV.clone());
    }

    #[test]
    fn drv_happy_path() {
        let mut known_paths = KnownPaths::default();

        // get_drv_by_drvpath should return None for non-existing Derivations,
        // same as get_hash_derivation_modulo and get_drv_path_for_output_path
        assert_eq!(None, known_paths.get_drv_by_drvpath(&BAR_DRV_PATH));
        assert_eq!(None, known_paths.get_hash_derivation_modulo(&BAR_DRV_PATH));
        assert_eq!(
            None,
            known_paths.get_drv_path_for_output_path(&BAR_OUT_PATH)
        );

        // Add BAR_DRV
        known_paths.add_derivation(BAR_DRV_PATH.clone(), BAR_DRV.clone());

        // We should get it back
        assert_eq!(
            Some(&BAR_DRV.clone()),
            known_paths.get_drv_by_drvpath(&BAR_DRV_PATH)
        );

        // Test get_drv_path_for_output_path
        assert_eq!(
            Some(&BAR_DRV_PATH.clone()),
            known_paths.get_drv_path_for_output_path(&BAR_OUT_PATH)
        );

        // It should be possible to get the hash derivation modulo.
        assert_eq!(
            Some(&hex!(
                "c79aebd0ce3269393d4a1fde2cbd1d975d879b40f0bf40a48f550edc107fd5df"
            )),
            known_paths.get_hash_derivation_modulo(&BAR_DRV_PATH.clone())
        );

        // Now insert FOO_DRV too. It shouldn't panic, as BAR_DRV is already
        // added.
        known_paths.add_derivation(FOO_DRV_PATH.clone(), FOO_DRV.clone());

        assert_eq!(
            Some(&FOO_DRV.clone()),
            known_paths.get_drv_by_drvpath(&FOO_DRV_PATH)
        );
        assert_eq!(
            Some(&hex!(
                "af030d36d63d3d7f56a71adaba26b36f5fa1f9847da5eed953ed62e18192762f"
            )),
            known_paths.get_hash_derivation_modulo(&FOO_DRV_PATH.clone())
        );

        // Test get_drv_path_for_output_path
        assert_eq!(
            Some(&FOO_DRV_PATH.clone()),
            known_paths.get_drv_path_for_output_path(&FOO_OUT_PATH)
        );
    }

    /*
    #[test]
    fn fetch_happy_path() {
        let mut known_paths = KnownPaths::default();

        // get_fetch_for_output_path should return None for new fetches.
        assert!(known_paths
            .get_fetch_for_output_path(&FETCH_TARBALL_OUT_PATH)
            .is_none());

        // add_fetch should return the properly calculated store paths.
        assert_eq!(
            *FETCH_TARBALL_OUT_PATH,
            known_paths
                .add_fetch(FETCH_TARBALL.clone(), "source")
                .unwrap()
                .to_owned()
        );

        assert_eq!(
            *FETCH_URL_OUT_PATH,
            known_paths
                .add_fetch(FETCH_URL.clone(), "notmuch-extract-patch")
                .unwrap()
                .to_owned()
        );
    }
    */

    #[test]
    fn get_derivations_working() {
        let mut known_paths = KnownPaths::default();

        // Add BAR_DRV
        known_paths.add_derivation(BAR_DRV_PATH.clone(), BAR_DRV.clone());

        // We should be able to find BAR_DRV_PATH and BAR_DRV as a pair in get_derivations.
        assert_eq!(
            Some((&BAR_DRV_PATH.clone(), &BAR_DRV.clone())),
            known_paths
                .get_derivations()
                .find(|(s, d)| (*s, *d) == (&BAR_DRV_PATH, &BAR_DRV))
        );
    }

    // TODO: add test panicking about missing digest
}
