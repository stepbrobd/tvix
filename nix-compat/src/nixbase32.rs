//! Implements the slightly odd "base32" encoding that's used in Nix.
//!
//! Nix uses a custom alphabet. Contrary to other implementations (RFC4648),
//! encoding to "nix base32" doesn't use any padding, and reads in characters
//! in reverse order.
//!
//! This is also the main reason why we can't use `data_encoding::Encoding` -
//! it gets things wrong if there normally would be a need for padding.

use std::fmt::Write;

use data_encoding::{DecodeError, DecodeKind};

const ALPHABET: &[u8; 32] = b"0123456789abcdfghijklmnpqrsvwxyz";

/// Returns encoded input
pub fn encode(input: &[u8]) -> String {
    let output_len = encode_len(input.len());
    let mut output = String::with_capacity(output_len);

    for n in (0..output_len).rev() {
        let b = n * 5; // bit offset within the entire input
        let i = b / 8; // input byte index
        let j = b % 8; // bit offset within that input byte

        // 5-bit words aren't aligned to bytes
        // we can only read byte-aligned units
        // read 16 bits then shift and mask to 5
        let c = {
            let mut word = input[i] as u16;
            if let Some(&msb) = input.get(i + 1) {
                word |= (msb as u16) << 8;
            }
            (word >> j) & 0x1f
        };

        output.write_char(ALPHABET[c as usize] as char).unwrap();
    }

    output
}

/// This maps a nixbase32-encoded character to its binary representation, which
/// is also the index of the character in the alphabet. Invalid characters are
/// mapped to 0xFF, which is itself an invalid value.
const BASE32_ORD: [u8; 256] = {
    let mut ord = [0xFF; 256];
    let mut alphabet = ALPHABET.as_slice();
    let mut i = 0;

    while let &[c, ref tail @ ..] = alphabet {
        ord[c as usize] = i;
        alphabet = tail;
        i += 1;
    }

    ord
};

/// Returns decoded input
pub fn decode(input: impl AsRef<[u8]>) -> Result<Vec<u8>, DecodeError> {
    let input = input.as_ref();

    let output_len = decode_len(input.len());
    if input.len() != encode_len(output_len) {
        return Err(DecodeError {
            position: input.len().min(encode_len(output_len)),
            kind: DecodeKind::Length,
        });
    }
    let mut output: Vec<u8> = vec![0x00; output_len];

    decode_inner(input, &mut output)?;
    Ok(output)
}

pub fn decode_fixed<const K: usize>(input: impl AsRef<[u8]>) -> Result<[u8; K], DecodeError> {
    let input = input.as_ref();

    if input.len() != encode_len(K) {
        return Err(DecodeError {
            position: input.len().min(encode_len(K)),
            kind: DecodeKind::Length,
        });
    }

    let mut output = [0; K];
    decode_inner(input, &mut output)?;
    Ok(output)
}

fn decode_inner(input: &[u8], output: &mut [u8]) -> Result<(), DecodeError> {
    // loop over all characters in reverse, and keep the iteration count in n.
    let mut carry = 0;
    let mut mask = 0;
    for (n, &c) in input.iter().rev().enumerate() {
        let b = n * 5;
        let i = b / 8;
        let j = b % 8;

        let digit = BASE32_ORD[c as usize];
        let value = (digit as u16) << j;
        output[i] |= value as u8 | carry;
        carry = (value >> 8) as u8;

        mask |= digit;
    }

    if mask == 0xFF {
        return Err(DecodeError {
            position: find_invalid(input),
            kind: DecodeKind::Symbol,
        });
    }

    // if we're at the end, but have a nonzero carry, the encoding is invalid.
    if carry != 0 {
        return Err(DecodeError {
            position: 0,
            kind: DecodeKind::Trailing,
        });
    }

    Ok(())
}

fn find_invalid(input: &[u8]) -> usize {
    for (i, &c) in input.iter().enumerate() {
        if !ALPHABET.contains(&c) {
            return i;
        }
    }

    unreachable!()
}

/// Returns the decoded length of an input of length len.
pub const fn decode_len(len: usize) -> usize {
    (len * 5) / 8
}

/// Returns the encoded length of an input of length len
pub const fn encode_len(len: usize) -> usize {
    (len * 8).div_ceil(5)
}

#[cfg(test)]
mod tests {
    use hex_literal::hex;
    use rstest::rstest;

    #[rstest]
    #[case::empty_bytes("", &[])]
    #[case::one_byte("0z", &hex!("1f"))]
    #[case::store_path("00bgd045z0d4icpbc2yyz4gx48ak44la", &hex!("8a12321522fd91efbd60ebb2481af88580f61600"))]
    #[case::sha256("0c5b8vw40dy178xlpddw65q9gf1h2186jcc3p4swinwggbllv8mk", &hex!("b3a24de97a8fdbc835b9833169501030b8977031bcb54b3b3ac13740f846ab30"))]
    #[test]
    fn encode(#[case] enc: &str, #[case] dec: &[u8]) {
        assert_eq!(enc, super::encode(dec));
    }

    #[rstest]
    #[case::empty_bytes("", Some(&[][..]) )]
    #[case::one_byte("0z", Some(&hex!("1f")[..]))]
    #[case::store_path("00bgd045z0d4icpbc2yyz4gx48ak44la", Some(&hex!("8a12321522fd91efbd60ebb2481af88580f61600")[..]))]
    #[case::sha256("0c5b8vw40dy178xlpddw65q9gf1h2186jcc3p4swinwggbllv8mk", Some(&hex!("b3a24de97a8fdbc835b9833169501030b8977031bcb54b3b3ac13740f846ab30")[..]))]
    // this is invalid encoding, because it encodes 10 1-bits, so the carry
    // would be 2 1-bits
    #[case::invalid_encoding_1("zz", None)]
    // this is an even more specific example - it'd decode as 00000000 11
    #[case::invalid_encoding_2("c0", None)]
    // This has an invalid length
    #[case::invalid_encoding_3("0", None)]
    // This has an invalid length
    #[case::invalid_encoding_4("0zz", None)]
    #[test]
    fn decode(#[case] enc: &str, #[case] dec: Option<&[u8]>) {
        match dec {
            Some(dec) => {
                // The decode needs to match what's passed in dec
                assert_eq!(dec, super::decode(enc).unwrap());
            }
            None => {
                // the decode needs to be an error
                assert!(super::decode(enc).is_err());
            }
        }
    }

    #[test]
    fn decode_fixed() {
        assert_eq!(
            super::decode_fixed("00bgd045z0d4icpbc2yyz4gx48ak44la").unwrap(),
            hex!("8a12321522fd91efbd60ebb2481af88580f61600")
        );
        assert_eq!(
            super::decode_fixed::<32>("00").unwrap_err(),
            super::DecodeError {
                position: 2,
                kind: super::DecodeKind::Length
            }
        );
    }

    #[test]
    fn encode_len() {
        assert_eq!(super::encode_len(0), 0);
        assert_eq!(super::encode_len(20), 32);
    }

    #[test]
    fn decode_len() {
        assert_eq!(super::decode_len(0), 0);
        assert_eq!(super::decode_len(1), 0);
        assert_eq!(super::decode_len(2), 1);
        assert_eq!(super::decode_len(3), 1);
        assert_eq!(super::decode_len(4), 2);
        assert_eq!(super::decode_len(5), 3);
        assert_eq!(super::decode_len(32), 20);
    }
}
