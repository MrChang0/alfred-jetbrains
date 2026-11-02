//! Stable, dependency-free identifiers for Alfred workflow objects.

pub fn fnv1a(value: &str) -> u64 {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

/// A deterministic UUID-shaped identifier (8-4-4-4-12 hex).
pub fn uuid_like(value: &str) -> String {
    let first = fnv1a(value);
    let second = fnv1a(&format!("{value}\u{0}second"));
    let mut hex = format!("{first:016X}{second:016X}")
        .chars()
        .collect::<Vec<_>>();
    // Force a valid UUID v4 shape: version nibble 4, variant nibble 8.
    hex[12] = '4';
    hex[16] = '8';
    let hex = hex.into_iter().collect::<String>();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uuid_is_stable_and_shaped() {
        let first = uuid_like("jb:search");
        let second = uuid_like("jb:search");
        assert_eq!(first, second);
        assert_eq!(first.len(), 36);
        assert_eq!(first.matches('-').count(), 4);
        assert_eq!(first.chars().nth(14), Some('4'));
        assert_eq!(first.chars().nth(19), Some('8'));
    }
}
