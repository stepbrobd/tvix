//! NAR info files describe a store path in a traditional Nix binary cache.
//! Over the wire, they are formatted as "Key: value" pairs separated by newlines.
//!
//! It contains four kinds of information:
//! 1. the description of the store path itself
//!    * store path prefix, digest, and name
//!    * NAR hash and size
//!    * references
//! 2. authenticity information
//!    * zero or more signatures over that description
//!    * an optional [CAHash] for content-addressed paths (fixed outputs, sources, and derivations)
//! 3. derivation metadata
//!    * deriver (the derivation that produced this path)
//!    * system (the system value of that derivation)
//! 4. cache-specific information
//!    * URL of the compressed NAR, relative to the NAR info file
//!    * compression algorithm used for the NAR
//!    * hash and size of the compressed NAR

use bitflags::bitflags;
use data_encoding::HEXLOWER;
use std::{
    fmt::{self, Display},
    mem,
};

use crate::{nixbase32, nixhash::CAHash, store_path::StorePathRef};

mod fingerprint;
mod signature;
mod signing_keys;
mod verifying_keys;

pub use fingerprint::fingerprint;
pub use signature::{Error as SignatureError, Signature, SignatureRef};
pub use signing_keys::parse_keypair;
pub use signing_keys::{Error as SigningKeyError, SigningKey};
pub use verifying_keys::{Error as VerifyingKeyError, VerifyingKey};

#[derive(Debug)]
pub struct NarInfo<'a> {
    pub flags: Flags,
    // core (authenticated, but unverified here)
    /// Store path described by this [NarInfo]
    pub store_path: StorePathRef<'a>,
    /// SHA-256 digest of the NAR file
    pub nar_hash: [u8; 32],
    /// Size of the NAR file in bytes
    pub nar_size: u64,
    /// Store paths known to be referenced by the contents
    pub references: Vec<StorePathRef<'a>>,
    // authenticity
    /// Ed25519 signature over the path fingerprint
    pub signatures: Vec<SignatureRef<'a>>,
    /// Content address (for content-defined paths)
    pub ca: Option<CAHash>,
    // derivation metadata
    /// Nix system triple of [NarInfo::deriver]
    pub system: Option<&'a str>,
    /// Store path of the derivation that produced this. The last .drv suffix is stripped.
    pub deriver: Option<StorePathRef<'a>>,
    // cache-specific untrusted metadata
    /// Relative URL of the compressed NAR file
    pub url: &'a str,
    /// Compression method of the NAR file
    /// `None` means `Compression: none`.
    ///
    /// Nix interprets a missing `Compression` field as `Some("bzip2")`,
    /// so we do as well. We haven't found any examples of this in the
    /// wild, not even in the cache.nixos.org dataset.
    pub compression: Option<&'a str>,
    /// SHA-256 digest of the file at `url`
    pub file_hash: Option<[u8; 32]>,
    /// Size of the file at `url` in bytes
    pub file_size: Option<u64>,
}

bitflags! {
    /// TODO(edef): be conscious of these when roundtripping
    #[derive(Debug, Copy, Clone)]
    pub struct Flags: u8 {
        const UNKNOWN_FIELD = 1 << 0;
        const COMPRESSION_DEFAULT = 1 << 1;
        // Format quirks encountered in the cache.nixos.org dataset
        const REFERENCES_OUT_OF_ORDER = 1 << 2;
        const NAR_HASH_HEX = 1 << 3;

        /// Deriver: unknown-deriver, produced by a legacy tool
        ///
        /// Only relevant if [NarInfo::deriver] is [None],
        /// but valid to have set either way.
        const EXPLICIT_UNKNOWN_DERIVER = 1 << 4;

        /// entirely missing References field, produced by harmonia
        const REFERENCES_MISSING = 1 << 5;
    }
}

const TAG_STOREPATH: &str = "StorePath";
const TAG_URL: &str = "URL";
const TAG_COMPRESSION: &str = "Compression";
const TAG_FILEHASH: &str = "FileHash";
const TAG_FILESIZE: &str = "FileSize";
const TAG_NARHASH: &str = "NarHash";
const TAG_NARSIZE: &str = "NarSize";
const TAG_REFERENCES: &str = "References";
const TAG_SYSTEM: &str = "System";
const TAG_DERIVER: &str = "Deriver";
const TAG_SIG: &str = "Sig";
const TAG_CA: &str = "CA";

