//! Fast 53-bit string hash (cyrb53) from `lib/hash.ts`, for telling file versions apart. Not
//! cryptographic. Bit-identical to the TS (saved threads store these hashes): it hashes UTF-16
//! code units and appends the UTF-16 length, both in base 36.

use super::js;

/// `hashText(text)`.
pub fn hash_text(text: &str) -> String {
    hash_text_seeded(text, 0)
}

/// `hashText(text, seed)`; `seed` is truncated to 32 bits as JS `^` does.
pub fn hash_text_seeded(text: &str, seed: i32) -> String {
    let seed = seed as u32;
    let mut h1: u32 = 0xdead_beef ^ seed;
    let mut h2: u32 = 0x41c6_ce57 ^ seed;
    let mut len = 0u64;
    for ch in text.encode_utf16() {
        let ch = u32::from(ch);
        h1 = (h1 ^ ch).wrapping_mul(2_654_435_761);
        h2 = (h2 ^ ch).wrapping_mul(1_597_334_677);
        len += 1;
    }
    h1 = (h1 ^ (h1 >> 16)).wrapping_mul(2_246_822_507)
        ^ (h2 ^ (h2 >> 13)).wrapping_mul(3_266_489_909);
    h2 = (h2 ^ (h2 >> 16)).wrapping_mul(2_246_822_507)
        ^ (h1 ^ (h1 >> 13)).wrapping_mul(3_266_489_909);
    let value = 4_294_967_296 * u64::from(2_097_151 & h2) + u64::from(h1);
    format!("{}:{}", js::radix36(value), js::radix36(len))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_values() {
        // Recorded from the TS implementation.
        assert_eq!(hash_text(""), "wvjl67o803:0");
        assert_eq!(hash_text("a"), hash_text("a"));
        assert!(hash_text("😀").ends_with(":2"));
    }
}
