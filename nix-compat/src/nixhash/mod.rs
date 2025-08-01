use crate::nixbase32;
use bstr::ByteSlice;
use data_encoding::{BASE64, BASE64_NOPAD, HEXLOWER};
use serde::Deserialize;
use serde::Serialize;
use std::cmp::Ordering;
use std::fmt::Display;
use thiserror;

mod algos;
mod ca_hash;

pub use algos::HashAlgo;
pub use ca_hash::CAHash;
pub use ca_hash::HashMode as CAHashMode;

/// NixHash represents hashes known by Nix.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum NixHash {
    Md5([u8; 16]),
    Sha1([u8; 20]),
    Sha256([u8; 32]),
    Sha512(Box<[u8; 64]>),
}

/// Same order as sorting the corresponding nixbase32 strings.
///
/// This order is used in the ATerm serialization of a derivation
/// and thus affects the calculated output hash.
impl Ord for NixHash {
    fn cmp(&self, other: &NixHash) -> Ordering {
        self.digest_as_bytes().cmp(other.digest_as_bytes())
    }
}

// See Ord for reason to implement this manually.
impl PartialOrd for NixHash {
    fn partial_cmp(&self, other: &NixHash) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Display for NixHash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::result::Result<(), std::fmt::Error> {
        write!(
            f,
            "{}-{}",
            self.algo(),
            BASE64.encode(self.digest_as_bytes())
        )
    }
}

/// convenience Result type for all nixhash parsing Results.
pub type NixHashResult<V> = std::result::Result<V, Error>;

impl NixHash {
    /// returns the algo as [HashAlgo].
    pub fn algo(&self) -> HashAlgo {
        match self {
            NixHash::Md5(_) => HashAlgo::Md5,
            NixHash::Sha1(_) => HashAlgo::Sha1,
            NixHash::Sha256(_) => HashAlgo::Sha256,
            NixHash::Sha512(_) => HashAlgo::Sha512,
        }
    }

    /// returns the digest as variable-length byte slice.
    pub fn digest_as_bytes(&self) -> &[u8] {
        match self {
            NixHash::Md5(digest) => digest,
            NixHash::Sha1(digest) => digest,
            NixHash::Sha256(digest) => digest,
            NixHash::Sha512(digest) => digest.as_ref(),
        }
    }

    /// Constructs a [NixHash] from the Nix default hash format,
    /// the inverse of [Self::to_nix_hex_string].
    pub fn from_nix_hex_str(s: &str) -> Option<Self> {
        let (tag, digest) = s.split_once(':')?;

        (match tag {
            "md5" => nixbase32::decode_fixed(digest).map(NixHash::Md5),
            "sha1" => nixbase32::decode_fixed(digest).map(NixHash::Sha1),
            "sha256" => nixbase32::decode_fixed(digest).map(NixHash::Sha256),
            "sha512" => nixbase32::decode_fixed(digest)
                .map(Box::new)
                .map(NixHash::Sha512),
            _ => return None,
        })
        .ok()
    }

    /// Formats a [NixHash] in the Nix default hash format,
    /// which is the algo, followed by a colon, then the lower hex encoded digest.
    pub fn to_nix_hex_string(&self) -> String {
        format!("{}:{}", self.algo(), self.to_plain_hex_string())
    }

    /// Formats a [NixHash] in the format that's used inside CAHash,
    /// which is the algo, followed by a colon, then the nixbase32-encoded digest.
    pub(crate) fn to_nix_nixbase32_string(&self) -> String {
        format!(
            "{}:{}",
            self.algo(),
            nixbase32::encode(self.digest_as_bytes())
        )
    }

    /// Returns the digest as a hex string -- without any algorithm prefix.
    pub fn to_plain_hex_string(&self) -> String {
        HEXLOWER.encode(self.digest_as_bytes())
    }
}

impl TryFrom<(HashAlgo, &[u8])> for NixHash {
    type Error = Error;

    /// Constructs a new [NixHash] by specifying [HashAlgo] and digest.
    /// It can fail if the passed digest length doesn't match what's expected for
    /// the passed algo.
    fn try_from(value: (HashAlgo, &[u8])) -> NixHashResult<Self> {
        let (algo, digest) = value;
        from_algo_and_digest(algo, digest)
    }
}

