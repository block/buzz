use std::path::{Path, PathBuf};

const TODAY_CAPABILITY: &str = "buzz.activity-ledger.today.read/v1";
const TODAY_PATH_ENV: &str = "BUZZ_ACTIVITY_LEDGER_TODAY_PATH";
const TODAY_CAPABILITY_ENV: &str = "BUZZ_ACTIVITY_LEDGER_TODAY_CAPABILITY";

fn honey_today_env(
    persona_id: Option<&str>,
    owner_hex: Option<&str>,
    nest: Option<&Path>,
) -> Option<(PathBuf, &'static str)> {
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
    Some((
        nest?
            .join("archive")
            .join(format!("activity-ledger-today-{owner}.json")),
        TODAY_CAPABILITY,
    ))
}

pub fn configure(
    command: &mut std::process::Command,
    record: &super::ManagedAgentRecord,
    owner_hex: Option<&str>,
) {
    command.env_remove(TODAY_PATH_ENV);
    command.env_remove(TODAY_CAPABILITY_ENV);
    let nest = super::nest_dir();
    if let Some((path, capability)) =
        honey_today_env(record.persona_id.as_deref(), owner_hex, nest.as_deref())
    {
        command.env(TODAY_PATH_ENV, path);
        command.env(TODAY_CAPABILITY_ENV, capability);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn today_env_is_owner_scoped_and_honey_only() {
        let owner = "a".repeat(64);
        let nest = Path::new("/private/buzz-nest");
        let (path, capability) =
            honey_today_env(Some("builtin:honey"), Some(&owner), Some(nest)).unwrap();
        assert_eq!(
            path,
            nest.join("archive")
                .join(format!("activity-ledger-today-{owner}.json"))
        );
        assert_eq!(capability, TODAY_CAPABILITY);
        assert!(honey_today_env(Some("builtin:fizz"), Some(&owner), Some(nest)).is_none());
        assert!(honey_today_env(Some("builtin:honey"), Some("bad"), Some(nest)).is_none());
        assert!(honey_today_env(Some("builtin:honey"), Some(&owner), None).is_none());
    }
}
