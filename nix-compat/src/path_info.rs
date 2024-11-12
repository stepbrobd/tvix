use crate::{narinfo::SignatureRef, nixbase32, nixhash::NixHash, store_path::StorePathRef};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// Represents information about a Store Path that Nix provides inside the build
/// if the exportReferencesGraph feature is used.
/// This is not to be confused with the format Nix uses in its `nix path-info` command.
/// It includes some more fields, like `registrationTime`, `signatures` and `ultimate`,
/// does not include the `closureSize` and encodes `narHash` as SRI.
#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct ExportedPathInfo<'a> {
    #[serde(rename = "closureSize")]
    pub closure_size: u64,

    #[serde(
        rename = "narHash",
        serialize_with = "to_nix_nixbase32_string",
        deserialize_with = "from_nix_hash_string"
    )]
    pub nar_sha256: [u8; 32],

    #[serde(rename = "narSize")]
    pub nar_size: u64,

    #[serde(borrow)]
    pub path: StorePathRef<'a>,

    #[serde(borrow)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deriver: Option<StorePathRef<'a>>,

    /// The list of other Store Paths this Store Path refers to.
    /// StorePathRef does Ord by the nixbase32-encoded string repr, so this is correct.
    pub references: BTreeSet<StorePathRef<'a>>,
    // more recent versions of Nix also have a `valid: true` field here, Nix 2.3 doesn't,
    // and nothing seems to use it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub signatures: Vec<SignatureRef<'a>>,
}

/// ExportedPathInfo are ordered by their `path` field.
impl Ord for ExportedPathInfo<'_> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.path.cmp(&other.path)
    }
}

impl PartialOrd for ExportedPathInfo<'_> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

fn to_nix_nixbase32_string<S>(v: &[u8; 32], serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    let string = NixHash::Sha256(*v).to_nix_nixbase32_string();
    string.serialize(serializer)
}

/// The length of a sha256 digest, nixbase32-encoded.
const NIXBASE32_SHA256_ENCODE_LEN: usize = nixbase32::encode_len(32);

fn from_nix_hash_string<'de, D>(deserializer: D) -> Result<[u8; 32], D::Error>
where
    D: serde::Deserializer<'de>,
{
    let str: &'de str = Deserialize::deserialize(deserializer)?;
    if let Some(digest_str) = str.strip_prefix("sha256:") {
        return from_nix_nixbase32_string::<D>(digest_str);
    }
    if let Some(digest_str) = str.strip_prefix("sha256-") {
        return from_sri_string::<D>(digest_str);
    }
    Err(serde::de::Error::invalid_value(
        serde::de::Unexpected::Str(str),
        &"extected a valid nixbase32 or sri narHash",
    ))
}

fn from_sri_string<'de, D>(str: &str) -> Result<[u8; 32], D::Error>
where
    D: serde::Deserializer<'de>,
{
    let digest: [u8; 32] = data_encoding::BASE64
        .decode(str.as_bytes())
        .map_err(|_| {
            serde::de::Error::invalid_value(
                serde::de::Unexpected::Str(str),
                &"valid base64 encoded string",
            )
        })?
        .try_into()
        .map_err(|_| {
            serde::de::Error::invalid_value(serde::de::Unexpected::Str(str), &"valid digest len")
        })?;

    Ok(digest)
}

fn from_nix_nixbase32_string<'de, D>(str: &str) -> Result<[u8; 32], D::Error>
where
    D: serde::Deserializer<'de>,
{
    let digest_str: [u8; NIXBASE32_SHA256_ENCODE_LEN] =
        str.as_bytes().try_into().map_err(|_| {
            serde::de::Error::invalid_value(serde::de::Unexpected::Str(str), &"valid digest len")
        })?;

    let digest: [u8; 32] = nixbase32::decode_fixed(digest_str).map_err(|_| {
        serde::de::Error::invalid_value(serde::de::Unexpected::Str(str), &"valid nixbase32")
    })?;

    Ok(digest)
}

#[cfg(test)]
mod tests {
    use hex_literal::hex;

    use super::*;

