//! User-group validation helpers.

/// Maximum number of members in one user group or membership command.
pub const MAX_USER_GROUP_MEMBERS: usize = 256;

/// Maximum number of default channels attached to one user group.
pub const MAX_USER_GROUP_DEFAULT_CHANNELS: usize = 32;

/// Returns whether `handle` matches `^[a-z0-9][a-z0-9_-]{1,31}$`.
///
/// Handles are ASCII-only, contain between 2 and 32 characters, and must
/// start with a lowercase letter or digit.
pub fn is_valid_group_handle(handle: &str) -> bool {
    let bytes = handle.as_bytes();
    if !(2..=32).contains(&bytes.len()) {
        return false;
    }

    (bytes[0].is_ascii_lowercase() || bytes[0].is_ascii_digit())
        && bytes[1..].iter().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(*byte, b'_' | b'-')
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_valid_group_handles() {
        for handle in ["ab", "ios-team", "team_42", "0platform", &"a".repeat(32)] {
            assert!(
                is_valid_group_handle(handle),
                "expected valid handle: {handle}"
            );
        }
    }

    #[test]
    fn rejects_invalid_group_handles() {
        for handle in [
            "",
            "a",
            "-team",
            "_team",
            "Team",
            "team.name",
            "team name",
            "tëam",
            &"a".repeat(33),
        ] {
            assert!(
                !is_valid_group_handle(handle),
                "expected invalid handle: {handle}"
            );
        }
    }
}
