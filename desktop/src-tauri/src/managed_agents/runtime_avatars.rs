use super::ManagedAgentRecord;

pub(crate) const KIRO_AVATAR_URL: &str = "https://kiro.dev/icon.svg?fe599162bb293ea0";
const LEGACY_KIRO_AVATAR_URL: &str = "https://kiro.dev/images/kiro-wordmark.png";

fn normalize_runtime_avatar(command: &str, avatar_url: &mut Option<String>) {
    if super::known_acp_runtime(command).is_some_and(|runtime| runtime.id == "kiro")
        && avatar_url.as_deref() == Some(LEGACY_KIRO_AVATAR_URL)
    {
        *avatar_url = Some(KIRO_AVATAR_URL.to_string());
    }
}

pub(crate) fn normalize_avatars(records: &mut [ManagedAgentRecord]) -> &mut [ManagedAgentRecord] {
    for record in records.iter_mut() {
        normalize_runtime_avatar(&record.agent_command, &mut record.avatar_url);
    }
    records
}

#[cfg(test)]
mod tests {
    use super::{normalize_runtime_avatar, KIRO_AVATAR_URL, LEGACY_KIRO_AVATAR_URL};

    #[test]
    fn updates_only_legacy_kiro_defaults() {
        let mut legacy_kiro = Some(LEGACY_KIRO_AVATAR_URL.to_string());
        let mut custom_kiro = Some("https://example.com/custom.png".to_string());
        let mut non_kiro = Some(LEGACY_KIRO_AVATAR_URL.to_string());

        normalize_runtime_avatar("kiro-cli", &mut legacy_kiro);
        normalize_runtime_avatar("kiro-cli", &mut custom_kiro);
        normalize_runtime_avatar("codex-acp", &mut non_kiro);

        assert_eq!(legacy_kiro.as_deref(), Some(KIRO_AVATAR_URL));
        assert_eq!(
            custom_kiro.as_deref(),
            Some("https://example.com/custom.png")
        );
        assert_eq!(non_kiro.as_deref(), Some(LEGACY_KIRO_AVATAR_URL));
    }
}
