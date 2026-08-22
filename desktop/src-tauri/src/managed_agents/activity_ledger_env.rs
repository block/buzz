use std::path::{Path, PathBuf};

const TODAY_CAPABILITY: &str = "buzz.activity-ledger.today.read/v1";
const TODAY_PATH_ENV: &str = "BUZZ_ACTIVITY_LEDGER_TODAY_PATH";
const TODAY_CAPABILITY_ENV: &str = "BUZZ_ACTIVITY_LEDGER_TODAY_CAPABILITY";
const TODAY_OWNER_PUBKEY_ENV: &str = "BUZZ_ACTIVITY_LEDGER_TODAY_OWNER_PUBKEY";
const TODAY_RELAY_URL_ENV: &str = "BUZZ_ACTIVITY_LEDGER_TODAY_RELAY_URL";

fn honey_today_env(
    persona_id: Option<&str>,
    owner_hex: Option<&str>,
    relay_url: Option<&str>,
    nest: Option<&Path>,
) -> Option<(PathBuf, &'static str, String, String)> {
    if persona_id != Some("builtin:honey") {
        return None;
    }
    let owner = owner_hex?;
    if owner.len() != 64
        || !owner
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return None;
    }
    let relay_url = buzz_core_pkg::relay::normalize_relay_url(relay_url?).ok()?;
    Some((
        crate::archive::today_snapshot::snapshot_path(nest?, owner, &relay_url),
        TODAY_CAPABILITY,
        owner.to_owned(),
        relay_url,
    ))
}

pub fn configure(
    command: &mut std::process::Command,
    record: &super::ManagedAgentRecord,
    owner_hex: Option<&str>,
    relay_url: &str,
) {
    command.env_remove(TODAY_PATH_ENV);
    command.env_remove(TODAY_CAPABILITY_ENV);
    command.env_remove(TODAY_OWNER_PUBKEY_ENV);
    command.env_remove(TODAY_RELAY_URL_ENV);
    let nest = super::nest_dir();
    if let Some((path, capability, owner_pubkey, expected_relay_url)) = honey_today_env(
        record.persona_id.as_deref(),
        owner_hex,
        Some(relay_url),
        nest.as_deref(),
    ) {
        command.env(TODAY_PATH_ENV, path);
        command.env(TODAY_CAPABILITY_ENV, capability);
        command.env(TODAY_OWNER_PUBKEY_ENV, owner_pubkey);
        command.env(TODAY_RELAY_URL_ENV, expected_relay_url);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn today_env_is_owner_scoped_and_honey_only() {
        let owner = "a".repeat(64);
        let relay = "wss://relay-a.test";
        let nest = Path::new("/private/buzz-nest");
        let (path, capability, expected_owner, expected_relay) =
            honey_today_env(Some("builtin:honey"), Some(&owner), Some(relay), Some(nest)).unwrap();
        assert_eq!(
            path,
            crate::archive::today_snapshot::snapshot_path(nest, &owner, relay)
        );
        assert_eq!(capability, TODAY_CAPABILITY);
        assert_eq!(expected_owner, owner);
        assert_eq!(expected_relay, relay);
        assert!(super::super::is_reserved_env_key(TODAY_OWNER_PUBKEY_ENV));
        assert!(super::super::is_reserved_env_key(TODAY_RELAY_URL_ENV));
        assert!(
            honey_today_env(Some("builtin:fizz"), Some(&owner), Some(relay), Some(nest)).is_none()
        );
        assert!(
            honey_today_env(Some("builtin:honey"), Some("bad"), Some(relay), Some(nest)).is_none()
        );
        assert!(honey_today_env(
            Some("builtin:honey"),
            Some(&owner),
            Some("https://bad"),
            Some(nest)
        )
        .is_none());
        assert!(honey_today_env(Some("builtin:honey"), Some(&owner), Some(relay), None).is_none());
    }

    #[test]
    fn today_env_canonicalizes_relay_before_path_and_contract() {
        let owner = "a".repeat(64);
        let nest = Path::new("/private/buzz-nest");
        let raw = honey_today_env(
            Some("builtin:honey"),
            Some(&owner),
            Some("ws://localhost:3000/"),
            Some(nest),
        )
        .unwrap();
        let canonical = honey_today_env(
            Some("builtin:honey"),
            Some(&owner),
            Some("ws://127.0.0.1:3000"),
            Some(nest),
        )
        .unwrap();
        assert_eq!(raw, canonical);
        assert_eq!(raw.3, "ws://127.0.0.1:3000");
    }
}
