use super::*;

#[test]
fn resolves_devin_avatar() {
    let avatar_url =
        managed_agent_avatar_url("/usr/local/bin/devin").expect("Devin avatar should resolve");

    assert_eq!(avatar_url, super::super::runtime_catalog::DEVIN_AVATAR_URL);
    assert!(avatar_url.starts_with("data:image/svg+xml,"));
    assert!(avatar_url.contains("fill%3D%22white%22"));
}

#[test]
fn migrates_only_the_superseded_devin_default_avatar() {
    let migrated = normalize_managed_agent_avatar(
        "devin",
        Some(super::super::runtime_catalog::LEGACY_DEVIN_AVATAR_URL.to_string()),
    );
    assert_eq!(
        migrated.as_deref(),
        Some(super::super::runtime_catalog::DEVIN_AVATAR_URL)
    );

    let custom = Some("https://example.test/custom-devin.png".to_string());
    assert_eq!(
        normalize_managed_agent_avatar("devin", custom.clone()),
        custom
    );

    let other_runtime = Some(super::super::runtime_catalog::LEGACY_DEVIN_AVATAR_URL.to_string());
    assert_eq!(
        normalize_managed_agent_avatar("goose", other_runtime.clone()),
        other_runtime
    );
}

#[test]
fn normalizes_devin_args_to_native_acp_subcommand() {
    assert_eq!(normalize_agent_args("devin", Vec::new()), vec!["acp"]);
    assert_eq!(
        normalize_agent_args("/usr/local/bin/devin", vec!["".into()]),
        vec!["acp"]
    );
    assert_eq!(
        normalize_agent_args(
            "devin",
            vec!["acp".into(), "--agent-type".into(), "review".into()]
        ),
        vec!["acp", "--agent-type", "review"]
    );
}

#[test]
fn runtime_catalog_exposes_devin_once() {
    let devin_entries = super::super::KNOWN_ACP_RUNTIMES
        .iter()
        .filter(|runtime| runtime.id == "devin")
        .collect::<Vec<_>>();

    assert_eq!(devin_entries.len(), 1);
    let devin = devin_entries[0];
    assert_eq!(devin.label, "Devin");
    assert_eq!(devin.display_label, "Devin");
    assert_eq!(devin.sort_priority, 20);
    assert!(devin.onboarding_visible);
    assert_eq!(devin.commands, &["devin"]);
    assert_eq!(devin.default_args, &["acp"]);
    assert_eq!(devin.icon_url, "/runtime-icons/devin.svg");
    assert_eq!(devin.icon_scale, 1.1);
    assert_eq!(devin.underlying_cli, Some("devin"));
    assert_eq!(devin.skill_dir, Some(".devin/skills"));
    assert_eq!(
        devin.auth_probe_args,
        Some(&["devin", "auth", "status"][..])
    );
    assert!(devin.default_env.is_empty());
    assert_eq!(
        devin.enforced_env,
        &[
            ("BUZZ_ACP_PERMISSION_MODE", "default"),
            ("BUZZ_ACP_AUTO_APPROVE_PERMISSIONS", "false"),
            ("BUZZ_ACP_INTERACTIVE_PERMISSIONS", "true"),
            ("BUZZ_ACP_SELF_PUBLISH_COMPLETION_GRACE", "30"),
        ]
    );
    // ACP_BACKEND must stay scrubbed: inherited from the Devin IDE it flips the
    // adapter to host-supplied credentials only, and every turn then fails with
    // "ACP host has not authenticated" despite a valid `devin auth login`.
    assert_eq!(
        devin.scrub_env_vars,
        &["WINDSURF_API_KEY", "ACP_BACKEND"],
        "ambient IDE state must not redefine Devin's credential policy"
    );
    assert!(!devin.supports_acp_model_switching);
    assert!(!devin.accepts_harness_model);
}

#[test]
fn runtime_discovery_exposes_devin_entry() {
    let runtimes = super::super::discover_acp_runtimes_from(None);
    let devin = runtimes
        .iter()
        .find(|runtime| runtime.id == "devin")
        .expect("runtime discovery must project the Devin catalog entry");

    assert_eq!(devin.label, "Devin");
    assert_eq!(devin.display_label, "Devin");
    assert_eq!(devin.sort_priority, 20);
    assert!(devin.onboarding_visible);
    assert_eq!(devin.icon_url, "/runtime-icons/devin.svg");
    assert_eq!(devin.icon_scale, 1.1);
    assert_eq!(
        devin.superseded_avatar_urls,
        [super::super::runtime_catalog::LEGACY_DEVIN_AVATAR_URL]
    );
    assert!(!devin.supports_buzz_model_config);
    assert_eq!(devin.default_args, ["acp"]);
}