impl<'de> Deserialize<'de> for NixHash {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let str: &'de str = Deserialize::deserialize(deserializer)?;
        from_str(str, None).map_err(|_| {
            serde::de::Error::invalid_value(serde::de::Unexpected::Str(str), &"NixHash")
        })
    }
}

impl Serialize for NixHash {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        // encode as SRI
        let string = format!("{}-{}", self.algo(), BASE64.encode(self.digest_as_bytes()));
        string.serialize(serializer)
    }
}

/// Constructs a new [NixHash] by specifying [HashAlgo] and digest.
/// It can fail if the passed digest length doesn't match what's expected for
/// the passed algo.
pub fn from_algo_and_digest(algo: HashAlgo, digest: &[u8]) -> NixHashResult<NixHash> {
    if digest.len() != algo.digest_length() {
        return Err(Error::InvalidEncodedDigestLength(digest.len(), algo));
    }

    Ok(match algo {
        HashAlgo::Md5 => NixHash::Md5(digest.try_into().unwrap()),
        HashAlgo::Sha1 => NixHash::Sha1(digest.try_into().unwrap()),
        HashAlgo::Sha256 => NixHash::Sha256(digest.try_into().unwrap()),
        HashAlgo::Sha512 => NixHash::Sha512(Box::new(digest.try_into().unwrap())),
    })
}

/// Errors related to NixHash construction.
#[derive(Debug, Eq, PartialEq, thiserror::Error)]
pub enum Error {
    #[error("invalid hash algo: {0}")]
    InvalidAlgo(String),
    #[error("invalid SRI string: {0}")]
    InvalidSRI(String),
    #[error("invalid encoded digest length '{0}' for algo {1}")]
    InvalidEncodedDigestLength(usize, HashAlgo),
    #[error("invalid base16 encoding: {0}")]
    InvalidBase16Encoding(data_encoding::DecodeError),
    #[error("invalid base32 encoding: {0}")]
    InvalidBase32Encoding(data_encoding::DecodeError),
    #[error("invalid base64 encoding: {0}")]
    InvalidBase64Encoding(data_encoding::DecodeError),
    #[error("conflicting hash algo: {0} (hash_algo) vs {1} (inline)")]
    ConflictingHashAlgos(HashAlgo, HashAlgo),
    #[error("missing inline hash algo, but no externally-specified algo: {0}")]
    MissingInlineHashAlgo(String),
}

/// Nix allows specifying hashes in various encodings, and magically just
/// derives the encoding.
/// This function parses strings to a NixHash.
///
/// Hashes can be:
/// - Nix hash strings
/// - SRI hashes
/// - bare digests
///
/// Encoding for Nix hash strings or bare digests can be:
/// - base16 (lowerhex),
/// - nixbase32,
/// - base64 (StdEncoding)
/// - sri string
///
/// The encoding is derived from the length of the string and the hash type.
/// The hash is communicated out-of-band, but might also be in-band (in the
/// case of a nix hash string or SRI), in which it needs to be consistent with the
/// one communicated out-of-band.
pub fn from_str(s: &str, algo_str: Option<&str>) -> NixHashResult<NixHash> {
    // if algo_str is some, parse or bail out
    let algo: Option<HashAlgo> = if let Some(algo_str) = algo_str {
        Some(algo_str.try_into()?)
    } else {
        None
    };

    // Peek at the beginning of the string to detect SRI hashes.
    if s.starts_with("sha1-")
        || s.starts_with("sha256-")
        || s.starts_with("sha512-")
        || s.starts_with("md5-")
    {
        let parsed_nixhash = from_sri_str(s)?;

        // ensure the algo matches with what has been passed externally, if so.
        if let Some(algo) = algo {
            if algo != parsed_nixhash.algo() {
                return Err(Error::ConflictingHashAlgos(algo, parsed_nixhash.algo()));
            }
        }
        return Ok(parsed_nixhash);
    }

    // Peek at the beginning again to see if it's a Nix Hash
    if s.starts_with("sha1:")
        || s.starts_with("sha256:")
        || s.starts_with("sha512:")
        || s.starts_with("md5:")
    {
        let parsed_nixhash = from_nix_str(s)?;
        // ensure the algo matches with what has been passed externally, if so.
        if let Some(algo) = algo {
            if algo != parsed_nixhash.algo() {
                return Err(Error::ConflictingHashAlgos(algo, parsed_nixhash.algo()));
            }
        }
        return Ok(parsed_nixhash);
    }

    // Neither of these, assume a bare digest, so there MUST be an externally-passed algo.
    match algo {
        // Fail if there isn't.
        None => Err(Error::MissingInlineHashAlgo(s.to_string())),
        Some(algo) => decode_digest(s.as_bytes(), algo),
    }
}

