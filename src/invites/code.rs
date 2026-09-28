//! Invite codes are 128 random bits in Crockford base32, which avoids look-alike letters and
//! is easy to type on a phone. Only a SHA-256 digest is stored, like session tokens

use sha2::{Digest, Sha256};

const ALPHABET: &[u8; 32] = b"0123456789abcdefghjkmnpqrstvwxyz";

/// A new code, grouped as `xxxx-xxxx-…` for reading aloud or copying
pub fn generate() -> String {
    let mut bytes = [0u8; 16];
    rand::fill(&mut bytes);
    let encoded = encode(&bytes);
    encoded
        .as_bytes()
        .chunks(4)
        .map(|chunk| std::str::from_utf8(chunk).expect("ascii alphabet"))
        .collect::<Vec<_>>()
        .join("-")
}

/// Digest of a code as typed: case, dashes and whitespace do not matter
pub fn hash(code: &str) -> Vec<u8> {
    Sha256::digest(normalise(code).as_bytes()).to_vec()
}

fn normalise(code: &str) -> String {
    code.chars()
        .filter(|c| !c.is_whitespace() && *c != '-')
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

fn encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 8 / 5 + 1);
    let mut buffer: u32 = 0;
    let mut bits = 0;
    for &byte in bytes {
        buffer = (buffer << 8) | u32::from(byte);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(ALPHABET[((buffer >> bits) & 31) as usize] as char);
        }
    }
    if bits > 0 {
        out.push(ALPHABET[((buffer << (5 - bits)) & 31) as usize] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_unique_grouped_and_unambiguous() {
        let a = generate();
        let b = generate();
        assert_ne!(a, b);
        assert_eq!(normalise(&a).len(), 26);
        assert!(normalise(&a).bytes().all(|c| ALPHABET.contains(&c)));
        assert!(a.split('-').all(|group| group.len() <= 4));
    }

    #[test]
    fn hashing_ignores_case_dashes_and_spaces() {
        assert_eq!(hash("abcd-efgh"), hash(" ABCD EFGH "));
        assert_ne!(hash("abcd-efgh"), hash("abcd-efgj"));
    }

    #[test]
    fn encodes_known_bytes() {
        assert_eq!(encode(&[0xff]), "zw");
        assert_eq!(encode(&[0, 0, 0, 0, 0]), "00000000");
    }
}