impl<'a> NarInfo<'a> {
    pub fn parse(input: &'a str) -> Result<Self, Error> {
        let mut flags = Flags::empty();
        let mut store_path = None;
        let mut url = None;
        let mut compression = None;
        let mut file_hash = None;
        let mut file_size = None;
        let mut nar_hash = None;
        let mut nar_size = None;
        let mut references = None;
        let mut system = None;
        let mut deriver = None;
        let mut signatures = vec![];
        let mut ca = None;

        for line in input.lines() {
            let (tag, val) = line
                .split_once(':')
                .ok_or_else(|| Error::InvalidLine(line.to_string()))?;

            let val = val
                .strip_prefix(' ')
                .ok_or_else(|| Error::InvalidLine(line.to_string()))?;

            match tag {
                TAG_STOREPATH => {
                    let val = val
                        .strip_prefix("/nix/store/")
                        .ok_or(Error::InvalidStorePath(
                            crate::store_path::Error::MissingStoreDir,
                        ))?;
                    let val = StorePathRef::from_bytes(val.as_bytes())
                        .map_err(Error::InvalidStorePath)?;

                    if store_path.replace(val).is_some() {
                        return Err(Error::DuplicateField(TAG_STOREPATH));
                    }
                }
                TAG_URL => {
                    if val.is_empty() {
                        return Err(Error::EmptyField(TAG_URL));
                    }

                    if url.replace(val).is_some() {
                        return Err(Error::DuplicateField(TAG_URL));
                    }
                }
                TAG_COMPRESSION => {
                    if val.is_empty() {
                        return Err(Error::EmptyField(TAG_COMPRESSION));
                    }

                    if compression.replace(val).is_some() {
                        return Err(Error::DuplicateField(TAG_COMPRESSION));
                    }
                }
                TAG_FILEHASH => {
                    let val = val
                        .strip_prefix("sha256:")
                        .ok_or(Error::MissingPrefixForHash(TAG_FILEHASH))?;
                    let val = nixbase32::decode_fixed::<32>(val)
                        .map_err(|e| Error::UnableToDecodeHash(TAG_FILEHASH, e))?;

                    if file_hash.replace(val).is_some() {
                        return Err(Error::DuplicateField(TAG_FILEHASH));
                    }
                }
                TAG_FILESIZE => {
                    let val = val
                        .parse::<u64>()
                        .map_err(|_| Error::UnableToParseSize(TAG_FILESIZE, val.to_string()))?;

                    if file_size.replace(val).is_some() {
                        return Err(Error::DuplicateField(TAG_FILESIZE));
                    }
                }
                TAG_NARHASH => {
                    let val = val
                        .strip_prefix("sha256:")
                        .ok_or(Error::MissingPrefixForHash(TAG_NARHASH))?;

                    let val = if val.len() != HEXLOWER.encode_len(32) {
                        nixbase32::decode_fixed::<32>(val)
                    } else {
                        flags |= Flags::NAR_HASH_HEX;

                        let val = val.as_bytes();
                        let mut buf = [0u8; 32];

                        HEXLOWER
                            .decode_mut(val, &mut buf)
                            .map_err(|e| e.error)
                            .map(|_| buf)
                    };

                    let val = val.map_err(|e| Error::UnableToDecodeHash(TAG_NARHASH, e))?;

                    if nar_hash.replace(val).is_some() {
                        return Err(Error::DuplicateField(TAG_NARHASH));
                    }
                }
                TAG_NARSIZE => {
                    let val = val
                        .parse::<u64>()
                        .map_err(|_| Error::UnableToParseSize(TAG_NARSIZE, val.to_string()))?;

                    if nar_size.replace(val).is_some() {
                        return Err(Error::DuplicateField(TAG_NARSIZE));
                    }
                }
                TAG_REFERENCES => {
                    let val: Vec<StorePathRef> = if !val.is_empty() {
                        let mut prev = "";
                        val.split(' ')
                            .enumerate()
                            .map(|(i, s)| {
                                // TODO(edef): track *duplicates* if this occurs
                                if mem::replace(&mut prev, s) >= s {
                                    flags |= Flags::REFERENCES_OUT_OF_ORDER;
                                }

                                StorePathRef::from_bytes(s.as_bytes())
                                    .map_err(|err| Error::InvalidReference(i, err))
                            })
                            .collect::<Result<_, _>>()?
                    } else {
                        vec![]
                    };

                    if references.replace(val).is_some() {
                        return Err(Error::DuplicateField(TAG_REFERENCES));
                    }
                }
                TAG_SYSTEM => {
                    if val.is_empty() {
                        return Err(Error::EmptyField(TAG_SYSTEM));
                    }

                    if system.replace(val).is_some() {
                        return Err(Error::DuplicateField(TAG_SYSTEM));
                    }
                }
                TAG_DERIVER => {
                    match val.strip_suffix(".drv") {
                        Some(val) => {
                            let val = StorePathRef::from_bytes(val.as_bytes())
                                .map_err(Error::InvalidDeriverStorePath)?;

                            if deriver.replace(val).is_some() {
                                return Err(Error::DuplicateField(TAG_DERIVER));
                            }
                        }
                        None => {
                            if val == "unknown-deriver" {
                                flags |= Flags::EXPLICIT_UNKNOWN_DERIVER;
                            } else {
                                return Err(Error::InvalidDeriverStorePathMissingSuffix);
                            }
                        }
                    };
                }
                TAG_SIG => {
                    let val = SignatureRef::parse(val)
                        .map_err(|e| Error::UnableToParseSignature(signatures.len(), e))?;

                    signatures.push(val);
                }
                TAG_CA => {
                    let val = CAHash::from_nix_hex_str(val)
                        .ok_or_else(|| Error::UnableToParseCA(val.to_string()))?;

                    if ca.replace(val).is_some() {
                        return Err(Error::DuplicateField(TAG_CA));
                    }
                }
                _ => {
                    flags |= Flags::UNKNOWN_FIELD;
                }
            }
        }

        Ok(NarInfo {
            store_path: store_path.ok_or(Error::MissingField("StorePath"))?,
            nar_hash: nar_hash.ok_or(Error::MissingField("NarHash"))?,
            nar_size: nar_size.ok_or(Error::MissingField("NarSize"))?,
            references: match references {
                Some(val) => val,
                None => {
                    flags |= Flags::REFERENCES_MISSING;
                    vec![]
                }
            },
            signatures,
            ca,
            system,
            deriver,
            url: url.ok_or(Error::MissingField("URL"))?,
            compression: match compression {
                Some("none") => None,
                None => {
                    flags |= Flags::COMPRESSION_DEFAULT;
                    Some("bzip2")
                }
                _ => compression,
            },
            file_hash,
            file_size,
            flags,
        })
    }

