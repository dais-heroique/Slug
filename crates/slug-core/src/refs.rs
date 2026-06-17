//! Stable reference (`ref`) generation — SEMANTIC-SCHEMA §4.
//!
//! The canonical scheme (§4.1) has the *compositor* mint a time-sortable ULID v4
//! per `wl_surface`-backed widget. Milestone 1 has no compositor: we live on the
//! AT-SPI2 path, so the ULID is **derived deterministically** from the AT-SPI
//! object identity `{unique_bus_name}:{accessible_path}` (see the step-1 adaptation
//! in the task brief). This keeps refs stable across harvests for the lifetime of
//! the application, which is the stability guarantee callers rely on at this stage.
//!
//! The output still conforms to the §4.3 ref format: a 26-character Crockford
//! Base32 string encoding 128 bits. It is simply derived rather than random, and
//! the timestamp component is not meaningful at this milestone.
//!
//! Refs are opaque to the agent (§4.3): nothing outside this module should parse
//! or construct them.

/// Crockford Base32 alphabet (no I, L, O, U), as used by ULID (§4.3).
const CROCKFORD: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// Derive a stable 26-char Crockford-Base32 ref from a stable identity string.
///
/// At milestone 1 the identity is `"{unique_bus_name}:{accessible_path}"`.
/// The mapping is a pure function: equal inputs always yield equal refs.
pub fn derive_ref(identity: &str) -> String {
    let bytes = hash_128(identity.as_bytes());
    encode_crockford_u128(bytes)
}

/// Convenience: derive a ref from an AT-SPI bus name and object path.
pub fn derive_ref_from_atspi(bus_name: &str, path: &str) -> String {
    // The ':' separator cannot appear in a D-Bus unique name or object path,
    // so the concatenation is unambiguous.
    let mut id = String::with_capacity(bus_name.len() + 1 + path.len());
    id.push_str(bus_name);
    id.push(':');
    id.push_str(path);
    derive_ref(&id)
}

/// A 128-bit hash built from two independent 64-bit FNV-1a streams with distinct
/// seeds. FNV is not cryptographic, but for deriving stable, well-distributed
/// 128-bit identifiers from short D-Bus identity strings the collision
/// probability is negligible at milestone-1 scale.
fn hash_128(data: &[u8]) -> u128 {
    const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

    let mut lo = FNV_OFFSET;
    for &b in data {
        lo ^= b as u64;
        lo = lo.wrapping_mul(FNV_PRIME);
    }
    // Second stream: different seed + a length salt so that the two halves do not
    // collapse to the same value for trivially structured inputs.
    let mut hi = FNV_OFFSET ^ 0x9e37_79b9_7f4a_7c15;
    hi = hi.wrapping_add((data.len() as u64).wrapping_mul(FNV_PRIME));
    for &b in data {
        hi ^= (b as u64).rotate_left(7);
        hi = hi.wrapping_mul(FNV_PRIME);
    }
    ((hi as u128) << 64) | (lo as u128)
}

/// Encode a `u128` as a 26-character Crockford Base32 string (130 bits of space,
/// top two bits are always zero), matching the ULID textual encoding.
fn encode_crockford_u128(mut v: u128) -> String {
    let mut buf = [0u8; 26];
    // Fill from the least-significant end.
    for slot in buf.iter_mut().rev() {
        *slot = CROCKFORD[(v & 0x1f) as usize];
        v >>= 5;
    }
    // Safe: CROCKFORD is ASCII.
    String::from_utf8(buf.to_vec()).expect("crockford alphabet is ASCII")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ref_is_deterministic() {
        let a = derive_ref_from_atspi(":1.42", "/org/a11y/atspi/accessible/12");
        let b = derive_ref_from_atspi(":1.42", "/org/a11y/atspi/accessible/12");
        assert_eq!(a, b);
    }

    #[test]
    fn ref_has_canonical_shape() {
        let r = derive_ref_from_atspi(":1.7", "/org/a11y/atspi/accessible/root");
        assert_eq!(r.len(), 26, "refs are 26 chars (§4.3)");
        assert!(r.bytes().all(|c| CROCKFORD.contains(&c)), "crockford alphabet only");
    }

    #[test]
    fn distinct_objects_get_distinct_refs() {
        let a = derive_ref_from_atspi(":1.1", "/org/a11y/atspi/accessible/1");
        let b = derive_ref_from_atspi(":1.1", "/org/a11y/atspi/accessible/2");
        let c = derive_ref_from_atspi(":1.2", "/org/a11y/atspi/accessible/1");
        assert_ne!(a, b);
        assert_ne!(a, c);
        assert_ne!(b, c);
    }
}