/// Parses a Nix hash string ($algo:$digest) to a NixHash.
pub fn from_nix_str(s: &str) -> NixHashResult<NixHash> {
    if let Some(rest) = s.strip_prefix("sha1:") {
        decode_digest(rest.as_bytes(), HashAlgo::Sha1)
    } else if let Some(rest) = s.strip_prefix("sha256:") {
        decode_digest(rest.as_bytes(), HashAlgo::Sha256)
    } else if let Some(rest) = s.strip_prefix("sha512:") {
        decode_digest(rest.as_bytes(), HashAlgo::Sha512)
    } else if let Some(rest) = s.strip_prefix("md5:") {
        decode_digest(rest.as_bytes(), HashAlgo::Md5)
    } else {
        Err(Error::InvalidAlgo(s.to_string()))
    }
}

/// Parses a Nix SRI string to a NixHash.
/// Contrary to the SRI spec, Nix doesn't have an understanding of passing
/// multiple hashes (with different algos) in SRI hashes.
/// It instead simply cuts everything off after the expected length for the
/// specified algo, and tries to parse the rest in permissive base64 (allowing
/// missing padding).
pub fn from_sri_str(s: &str) -> NixHashResult<NixHash> {
    // split at the first occurence of "-"
    let (algo_str, digest_str) = s
        .split_once('-')
        .ok_or_else(|| Error::InvalidSRI(s.to_string()))?;

    // try to map the part before that `-` to a supported hash algo:
    let algo: HashAlgo = algo_str.try_into()?;

    // For the digest string, Nix ignores everything after the expected BASE64
    // (with padding) length, to account for the fact SRI allows specifying more
    // than one checksum, so shorten it.
    let digest_str = {
        let encoded_max_len = BASE64.encode_len(algo.digest_length());
        if digest_str.len() > encoded_max_len {
            &digest_str.as_bytes()[..encoded_max_len]
        } else {
            digest_str.as_bytes()
        }
    };

    // if the digest string is too small to fit even the BASE64_NOPAD version, bail out.
    if digest_str.len() < BASE64_NOPAD.encode_len(algo.digest_length()) {
        return Err(Error::InvalidEncodedDigestLength(digest_str.len(), algo));
    }

    // trim potential padding, and use a version that does not do trailing bit
    // checking.
    let mut spec = BASE64_NOPAD.specification();
    spec.check_trailing_bits = false;
    let encoding = spec
        .encoding()
        .expect("Tvix bug: failed to get the special base64 encoder for Nix SRI hashes");

    let digest = encoding
        .decode(digest_str.trim_end_with(|c| c == '='))
        .map_err(Error::InvalidBase64Encoding)?;

    from_algo_and_digest(algo, &digest)
}

/// Decode a plain digest depending on the hash algo specified externally.
/// hexlower, nixbase32 and base64 encodings are supported - the encoding is
/// inferred from the input length.
fn decode_digest(s: &[u8], algo: HashAlgo) -> NixHashResult<NixHash> {
    // for the chosen hash algo, calculate the expected (decoded) digest length
    // (as bytes)
    let digest = if s.len() == HEXLOWER.encode_len(algo.digest_length()) {
        HEXLOWER
            .decode(s.as_ref())
            .map_err(Error::InvalidBase16Encoding)?
    } else if s.len() == nixbase32::encode_len(algo.digest_length()) {
        nixbase32::decode(s).map_err(Error::InvalidBase32Encoding)?
    } else if s.len() == BASE64.encode_len(algo.digest_length()) {
        BASE64
            .decode(s.as_ref())
            .map_err(Error::InvalidBase64Encoding)?
    } else {
        Err(Error::InvalidEncodedDigestLength(s.len(), algo))?
    };

    Ok(from_algo_and_digest(algo, &digest).unwrap())
}