    /// Computes the fingerprint string for certain fields in this [NarInfo].
    /// This fingerprint is signed in [self.signatures].
    pub fn fingerprint(&self) -> String {
        fingerprint(
            &self.store_path,
            &self.nar_hash,
            self.nar_size,
            self.references.iter(),
        )
    }

    /// Adds a signature, using the passed signer to sign.
    /// This is generic over algo implementations / providers,
    /// so users can bring their own signers.
    pub fn add_signature<S>(&mut self, signer: &'a SigningKey<S>)
    where
        S: ed25519::signature::Signer<ed25519::Signature>,
    {
        // calculate the fingerprint to sign
        let fp = self.fingerprint();

        let sig = signer.sign(fp.as_bytes());

        self.signatures.push(sig);
    }
}

impl Display for NarInfo<'_> {
    fn fmt(&self, w: &mut fmt::Formatter) -> fmt::Result {
        writeln!(w, "StorePath: /nix/store/{}", self.store_path)?;
        writeln!(w, "URL: {}", self.url)?;

        if !self.flags.contains(Flags::COMPRESSION_DEFAULT) {
            let compression = self.compression.unwrap_or("none");
            writeln!(w, "Compression: {compression}")?;
        };

        if let Some(file_hash) = self.file_hash {
            writeln!(w, "FileHash: sha256:{}", nixbase32::encode(&file_hash),)?;
        }

        if let Some(file_size) = self.file_size {
            writeln!(w, "FileSize: {file_size}")?;
        }

        writeln!(w, "NarHash: sha256:{}", nixbase32::encode(&self.nar_hash),)?;
        writeln!(w, "NarSize: {}", self.nar_size)?;

        if !self.flags.contains(Flags::REFERENCES_MISSING) {
            write!(w, "References:")?;
            if self.references.is_empty() {
                write!(w, " ")?;
            } else {
                for path in &self.references {
                    write!(w, " {path}")?;
                }
            }
            writeln!(w)?;
        }

        if let Some(deriver) = &self.deriver {
            writeln!(w, "Deriver: {deriver}.drv")?;
        } else if self.flags.contains(Flags::EXPLICIT_UNKNOWN_DERIVER) {
            writeln!(w, "Deriver: unknown-deriver")?;
        }

        if let Some(system) = self.system {
            writeln!(w, "System: {system}")?;
        }

        for sig in &self.signatures {
            writeln!(w, "Sig: {sig}")?;
        }

        if let Some(ca) = &self.ca {
            writeln!(w, "CA: {}", ca.to_nix_nixbase32_string())?;
        }

        Ok(())
    }
}

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("duplicate field: {0}")]
    DuplicateField(&'static str),

    #[error("missing field: {0}")]
    MissingField(&'static str),

    #[error("invalid line: {0}")]
    InvalidLine(String),

    #[error("invalid StorePath: {0}")]
    InvalidStorePath(crate::store_path::Error),

    #[error("field {0} may not be empty string")]
    EmptyField(&'static str),

    #[error("invalid {0}: {1}")]
    UnableToParseSize(&'static str, String),

    #[error("unable to parse #{0} reference: {1}")]
    InvalidReference(usize, crate::store_path::Error),

    #[error("invalid Deriver store path: {0}")]
    InvalidDeriverStorePath(crate::store_path::Error),

    #[error("invalid Deriver store path, must end with .drv")]
    InvalidDeriverStorePathMissingSuffix,

    #[error("missing prefix for {0}")]
    MissingPrefixForHash(&'static str),

    #[error("unable to decode {0}: {1}")]
    UnableToDecodeHash(&'static str, data_encoding::DecodeError),

    #[error("unable to parse signature #{0}: {1}")]
    UnableToParseSignature(usize, SignatureError),

    #[error("unable to parse CA field: {0}")]
    UnableToParseCA(String),
}

#[cfg(test)]
const DUMMY_KEYPAIR: &str = "cache.example.com-1:cCta2MEsRNuYCgWYyeRXLyfoFpKhQJKn8gLMeXWAb7vIpRKKo/3JoxJ24OYa3DxT2JVV38KjK/1ywHWuMe2JEw==";
#[cfg(test)]
const DUMMY_VERIFYING_KEY: &str =
    "cache.example.com-1:yKUSiqP9yaMSduDmGtw8U9iVVd/Coyv9csB1rjHtiRM=";

#[cfg(test)]
mod test {
    use hex_literal::hex;
    use pretty_assertions::assert_eq;
    use std::sync::LazyLock;
    use std::{io, str};

    use crate::{
        nixhash::{CAHash, NixHash},
        store_path::StorePathRef,
    };

    use super::{Flags, NarInfo};

    static CASES: LazyLock<&'static [&'static str]> = LazyLock::new(|| {
        let data = zstd::decode_all(io::Cursor::new(include_bytes!(
            "../../testdata/narinfo.zst"
        )))
        .unwrap();
        let data = str::from_utf8(Vec::leak(data)).unwrap();
        Vec::leak(
            data.split_inclusive("\n\n")
                .map(|s| s.strip_suffix('\n').unwrap())
                .collect::<Vec<_>>(),
        )
    });

    #[test]
    fn roundtrip() {
        for &input in *CASES {
            let parsed = NarInfo::parse(input).expect("should parse");
            let output = format!("{parsed}");
            assert_eq!(input, output, "should roundtrip");
        }
    }

    #[test]
    fn references_out_of_order() {
        let parsed = NarInfo::parse(
            r#"StorePath: /nix/store/xi429w4ddvb1r77978hm7jfb2jsn559r-gcc-3.4.6
URL: nar/1hr09cgkyw1hcsfkv5qp5jlpmf2mqrkrqs3xj5zklq9c1h9544ff.nar.bz2
Compression: bzip2
FileHash: sha256:1hr09cgkyw1hcsfkv5qp5jlpmf2mqrkrqs3xj5zklq9c1h9544ff
FileSize: 4006
NarHash: sha256:0ik9mpqxpd9hv325hdblj2nawqj5w7951qdyy8ikxgwr6fq7m11c
NarSize: 21264
References: a8922c0h87iilxzzvwn2hmv8x210aqb9-glibc-2.7 7w2acjgalb0cm7b3bg8yswza4l7iil9y-binutils-2.18 mm631h09mj964hm9q04l5fd8vw12j1mm-bash-3.2-p39 nx2zs2qd6snfcpzw4a0jnh26z9m0yihz-gcc-3.4.6 xi429w4ddvb1r77978hm7jfb2jsn559r-gcc-3.4.6
Deriver: 2dzpn70c1hawczwhg9aavqk18zp9zsva-gcc-3.4.6.drv
Sig: cache.nixos.org-1:o1DTsjCz0PofLJ216P2RBuSulI8BAb6zHxWE4N+tzlcELk5Uk/GO2SCxWTRN5wJutLZZ+cHTMdWqOHF88KGQDg==
"#).expect("should parse");

        assert!(parsed.flags.contains(Flags::REFERENCES_OUT_OF_ORDER));
        assert_eq!(
            vec![
                "a8922c0h87iilxzzvwn2hmv8x210aqb9-glibc-2.7",
                "7w2acjgalb0cm7b3bg8yswza4l7iil9y-binutils-2.18",
                "mm631h09mj964hm9q04l5fd8vw12j1mm-bash-3.2-p39",
                "nx2zs2qd6snfcpzw4a0jnh26z9m0yihz-gcc-3.4.6",
                "xi429w4ddvb1r77978hm7jfb2jsn559r-gcc-3.4.6"
            ],
            parsed
                .references
                .iter()
                .map(StorePathRef::to_string)
                .collect::<Vec<_>>(),
        );
    }

    #[test]
    fn ca_nar_hash_sha1() {
        let parsed = NarInfo::parse(
            r#"StorePath: /nix/store/k20pahypzvr49fy82cw5sx72hdfg3qcr-texlive-hyphenex-37354
URL: nar/0i5biw0g01514llhfswxy6xfav8lxxdq1xg6ik7hgsqbpw0f06yi.nar.xz
Compression: xz
FileHash: sha256:0i5biw0g01514llhfswxy6xfav8lxxdq1xg6ik7hgsqbpw0f06yi
FileSize: 7120
NarHash: sha256:0h1bm4sj1cnfkxgyhvgi8df1qavnnv94sd0v09wcrm971602shfg
NarSize: 22552
References: 
Sig: cache.nixos.org-1:u01BybwQhyI5H1bW1EIWXssMDhDDIvXOG5uh8Qzgdyjz6U1qg6DHhMAvXZOUStIj6X5t4/ufFgR8i3fjf0bMAw==
CA: fixed:r:sha1:1ak1ymbmsfx7z8kh09jzkr3a4dvkrfjw
"#).expect("should parse");

        assert_eq!(
            parsed.ca,
            Some(CAHash::Nar(NixHash::Sha1(hex!(
                "5cba3c77236ae4f9650270a27fbad375551fa60a"
            ))))
        );
    }

    #[test]
    fn compression_default() {
        // This doesn't exist as such in cache.nixos.org.
        // We explicitly removed the compression field for the sake of this test.
        let input = r#"StorePath: /nix/store/a1jjalr4csx9hcga7fnm122aqabrjnch-digikam-2.6.0
URL: nar/1fzimfnvq2k8b40n4g54abmncpx2ddckh6qlb77pgq6xiysyil69.nar.bz2
FileHash: sha256:1fzimfnvq2k8b40n4g54abmncpx2ddckh6qlb77pgq6xiysyil69
FileSize: 43503778
NarHash: sha256:0zpbbwipqzr5p8mlpag9wrsp5hlaxkq7gax5jj0hg3vvdziypcw5
NarSize: 100658640
References: 0izkyk7bq2ag9393nvnhgm87p75cq09w-liblqr-1-0.4.1 1cslpgyb7vb30inj3210jv6agqv42jxz-qca-2.0.3 1sya3bwjxkzpkmwn67gfzp4gz4g62l36-libXrandr-1.3.1 26yxdaa9z0ma5sgw02i670rsqnl57crs-glib-2.30.3 27lnjh99236kmhbpc5747599zcymfzmg-qt-4.8.2 2v6x378vcfvyxilkvihs60zha54z2x2y-qjson-0.7.1 45hgr3fbnr45n795hn2x7hsymp0h2j2m-libjpeg-8c 4kw1b212s80ap2iyibxrimcqb5imhfj7-libkexiv2-4.7.4 7dvylm5crlc0sfafcc0n46mb5ch67q0j-glibc-2.13 a05cbh1awjbl1rbyb2ynyf4k42v5a9a7-boost-1.47.0 a1jjalr4csx9hcga7fnm122aqabrjnch-digikam-2.6.0 aav5ffg8wlnilgnvdb2jnrv2aam4zmmz-perl-5.14.2 ab0m9h30nsr13w48qriv0k350kmwx567-kdelibs-4.7.4 avffkd49cqvpwdkzry8bn69dkbw4cy29-lensfun-0.2.5 cy8rl8h4yp2j3h8987vkklg328q3wmjz-gcc-4.6.3 dmmh5ihyg1r2dm4azgsfj2kprj92czlg-libSM-1.2.0 fl56j5n4shfw9c0r6vs2i4f1h9zx5kac-soprano-2.7.6 g15cmvh15ggdjcwapskngv20q4yhix40-jasper-1.900.1 i04maxd0din6v92rnqcwl9yra0kl2vk5-marble-4.7.4 kqjjb3m26rdddwwwkk8v45821aps877k-libICE-1.0.7 lxz9r135wkndvi642z4bjgmvyypsgirb-libtiff-3.9.4 m9c8i0a6cl30lcqp654dqkbag3wjmd00-libX11-1.4.1 mpnj4k2ijrgyfkh48fg96nzcmklfh5pl-coreutils-8.15 nppljblap477s0893c151lyq7r7n5v1q-zlib-1.2.7 nw9mdbyp8kyn3v4vkdzq0gsnqbc4mnx3-expat-2.0.1 p1a0dn931mzdkvj6h5yzshbmgxba5r0z-libgphoto2-2.4.11 pvjj07xa1cfkad3gwk376nzdrgknbcqm-mesa-7.11.2 pzcxag98jqccp9ycbxknyh0w95pgnsk4-lcms-1.19 qfi5pgds33kg6vlnxsmj0hyl74vcmyiz-libpng-1.5.10 scm6bj86s3qh3s3x0b9ayjp6755p4q86-mysql-5.1.54 sd23qspcyg385va0lr35xgz3hvlqphg6-libkipi-4.7.4 svmbrhc6kzfzakv20a7zrfl6kbr5mfpq-kdepimlibs-4.7.4 v7kh3h7xfwjz4hgffg3gwrfzjff9bw9d-bash-4.2-p24 vi17f22064djgpk0w248da348q8gxkww-libkdcraw-4.7.4 wkjdzmj3z4dcbsc9f833zs6krdgg2krk-phonon-4.6.0 xf3i3awqi0035ixy2qyb6hk4c92r3vrn-opencv-2.4.2 y1vr0nz8i59x59501020nh2k1dw3bhwq-libusb-0.1.12 yf3hin2hb6i08n7zrk8g3acy54rhg9bp-libXext-1.2.0
Deriver: la77dr44phk5m5jnl4dvk01cwpykyw9s-digikam-2.6.0.drv
System: i686-linux
Sig: cache.nixos.org-1:92fl0i5q7EyegCj5Yf4L0bENkWuVAtgveiRcTEEUH0P6HvCE1xFcPbz/0Pf6Np+K1LPzHK+s5RHOmVoxRsvsDg==
"#;
        let parsed = NarInfo::parse(input).expect("should parse");

        assert!(parsed.flags.contains(Flags::COMPRESSION_DEFAULT));
        assert_eq!(parsed.compression, Some("bzip2"));
        assert_eq!(parsed.to_string(), input);
    }

    #[test]
    fn compression_none() {
        // This doesn't exist as such in cache.nixos.org.
        // We explicitly changed the Compression, FileHash and FileSize fields for the sake of this test.
        let input = r#"StorePath: /nix/store/a1jjalr4csx9hcga7fnm122aqabrjnch-digikam-2.6.0
URL: nar/1fzimfnvq2k8b40n4g54abmncpx2ddckh6qlb77pgq6xiysyil69.nar.bz2
Compression: none
FileHash: sha256:0zpbbwipqzr5p8mlpag9wrsp5hlaxkq7gax5jj0hg3vvdziypcw5
FileSize: 100658640
NarHash: sha256:0zpbbwipqzr5p8mlpag9wrsp5hlaxkq7gax5jj0hg3vvdziypcw5
NarSize: 100658640
References: 0izkyk7bq2ag9393nvnhgm87p75cq09w-liblqr-1-0.4.1 1cslpgyb7vb30inj3210jv6agqv42jxz-qca-2.0.3 1sya3bwjxkzpkmwn67gfzp4gz4g62l36-libXrandr-1.3.1 26yxdaa9z0ma5sgw02i670rsqnl57crs-glib-2.30.3 27lnjh99236kmhbpc5747599zcymfzmg-qt-4.8.2 2v6x378vcfvyxilkvihs60zha54z2x2y-qjson-0.7.1 45hgr3fbnr45n795hn2x7hsymp0h2j2m-libjpeg-8c 4kw1b212s80ap2iyibxrimcqb5imhfj7-libkexiv2-4.7.4 7dvylm5crlc0sfafcc0n46mb5ch67q0j-glibc-2.13 a05cbh1awjbl1rbyb2ynyf4k42v5a9a7-boost-1.47.0 a1jjalr4csx9hcga7fnm122aqabrjnch-digikam-2.6.0 aav5ffg8wlnilgnvdb2jnrv2aam4zmmz-perl-5.14.2 ab0m9h30nsr13w48qriv0k350kmwx567-kdelibs-4.7.4 avffkd49cqvpwdkzry8bn69dkbw4cy29-lensfun-0.2.5 cy8rl8h4yp2j3h8987vkklg328q3wmjz-gcc-4.6.3 dmmh5ihyg1r2dm4azgsfj2kprj92czlg-libSM-1.2.0 fl56j5n4shfw9c0r6vs2i4f1h9zx5kac-soprano-2.7.6 g15cmvh15ggdjcwapskngv20q4yhix40-jasper-1.900.1 i04maxd0din6v92rnqcwl9yra0kl2vk5-marble-4.7.4 kqjjb3m26rdddwwwkk8v45821aps877k-libICE-1.0.7 lxz9r135wkndvi642z4bjgmvyypsgirb-libtiff-3.9.4 m9c8i0a6cl30lcqp654dqkbag3wjmd00-libX11-1.4.1 mpnj4k2ijrgyfkh48fg96nzcmklfh5pl-coreutils-8.15 nppljblap477s0893c151lyq7r7n5v1q-zlib-1.2.7 nw9mdbyp8kyn3v4vkdzq0gsnqbc4mnx3-expat-2.0.1 p1a0dn931mzdkvj6h5yzshbmgxba5r0z-libgphoto2-2.4.11 pvjj07xa1cfkad3gwk376nzdrgknbcqm-mesa-7.11.2 pzcxag98jqccp9ycbxknyh0w95pgnsk4-lcms-1.19 qfi5pgds33kg6vlnxsmj0hyl74vcmyiz-libpng-1.5.10 scm6bj86s3qh3s3x0b9ayjp6755p4q86-mysql-5.1.54 sd23qspcyg385va0lr35xgz3hvlqphg6-libkipi-4.7.4 svmbrhc6kzfzakv20a7zrfl6kbr5mfpq-kdepimlibs-4.7.4 v7kh3h7xfwjz4hgffg3gwrfzjff9bw9d-bash-4.2-p24 vi17f22064djgpk0w248da348q8gxkww-libkdcraw-4.7.4 wkjdzmj3z4dcbsc9f833zs6krdgg2krk-phonon-4.6.0 xf3i3awqi0035ixy2qyb6hk4c92r3vrn-opencv-2.4.2 y1vr0nz8i59x59501020nh2k1dw3bhwq-libusb-0.1.12 yf3hin2hb6i08n7zrk8g3acy54rhg9bp-libXext-1.2.0
Deriver: la77dr44phk5m5jnl4dvk01cwpykyw9s-digikam-2.6.0.drv
System: i686-linux
Sig: cache.nixos.org-1:92fl0i5q7EyegCj5Yf4L0bENkWuVAtgveiRcTEEUH0P6HvCE1xFcPbz/0Pf6Np+K1LPzHK+s5RHOmVoxRsvsDg==
"#;
        let parsed = NarInfo::parse(input).expect("should parse");

        assert!(!parsed.flags.contains(Flags::COMPRESSION_DEFAULT));
        assert_eq!(parsed.compression, None);
        assert_eq!(parsed.to_string(), input);
    }

    #[test]
    fn explicit_unknown_deriver() {
        // This is a NARInfo "produced by a legacy tool" according to Nix commit
        // c60715e937e3773bbb8a114fc9b9c6577f8c5cb5
        let input = r#"StorePath: /nix/store/00bgd045z0d4icpbc2yyz4gx48ak44la-net-tools-1.60_p20170221182432
URL: nar/1094wph9z4nwlgvsd53abfz8i117ykiv5dwnq9nnhz846s7xqd7d.nar.xz
Compression: xz
FileHash: sha256:1094wph9z4nwlgvsd53abfz8i117ykiv5dwnq9nnhz846s7xqd7d
FileSize: 114980
NarHash: sha256:0lxjvvpr59c2mdram7ympy5ay741f180kv3349hvfc3f8nrmbqf6
NarSize: 464152
References: 7gx4kiv5m0i7d7qkixq2cwzbr10lvxwc-glibc-2.27
Deriver: unknown-deriver
Sig: cache.nixos.org-1:sn5s/RrqEI+YG6/PjwdbPjcAC7rcta7sJU4mFOawGvJBLsWkyLtBrT2EuFt/LJjWkTZ+ZWOI9NTtjo/woMdvAg==
Sig: hydra.other.net-1:JXQ3Z/PXf0EZSFkFioa4FbyYpbbTbHlFBtZf4VqU0tuMTWzhMD7p9Q7acJjLn3jofOtilAAwRILKIfVuyrbjAA==
"#;
        let parsed = NarInfo::parse(input).expect("should parse");

        assert!(parsed.flags.contains(Flags::EXPLICIT_UNKNOWN_DERIVER));
        assert!(parsed.deriver.is_none());
        assert_eq!(parsed.to_string(), input);
    }

    #[test]
    fn nar_hash_hex() {
        let parsed = NarInfo::parse(r#"StorePath: /nix/store/0vpqfxbkx0ffrnhbws6g9qwhmliksz7f-perl-HTTP-Cookies-6.01
URL: nar/1rv1m9inydm1r4krw8hmwg1hs86d0nxddd1pbhihx7l7fycjvfk3.nar.xz
Compression: xz
FileHash: sha256:1rv1m9inydm1r4krw8hmwg1hs86d0nxddd1pbhihx7l7fycjvfk3
FileSize: 19912
NarHash: sha256:60adfd293a4d81ad7cd7e47263cbb3fc846309ef91b154a08ba672b558f94ff3
NarSize: 45840
References: 0vpqfxbkx0ffrnhbws6g9qwhmliksz7f-perl-HTTP-Cookies-6.01 9vrhbib2lxd9pjlg6fnl5b82gblidrcr-perl-HTTP-Message-6.06 wy20zslqxzxxfpzzk0rajh41d7a6mlnf-perl-HTTP-Date-6.02
Deriver: fb4ihlq3psnsjq95mvvs49rwpplpc8zj-perl-HTTP-Cookies-6.01.drv
Sig: cache.nixos.org-1:HhaiY36Uk3XV1JGe9d9xHnzAapqJXprU1YZZzSzxE97jCuO5RR7vlG2kF7MSC5thwRyxAtdghdSz3AqFi+QSCw==
"#).expect("should parse");

        assert!(parsed.flags.contains(Flags::NAR_HASH_HEX));
        assert_eq!(
            hex!("60adfd293a4d81ad7cd7e47263cbb3fc846309ef91b154a08ba672b558f94ff3"),
            parsed.nar_hash,
        );
    }

    #[test]
    fn references_missing() {
        // This is a NARInfo without a References field.
        // This NARInfo was produced by harmonia (but the signature was altered).
        let input = r#"StorePath: /nix/store/64s9zav4fk5qiba1jq0ipvyhnn57r7dq-cfg-if-1.0.0
URL: nar/0lxxfhy5fmfz0sbnqkqjdf7gx9gsxrfzz49n19y8sr93inawhshh.nar?hash=64s9zav4fk5qiba1jq0ipvyhnn57r7dq
Compression: none
FileHash: sha256:0lxxfhy5fmfz0sbnqkqjdf7gx9gsxrfzz49n19y8sr93inawhshh
FileSize: 24944
NarHash: sha256:0lxxfhy5fmfz0sbnqkqjdf7gx9gsxrfzz49n19y8sr93inawhshh
NarSize: 24944
Deriver: s409kxiz6bx2g0da01gzvlnnjpl3i4h9-cfg-if-1.0.0.drv
Sig: cache.nixos.org-1:WDvKIdxSnQ8p2w9SD0ffdibUSNMz6QQN6jpe+A8LLNHmZFsX+m8GZF0x9DN6PWV6k+OlnBT5UVbiWQYgXIsQAQ==
"#;
        let parsed = NarInfo::parse(input).expect("should parse");

        assert!(parsed.flags.contains(Flags::REFERENCES_MISSING));
        assert_eq!(parsed.references, vec![]);
        assert_eq!(parsed.to_string(), input);
    }

    /// Adds a signature to a NARInfo, using key material parsed from DUMMY_KEYPAIR.
    /// It then ensures signature verification with the parsed
    /// DUMMY_VERIFYING_KEY succeeds.
    #[test]
    fn sign() {
        let mut narinfo = NarInfo::parse(
            r#"StorePath: /nix/store/0vpqfxbkx0ffrnhbws6g9qwhmliksz7f-perl-HTTP-Cookies-6.01
URL: nar/0i5biw0g01514llhfswxy6xfav8lxxdq1xg6ik7hgsqbpw0f06yi.nar.xz
Compression: xz
FileHash: sha256:0i5biw0g01514llhfswxy6xfav8lxxdq1xg6ik7hgsqbpw0f06yi
FileSize: 7120
NarHash: sha256:0h1bm4sj1cnfkxgyhvgi8df1qavnnv94sd0v09wcrm971602shfg
NarSize: 22552
References: 
CA: fixed:r:sha1:1ak1ymbmsfx7z8kh09jzkr3a4dvkrfjw
"#,
        )
        .expect("should parse");

        let fp = narinfo.fingerprint();

        // load our keypair from the fixtures
        let (signing_key, _verifying_key) =
            super::parse_keypair(super::DUMMY_KEYPAIR).expect("must succeed");

        // add signature
        narinfo.add_signature(&signing_key);

        // ensure the signature is added
        let new_sig = narinfo.signatures.last().unwrap();
        assert_eq!(signing_key.name(), *new_sig.name());

        // verify the new signature against the verifying key
        let verifying_key = super::VerifyingKey::parse(super::DUMMY_VERIFYING_KEY)
            .expect("parsing dummy verifying key");

        assert!(
            verifying_key.verify(&fp, new_sig),
            "expect signature to be valid"
        );
    }
}
