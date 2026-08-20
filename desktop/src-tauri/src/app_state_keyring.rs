/// Service name for the desktop OS keyring. Debug builds default to a distinct
/// service, while standalone worktree launches may request a scoped dev service.
fn is_allowed_dev_keyring_service(service: &str) -> bool {
    service.starts_with("buzz-desktop-dev.")
        || service == "superhuman-mesh-desktop-dev"
        || service.starts_with("superhuman-mesh-desktop-dev.")
}

fn dev_keyring_service(configured: Option<String>) -> String {
    configured
        .filter(|service| is_allowed_dev_keyring_service(service))
        .unwrap_or_else(|| "buzz-desktop-dev".to_string())
}

pub(crate) fn keyring_service() -> &'static str {
    if cfg!(debug_assertions) {
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
        assert_eq!(
            dev_keyring_service(Some("superhuman-mesh-desktop-dev".to_string())),
            "superhuman-mesh-desktop-dev"
        );
        assert_eq!(
            dev_keyring_service(Some("superhuman-mesh-desktop-dev.app-rebrand".to_string())),
            "superhuman-mesh-desktop-dev.app-rebrand"
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
}