    /// Ensure we can create the same JSON as the exportReferencesGraph feature
    #[test]
    fn serialize_deserialize() {
        // JSON extracted from a build of
        // stdenv.mkDerivation { name = "hello"; __structuredAttrs = true; exportReferencesGraph.blub = [ pkgs.hello ]; nativeBuildInputs = [pkgs.jq]; buildCommand = "jq -rc .blub $NIX_ATTRS_JSON_FILE > $out"; }
        let pathinfos_str_json = r#"[{"closureSize":1828984,"narHash":"sha256:11vm2x1ajhzsrzw7lsyss51mmr3b6yll9wdjn51bh7liwkpc8ila","narSize":1828984,"path":"/nix/store/7n0mbqydcipkpbxm24fab066lxk68aqk-libunistring-1.1","references":["/nix/store/7n0mbqydcipkpbxm24fab066lxk68aqk-libunistring-1.1"]},{"closureSize":32696176,"narHash":"sha256:0alzbhjxdcsmr1pk7z0bdh46r2xpq3xs3k9y82bi4bx5pklcvw5x","narSize":226560,"path":"/nix/store/dbghhbq1x39yxgkv3vkgfwbxrmw9nfzi-hello-2.12.1","references":["/nix/store/dbghhbq1x39yxgkv3vkgfwbxrmw9nfzi-hello-2.12.1","/nix/store/ddwyrxif62r8n6xclvskjyy6szdhvj60-glibc-2.39-5"]},{"closureSize":32469616,"narHash":"sha256:1zw5p05fh0k836ybfxkskv8apcv2m3pm2wa6y90wqn5w5kjyj13c","narSize":30119936,"path":"/nix/store/ddwyrxif62r8n6xclvskjyy6szdhvj60-glibc-2.39-5","references":["/nix/store/ddwyrxif62r8n6xclvskjyy6szdhvj60-glibc-2.39-5","/nix/store/rxganm4ibf31qngal3j3psp20mak37yy-xgcc-13.2.0-libgcc","/nix/store/s32cldbh9pfzd9z82izi12mdlrw0yf8q-libidn2-2.3.7"]},{"closureSize":159560,"narHash":"sha256:10q8iyvfmpfck3yiisnj1j8vp6lq3km17r26sr95zpdf9mgmk69s","narSize":159560,"path":"/nix/store/rxganm4ibf31qngal3j3psp20mak37yy-xgcc-13.2.0-libgcc","references":[]},{"closureSize":2190120,"narHash":"sha256:1cv997nzxbd91jhmzwnhxa1ahlzp5ffli8m4a5npcq8zg0vb1kwg","narSize":361136,"path":"/nix/store/s32cldbh9pfzd9z82izi12mdlrw0yf8q-libidn2-2.3.7","references":["/nix/store/7n0mbqydcipkpbxm24fab066lxk68aqk-libunistring-1.1","/nix/store/s32cldbh9pfzd9z82izi12mdlrw0yf8q-libidn2-2.3.7"]}]"#;

        // We ensure it roundtrips (to check the sorting is correct)
        let deserialized: BTreeSet<ExportedPathInfo> =
            serde_json::from_str(pathinfos_str_json).expect("must serialize");

        let serialized_again = serde_json::to_string(&deserialized).expect("must deserialize");
        assert_eq!(pathinfos_str_json, serialized_again);

        // Also compare one specific item to be populated as expected.
        assert_eq!(
            &ExportedPathInfo {
                closure_size: 1828984,
                nar_sha256: hex!(
                    "8a46c4eee4911eb842b1b2f144a9376be45a43d1da6b7af8cffa43a942177587"
                ),
                nar_size: 1828984,
                path: StorePathRef::from_bytes(
                    b"7n0mbqydcipkpbxm24fab066lxk68aqk-libunistring-1.1"
                )
                .expect("must parse"),
                deriver: None,
                references: BTreeSet::from_iter([StorePathRef::from_bytes(
                    b"7n0mbqydcipkpbxm24fab066lxk68aqk-libunistring-1.1"
                )
                .unwrap()]),
                signatures: vec![],
            },
            deserialized.first().unwrap()
        );
    }

    /// Ensure we can parse output from `nix path-info --json``
    #[test]
    fn serialize_deserialize_from_path_info() {
        // JSON extracted from
        // nix path-info /nix/store/z6r3bn5l51679pwkvh9nalp6c317z34m-libcxx-16.0.6-dev --json --closure-size
        let pathinfos_str_json = r#"[{"closureSize":10756176,"deriver":"/nix/store/vs9976cyyxpykvdnlv7x85fpp3shn6ij-libcxx-16.0.6.drv","narHash":"sha256-E73Nt0NAKGxCnsyBFDUaCAbA+wiF5qjq1O9J7WrnT0E=","narSize":7020664,"path":"/nix/store/z6r3bn5l51679pwkvh9nalp6c317z34m-libcxx-16.0.6-dev","references":["/nix/store/lzzd5jgybnpfj86xkcpnd54xgwc4m457-libcxx-16.0.6"],"registrationTime":1730048276,"signatures":["cache.nixos.org-1:cTdhK6hnpPwtMXFX43CYb7v+CbpAusVI/MORZ3v5aHvpBYNg1MfBHVVeoexMBpNtHA8uFAn0aEsJaLXYIDhJDg=="],"valid":true}]"#;

        let deserialized: BTreeSet<ExportedPathInfo> =
            serde_json::from_str(pathinfos_str_json).expect("must serialize");

        assert_eq!(
            &ExportedPathInfo {
                closure_size: 10756176,
                nar_sha256: hex!(
                    "13bdcdb74340286c429ecc8114351a0806c0fb0885e6a8ead4ef49ed6ae74f41"
                ),
                nar_size: 7020664,
                path: StorePathRef::from_bytes(
                    b"z6r3bn5l51679pwkvh9nalp6c317z34m-libcxx-16.0.6-dev"
                )
                .expect("must parse"),
                deriver: Some(
                    StorePathRef::from_bytes(
                        b"vs9976cyyxpykvdnlv7x85fpp3shn6ij-libcxx-16.0.6.drv"
                    )
                    .expect("must parse")
                ),
                references: BTreeSet::from_iter([StorePathRef::from_bytes(
                    b"lzzd5jgybnpfj86xkcpnd54xgwc4m457-libcxx-16.0.6"
                )
                .unwrap()]),
                signatures: vec![SignatureRef::parse("cache.nixos.org-1:cTdhK6hnpPwtMXFX43CYb7v+CbpAusVI/MORZ3v5aHvpBYNg1MfBHVVeoexMBpNtHA8uFAn0aEsJaLXYIDhJDg==").expect("must parse")],
            },
            deserialized.first().unwrap()
        );
    }
}