#[cfg(test)]
mod tests {
    use crate::{
        nixbase32,
        nixhash::{self, HashAlgo, NixHash},
    };
    use data_encoding::{BASE64, BASE64_NOPAD, HEXLOWER};
    use hex_literal::hex;
    use rstest::rstest;

    const DIGEST_SHA1: [u8; 20] = hex!("6016777997c30ab02413cf5095622cd7924283ac");
    const DIGEST_SHA256: [u8; 32] =
        hex!("a5ce9c155ed09397614646c9717fc7cd94b1023d7b76b618d409e4fefd6e9d39");
    const DIGEST_SHA512: [u8; 64] = hex!("ab40d0be3541f0774bba7815d13d10b03252e96e95f7dbb4ee99a3b431c21662fd6971a020160e39848aa5f305b9be0f78727b2b0789e39f124d21e92b8f39ef");
    const DIGEST_MD5: [u8; 16] = hex!("c4874a8897440b393d862d8fd459073f");

    fn to_base16(digest: &[u8]) -> String {
        HEXLOWER.encode(digest)
    }

    fn to_nixbase32(digest: &[u8]) -> String {
        nixbase32::encode(digest)
    }

    fn to_base64(digest: &[u8]) -> String {
        BASE64.encode(digest)
    }

    fn to_base64_nopad(digest: &[u8]) -> String {
        BASE64_NOPAD.encode(digest)
    }

    // TODO
    fn make_nixhash(algo: &HashAlgo, digest_encoded: String) -> String {
        format!("{algo}:{digest_encoded}")
    }
    fn make_sri_string(algo: &HashAlgo, digest_encoded: String) -> String {
        format!("{algo}-{digest_encoded}")
    }

    /// Test parsing a hash string in various formats, and also when/how the out-of-band algo is needed.
    #[rstest]
    #[case::sha1(&NixHash::Sha1(DIGEST_SHA1))]
    #[case::sha256(&NixHash::Sha256(DIGEST_SHA256))]
    #[case::sha512(&NixHash::Sha512(Box::new(DIGEST_SHA512)))]
    #[case::md5(&NixHash::Md5(DIGEST_MD5))]
    fn from_str(#[case] expected_hash: &NixHash) {
        let algo = &expected_hash.algo();
        let digest = expected_hash.digest_as_bytes();
        // parse SRI
        {
            // base64 without out-of-band algo
            let s = make_sri_string(algo, to_base64(digest));
            let h = nixhash::from_str(&s, None).expect("must succeed");
            assert_eq!(expected_hash, &h);

            // base64 with out-of-band-algo
            let s = make_sri_string(algo, to_base64(digest));
            let h = nixhash::from_str(&s, Some(&expected_hash.algo().to_string()))
                .expect("must succeed");
            assert_eq!(expected_hash, &h);

            // base64_nopad without out-of-band algo
            let s = make_sri_string(algo, to_base64_nopad(digest));
            let h = nixhash::from_str(&s, None).expect("must succeed");
            assert_eq!(expected_hash, &h);

            // base64_nopad with out-of-band-algo
            let s = make_sri_string(algo, to_base64_nopad(digest));
            let h = nixhash::from_str(&s, Some(&algo.to_string())).expect("must succeed");
            assert_eq!(expected_hash, &h);
        }

        // parse plain base16. should succeed with algo out-of-band, but fail without.
        {
            let s = to_base16(digest);
            nixhash::from_str(&s, None).expect_err("must fail");
            let h = nixhash::from_str(&s, Some(&algo.to_string())).expect("must succeed");
            assert_eq!(expected_hash, &h);
        }

        // parse plain nixbase32. should succeed with algo out-of-band, but fail without.
        {
            let s = to_nixbase32(digest);
            nixhash::from_str(&s, None).expect_err("must fail");
            let h = nixhash::from_str(&s, Some(&algo.to_string())).expect("must succeed");
            assert_eq!(expected_hash, &h);
        }

        // parse plain base64. should succeed with algo out-of-band, but fail without.
        {
            let s = to_base64(digest);
            nixhash::from_str(&s, None).expect_err("must fail");
            let h = nixhash::from_str(&s, Some(&algo.to_string())).expect("must succeed");
            assert_eq!(expected_hash, &h);
        }

        // parse Nix hash strings
        {
            // base16. should succeed with both algo out-of-band and in-band.
            {
                let s = make_nixhash(algo, to_base16(digest));
                assert_eq!(
                    expected_hash,
                    &nixhash::from_str(&s, None).expect("must succeed")
                );
                assert_eq!(
                    expected_hash,
                    &nixhash::from_str(&s, Some(&algo.to_string())).expect("must succeed")
                );
            }
            // nixbase32. should succeed with both algo out-of-band and in-band.
            {
                let s = make_nixhash(algo, to_nixbase32(digest));
                assert_eq!(
                    expected_hash,
                    &nixhash::from_str(&s, None).expect("must succeed")
                );
                assert_eq!(
                    expected_hash,
                    &nixhash::from_str(&s, Some(&algo.to_string())).expect("must succeed")
                );
            }
            // base64. should succeed with both algo out-of-band and in-band.
            {
                let s = make_nixhash(algo, to_base64(digest));
                assert_eq!(
                    expected_hash,
                    &nixhash::from_str(&s, None).expect("must succeed")
                );
                assert_eq!(
                    expected_hash,
                    &nixhash::from_str(&s, Some(&algo.to_string())).expect("must succeed")
                );
            }
        }
    }

