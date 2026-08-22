/// Service name for the desktop OS keyring. Debug builds default to a distinct
/// service, while standalone worktree launches may request a scoped dev service.
fn dev_keyring_service(configured: Option<String>) -> String {
    configured
        .filter(|service| service.starts_with("buzz-desktop-dev."))
        .unwrap_or_else(|| "buzz-desktop-dev".to_string())
}

pub(crate) fn keyring_service() -> &'static str {
    // Candidate builds get their own keyring service so an installed test build
    // never reads or writes the released Buzz app's stored identity. Set at
    // compile time by build.rs from BUZZ_BUILD_CANDIDATE_ID.
    if let Some(candidate_id) = option_env!("BUZZ_DESKTOP_BUILD_CANDIDATE_ID") {
        static CANDIDATE_SERVICE: std::sync::OnceLock<String> = std::sync::OnceLock::new();
        CANDIDATE_SERVICE
            .get_or_init(|| format!("buzz-desktop-candidate.{candidate_id}"))
            .as_str()
    } else if cfg!(debug_assertions) {
        static DEV_SERVICE: std::sync::OnceLock<String> = std::sync::OnceLock::new();
        DEV_SERVICE
            .get_or_init(|| dev_keyring_service(std::env::var("BUZZ_DEV_KEYRING_SERVICE").ok()))
            .as_str()
    } else {
        "buzz-desktop"
    }
}

pub(super) fn migration_marker_name(service: &str, default_name: &str) -> String {
    if service == "buzz-desktop" || service == "buzz-desktop-dev" {
        default_name.to_string()
    } else {
        format!("identity.{service}.migrated")
    }
}

#[cfg(test)]
mod tests {
    use super::{dev_keyring_service, migration_marker_name};

    #[test]
    fn standalone_scope_must_remain_under_dev_service() {
        assert_eq!(
            dev_keyring_service(Some("buzz-desktop-dev.example".to_string())),
            "buzz-desktop-dev.example"
        );
        assert_eq!(
            dev_keyring_service(Some("buzz-desktop".to_string())),
            "buzz-desktop-dev"
        );
    }

    #[test]
    fn standalone_scope_uses_its_own_migration_marker() {
        assert_eq!(
            migration_marker_name("buzz-desktop", "identity.migrated"),
            "identity.migrated"
        );
        assert_eq!(
            migration_marker_name("buzz-desktop-dev", "identity.migrated"),
            "identity.migrated"
        );
        assert_eq!(
            migration_marker_name("buzz-desktop-dev.example", "identity.migrated"),
            "identity.buzz-desktop-dev.example.migrated"
        );
    }

    #[test]
    fn candidate_builds_never_share_the_release_identity() {
        // A candidate build must not read or write the released app's stored
        // identity: its keyring service and its migration marker both have to
        // stay distinct from the plain "buzz-desktop" release pair.
        let candidate = "buzz-desktop-candidate.test-release";
        assert_ne!(candidate, "buzz-desktop");
        assert_eq!(
            migration_marker_name(candidate, "identity.migrated"),
            "identity.buzz-desktop-candidate.test-release.migrated"
        );
    }
}
