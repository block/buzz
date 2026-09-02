//! Project related-channel snapshot identity helpers.

use sha2::{Digest, Sha256};

/// Domain-separation prefix for Project related-channel snapshot coordinates.
pub const PROJECT_RELATED_CHANNELS_SNAPSHOT_DOMAIN: &[u8] = b"buzz:project-related-channels:v1";

/// Maximum effective related channels represented by one snapshot.
pub const PROJECT_RELATED_CHANNELS_SNAPSHOT_CAP: usize = 64;

/// Derive the deterministic lowercase-hex `d` tag for a canonical Project coordinate.
#[must_use]
pub fn project_related_channels_snapshot_d(project_coordinate: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(PROJECT_RELATED_CHANNELS_SNAPSHOT_DOMAIN);
    hasher.update([0]);
    hasher.update(project_coordinate.as_bytes());
    hex::encode(hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_coordinate_is_domain_separated_and_stable() {
        let coordinate = format!("30621:{}:relay", "a".repeat(64));
        let derived = project_related_channels_snapshot_d(&coordinate);
        assert_eq!(
            derived, "85aa287457e4ad0ba0a5f9fbc9a3c3b643a388d1115b4342c7e4f934d602d995",
            "keep the relay and Desktop coordinate derivations pinned to one protocol vector"
        );
        assert_eq!(derived.len(), 64);
        assert!(derived
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase()));
        assert_eq!(derived, project_related_channels_snapshot_d(&coordinate));
        assert_ne!(
            derived,
            project_related_channels_snapshot_d(&format!("{coordinate}-other"))
        );
    }
}
