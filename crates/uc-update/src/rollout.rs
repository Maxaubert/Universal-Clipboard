//! Staged rollout (plan 3.13): a device takes an update only if a hash of its device ID falls
//! inside the rollout percentage. The bucket of a device never changes, so the same devices
//! go first every time, and a device inside 10% is also inside 100%.

/// The bucket of a device, 0 to 99.
pub fn bucket(device_id: &[u8]) -> u8 {
    let mut h = blake3::Hasher::new();
    h.update(b"uclip rollout v1");
    h.update(device_id);
    let b = h.finalize();
    let n = u16::from_le_bytes([b.as_bytes()[0], b.as_bytes()[1]]);
    (n % 100) as u8
}

/// True if this device takes a release rolled out to `percent`.
pub fn in_rollout(device_id: &[u8], percent: u8) -> bool {
    bucket(device_id) < percent.min(100)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edges_and_spread() {
        let id = [7u8; 32];
        assert!(!in_rollout(&id, 0));
        assert!(in_rollout(&id, 100));
        let mut inside = 0;
        for i in 0..2000u32 {
            if in_rollout(&i.to_le_bytes(), 10) {
                inside += 1;
            }
        }
        // About 10% of 2000.
        assert!((120..=280).contains(&inside), "{inside}");
    }

    #[test]
    fn inside_10_is_inside_50() {
        for i in 0..500u32 {
            let id = i.to_le_bytes();
            if in_rollout(&id, 10) {
                assert!(in_rollout(&id, 50));
            }
        }
    }
}