    /// Test parsing an SRI hash via the [nixhash::from_sri_str] method.
    #[test]
    fn from_sri_str() {
        let nix_hash = nixhash::from_sri_str("sha256-pc6cFV7Qk5dhRkbJcX/HzZSxAj17drYY1Ank/v1unTk=")
            .expect("must succeed");

        assert_eq!(HashAlgo::Sha256, nix_hash.algo());
        assert_eq!(
            &hex!("a5ce9c155ed09397614646c9717fc7cd94b1023d7b76b618d409e4fefd6e9d39"),
            nix_hash.digest_as_bytes()
        )
    }

    /// Test parsing sha512 SRI hash with various paddings, Nix accepts all of them.
    #[rstest]
    #[case::no_padding("sha512-7g91TBvYoYQorRTqo+rYD/i5YnWvUBLnqDhPHxBJDaBW7smuPMeRp6E6JOFuVN9bzN0QnH1ToUU0u9c2CjALEQ")]
    #[case::too_little_padding("sha512-7g91TBvYoYQorRTqo+rYD/i5YnWvUBLnqDhPHxBJDaBW7smuPMeRp6E6JOFuVN9bzN0QnH1ToUU0u9c2CjALEQ=")]
    #[case::correct_padding("sha512-7g91TBvYoYQorRTqo+rYD/i5YnWvUBLnqDhPHxBJDaBW7smuPMeRp6E6JOFuVN9bzN0QnH1ToUU0u9c2CjALEQ==")]
    #[case::too_much_padding("sha512-7g91TBvYoYQorRTqo+rYD/i5YnWvUBLnqDhPHxBJDaBW7smuPMeRp6E6JOFuVN9bzN0QnH1ToUU0u9c2CjALEQ===")]
    #[case::additional_suffix_ignored("sha512-7g91TBvYoYQorRTqo+rYD/i5YnWvUBLnqDhPHxBJDaBW7smuPMeRp6E6JOFuVN9bzN0QnH1ToUU0u9c2CjALEQ== cheesecake")]
    fn from_sri_str_sha512_paddings(#[case] sri_str: &str) {
        let nix_hash = nixhash::from_sri_str(sri_str).expect("must succeed");

        assert_eq!(HashAlgo::Sha512, nix_hash.algo());
        assert_eq!(
            &hex!("ee0f754c1bd8a18428ad14eaa3ead80ff8b96275af5012e7a8384f1f10490da056eec9ae3cc791a7a13a24e16e54df5bccdd109c7d53a14534bbd7360a300b11"),
            nix_hash.digest_as_bytes()
        )
    }

    /// Ensure we detect truncated base64 digests, where the digest size
    /// doesn't match what's expected from that hash function.
    #[test]
    fn from_sri_str_truncated() {
        nixhash::from_sri_str("sha256-pc6cFV7Qk5dhRkbJcX/HzZSxAj17drYY1Ank")
            .expect_err("must fail");
    }

    /// Ensure we fail on SRI hashes that Nix doesn't support.
    #[test]
    fn from_sri_str_unsupported() {
        nixhash::from_sri_str(
            "sha384-o4UVSl89mIB0sFUK+3jQbG+C9Zc9dRlV/Xd3KAvXEbhqxu0J5OAdg6b6VHKHwQ7U",
        )
        .expect_err("must fail");
    }

    /// Ensure we reject invalid base64 encoding
    #[test]
    fn from_sri_str_invalid_base64() {
        nixhash::from_sri_str("sha256-invalid=base64").expect_err("must fail");
    }

    /// Nix also accepts SRI strings with missing padding, but only in case the
    /// string is expressed as SRI, so it still needs to have a `sha256-` prefix.
    ///
    /// This both seems to work if it is passed with and without specifying the
    /// hash algo out-of-band (hash = "sha256-…" or sha256 = "sha256-…")
    ///
    /// Passing the same broken base64 string, but not as SRI, while passing
    /// the hash algo out-of-band does not work.
    #[test]
    fn sha256_broken_padding() {
        let broken_base64 = "fgIr3TyFGDAXP5+qoAaiMKDg/a1MlT6Fv/S/DaA24S8";
        // if padded with a trailing '='
        let expected_digest =
            hex!("7e022bdd3c851830173f9faaa006a230a0e0fdad4c953e85bff4bf0da036e12f");

        // passing hash algo out of band should succeed
        let nix_hash = nixhash::from_str(&format!("sha256-{}", &broken_base64), Some("sha256"))
            .expect("must succeed");
        assert_eq!(&expected_digest, &nix_hash.digest_as_bytes());

        // not passing hash algo out of band should succeed
        let nix_hash =
            nixhash::from_str(&format!("sha256-{}", &broken_base64), None).expect("must succeed");
        assert_eq!(&expected_digest, &nix_hash.digest_as_bytes());

        // not passing SRI, but hash algo out of band should fail
        nixhash::from_str(broken_base64, Some("sha256")).expect_err("must fail");
    }

    /// As we decided to pass our hashes by trimming `=` completely,
    /// we need to take into account hashes with padding requirements which
    /// contains trailing bits which would be checked by `BASE64_NOPAD` and would
    /// make the verification crash.
    ///
    /// This base64 has a trailing non-zero bit at bit 42.
    #[test]
    fn sha256_weird_base64() {
        let weird_base64 = "syceJMUEknBDCHK8eGs6rUU3IQn+HnQfURfCrDxYPa9=";
        let expected_digest =
            hex!("b3271e24c5049270430872bc786b3aad45372109fe1e741f5117c2ac3c583daf");

        let nix_hash = nixhash::from_str(&format!("sha256-{}", &weird_base64), Some("sha256"))
            .expect("must succeed");
        assert_eq!(&expected_digest, &nix_hash.digest_as_bytes());

        // not passing hash algo out of band should succeed
        let nix_hash =
            nixhash::from_str(&format!("sha256-{}", &weird_base64), None).expect("must succeed");
        assert_eq!(&expected_digest, &nix_hash.digest_as_bytes());

        // not passing SRI, but hash algo out of band should fail
        nixhash::from_str(weird_base64, Some("sha256")).expect_err("must fail");
    }

    #[test]
    fn serialize_deserialize() {
        let nixhash_actual = NixHash::Sha256(hex!(
            "b3271e24c5049270430872bc786b3aad45372109fe1e741f5117c2ac3c583daf"
        ));
        let nixhash_str_json = "\"sha256-syceJMUEknBDCHK8eGs6rUU3IQn+HnQfURfCrDxYPa8=\"";

        let serialized = serde_json::to_string(&nixhash_actual).expect("can serialize");

        assert_eq!(nixhash_str_json, &serialized);

        let deserialized: NixHash =
            serde_json::from_str(nixhash_str_json).expect("must deserialize");
        assert_eq!(&nixhash_actual, &deserialized);
    }
}
