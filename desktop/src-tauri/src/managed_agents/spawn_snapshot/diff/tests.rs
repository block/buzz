use super::*;
use std::collections::{BTreeMap, BTreeSet};

const SECRET: &str = "sk-live-SENTINEL-0000";
const RELAY_WITH_TOKEN: &str = "wss://relay.example/ws?token=SENTINEL";

/// Every field populated, so mutating one to `None` is a real change and the
/// coverage guard below sees the full serialized key set.
fn base() -> SpawnConfigSnapshot {
    SpawnConfigSnapshot {
        acp_command: "buzz-acp".into(),
        command: "goose".into(),
        args: vec!["--mode".into(), "acp".into()],
        mcp_command: "goose-mcp".into(),
        env: BTreeMap::from([
            ("OPENAI_API_KEY".to_string(), SECRET.to_string()),
            ("BUZZ_LOG".to_string(), "info".to_string()),
        ]),
        relay_url: "wss://relay.example".into(),
        team_instructions: Some("Team says hello.".into()),
        system_prompt: Some("You are a test agent.".into()),
        model: Some("gpt-5".into()),
        provider: Some("openai".into()),
        session_title: Some("Fizz".into()),
        auth_tag: Some("tag-abcdefgh".into()),
        respond_to: "owner-only".into(),
        respond_to_allowlist: Some(vec!["a".repeat(64)]),
        idle_timeout_seconds: Some(600),
        max_turn_duration_seconds: Some(7200),
        parallelism: 1,
    }
}

fn fields(entries: &[RestartDiffEntry]) -> Vec<&str> {
    entries.iter().map(|entry| entry.field.as_str()).collect()
}

fn change_at<'a>(entries: &'a [RestartDiffEntry], field: &str) -> &'a RestartChange {
    &entries
        .iter()
        .find(|entry| entry.field == field)
        .unwrap_or_else(|| panic!("no entry for {field}; got {:?}", fields(entries)))
        .change
}

/// One mutation per snapshot field, keyed by the diff path it must produce.
type Mutation = (&'static str, fn(&mut SpawnConfigSnapshot));

fn mutations() -> Vec<Mutation> {
    vec![
        ("acp_command", |s| s.acp_command = "other-acp".into()),
        ("command", |s| s.command = "claude".into()),
        ("args", |s| s.args = vec!["--other".into()]),
        ("mcp_command", |s| s.mcp_command = String::new()),
        ("env.OPENAI_API_KEY", |s| {
            s.env
                .insert("OPENAI_API_KEY".into(), "sk-live-rotated-9999".into());
        }),
        ("relay_url", |s| s.relay_url = "wss://other.example".into()),
        ("team_instructions", |s| s.team_instructions = None),
        ("system_prompt", |s| s.system_prompt = None),
        ("model", |s| s.model = None),
        ("provider", |s| s.provider = None),
        ("session_title", |s| s.session_title = None),
        ("auth_tag", |s| s.auth_tag = None),
        ("respond_to", |s| s.respond_to = "anyone".into()),
        ("respond_to_allowlist", |s| s.respond_to_allowlist = None),
        ("idle_timeout_seconds", |s| s.idle_timeout_seconds = None),
        ("max_turn_duration_seconds", |s| {
            s.max_turn_duration_seconds = None
        }),
        ("parallelism", |s| s.parallelism = 8),
    ]
}

#[test]
fn every_field_mutation_drifts_the_canonical_value_and_names_that_field() {
    for (field, mutate) in mutations() {
        let before = base();
        let mut after = base();
        mutate(&mut after);

        assert_ne!(
            before.canonical(),
            after.canonical(),
            "{field}: mutation must move the canonical value the badge compares"
        );
        assert_eq!(
            fields(&diff(&before, &after)),
            vec![field],
            "{field}: mutation must produce exactly that field's entry"
        );
        // Both directions: `None -> Some` must be as visible as `Some -> None`.
        assert_eq!(
            fields(&diff(&after, &before)),
            vec![field],
            "{field}: reverse mutation must be equally visible"
        );
    }
}

#[test]
fn mutation_table_covers_every_serialized_field() {
    let covered: BTreeSet<&str> = mutations()
        .iter()
        .map(|(field, _)| field.split('.').next().expect("non-empty path"))
        .collect();
    let canonical = base().canonical();
    let serialized: BTreeSet<&str> = canonical
        .as_object()
        .expect("snapshot serializes as an object")
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        covered, serialized,
        "add a mutation row for every new snapshot field"
    );
}

#[test]
fn identical_snapshots_produce_no_entries() {
    assert!(diff(&base(), &base()).is_empty());
}

#[test]
fn env_map_insertion_order_is_not_drift() {
    let mut reordered = base();
    reordered.env = base().env.into_iter().rev().collect();
    assert!(diff(&base(), &reordered).is_empty());
}

#[test]
fn entries_are_ordered_lexicographically_by_path() {
    let mut after = base();
    after.parallelism = 4;
    after.command = "claude".into();
    after.env.insert("ZZZ".into(), "1".into());
    after.env.insert("AAA".into(), "1".into());
    assert_eq!(
        fields(&diff(&base(), &after)),
        vec!["command", "env.AAA", "env.ZZZ", "parallelism"]
    );
}

// ── map membership vs. nullable struct fields ────────────────────────────

#[test]
fn env_key_insertion_is_added_without_a_payload() {
    let mut after = base();
    after.env.insert("NEW_KEY".into(), SECRET.into());
    assert_eq!(
        change_at(&diff(&base(), &after), "env.NEW_KEY"),
        &RestartChange::Added
    );
}

#[test]
fn env_key_removal_is_removed_without_a_payload() {
    let mut after = base();
    after.env.remove("BUZZ_LOG");
    assert_eq!(
        change_at(&diff(&base(), &after), "env.BUZZ_LOG"),
        &RestartChange::Removed
    );
}

#[test]
fn cleared_nullable_field_stays_a_value_change_not_a_removal() {
    let mut after = base();
    after.model = None;
    assert_eq!(
        change_at(&diff(&base(), &after), "model"),
        &RestartChange::Value {
            before: Value::String("gpt-5".into()),
            after: Value::Null,
        }
    );
}

#[test]
fn array_field_changes_as_one_atomic_leaf() {
    let mut after = base();
    after.respond_to_allowlist = Some(vec!["b".repeat(64)]);
    let entries = diff(&base(), &after);
    assert_eq!(fields(&entries), vec!["respond_to_allowlist"]);
    assert!(matches!(
        change_at(&entries, "respond_to_allowlist"),
        RestartChange::Value { .. }
    ));
}

#[test]
fn allowlisted_env_key_shows_plain_value() {
    // BUZZ_AGENT_THINKING_EFFORT is on the safe-to-reveal allowlist — the user
    // must be able to see actual enum values like "medium → high".
    let mut before = base();
    before
        .env
        .insert("BUZZ_AGENT_THINKING_EFFORT".into(), "medium".into());
    let mut after = before.clone();
    after
        .env
        .insert("BUZZ_AGENT_THINKING_EFFORT".into(), "high".into());
    assert_eq!(
        change_at(&diff(&before, &after), "env.BUZZ_AGENT_THINKING_EFFORT"),
        &RestartChange::Value {
            before: Value::String("medium".into()),
            after: Value::String("high".into()),
        },
        "allowlisted env key must render plain before/after values"
    );
}

#[test]
fn allowlisted_env_key_is_case_insensitive() {
    // The allowlist comparison is case-insensitive; lowercase path must also
    // render plain.
    let mut before = base();
    before
        .env
        .insert("buzz_agent_provider".into(), "anthropic".into());
    let mut after = before.clone();
    after
        .env
        .insert("buzz_agent_provider".into(), "openai".into());
    assert_eq!(
        change_at(&diff(&before, &after), "env.buzz_agent_provider"),
        &RestartChange::Value {
            before: Value::String("anthropic".into()),
            after: Value::String("openai".into()),
        },
        "allowlist match must be case-insensitive"
    );
}

#[test]
fn non_allowlisted_env_key_stays_masked() {
    // A key not in the allowlist must remain masked regardless of its name.
    let mut after = base();
    after
        .env
        .insert("SOME_API_KEY".into(), "sk-live-rotated-9999".into());
    // SOME_API_KEY is a new key — starts as Added, not a value change.
    // Use an existing env key (OPENAI_API_KEY is in base()) to test masking.
    let mut before = base();
    before
        .env
        .insert("OPENAI_API_KEY".into(), "sk-live-SENTINEL-0000".into());
    let mut after2 = before.clone();
    after2
        .env
        .insert("OPENAI_API_KEY".into(), "sk-live-rotated-9999".into());
    assert!(
        matches!(
            change_at(&diff(&before, &after2), "env.OPENAI_API_KEY"),
            RestartChange::Masked { .. }
        ),
        "non-allowlisted env key must stay masked"
    );
}

// ── masking policy ───────────────────────────────────────────────────────

#[test]
fn env_value_longer_than_eight_chars_shows_a_four_char_suffix() {
    let mut after = base();
    after
        .env
        .insert("OPENAI_API_KEY".into(), "abcdefghi".into());
    assert_eq!(
        change_at(&diff(&base(), &after), "env.OPENAI_API_KEY"),
        &RestartChange::Masked {
            before: Some("••••0000".into()),
            after: Some("••••fghi".into()),
        }
    );
}

#[test]
fn env_value_of_exactly_eight_chars_shows_no_suffix() {
    let mut before = base();
    before
        .env
        .insert("OPENAI_API_KEY".into(), "abcdefgh".into());
    let mut after = before.clone();
    after.env.insert("OPENAI_API_KEY".into(), "12345678".into());
    assert_eq!(
        change_at(&diff(&before, &after), "env.OPENAI_API_KEY"),
        &RestartChange::Masked {
            before: Some("••••".into()),
            after: Some("••••".into()),
        }
    );
}

#[test]
fn masking_counts_characters_not_bytes() {
    // Nine two-byte characters: a byte-based length test would call this
    // short, and byte slicing the last four would split a code point.
    let mut before = base();
    before.env.insert("K".into(), "áéíóúàèìò".into());
    let mut after = before.clone();
    after.env.insert("K".into(), "áéíóúàèìá".into());
    assert_eq!(
        change_at(&diff(&before, &after), "env.K"),
        &RestartChange::Masked {
            before: Some("••••àèìò".into()),
            after: Some("••••àèìá".into()),
        }
    );
}

#[test]
fn args_are_masked_without_any_suffix() {
    let mut after = base();
    after.args = vec![format!("--token={SECRET}")];
    assert_eq!(
        change_at(&diff(&base(), &after), "args"),
        &RestartChange::Masked {
            before: Some("••••".into()),
            after: Some("••••".into()),
        }
    );
}

#[test]
fn relay_url_is_masked_without_any_suffix() {
    let mut after = base();
    after.relay_url = RELAY_WITH_TOKEN.into();
    assert_eq!(
        change_at(&diff(&base(), &after), "relay_url"),
        &RestartChange::Masked {
            before: Some("••••".into()),
            after: Some("••••".into()),
        }
    );
}

#[test]
fn auth_tag_is_masked_with_a_suffix() {
    let mut after = base();
    after.auth_tag = Some("tag-ijklmnop".into());
    assert_eq!(
        change_at(&diff(&base(), &after), "auth_tag"),
        &RestartChange::Masked {
            before: Some("••••efgh".into()),
            after: Some("••••mnop".into()),
        }
    );
}

#[test]
fn large_text_fields_report_character_counts_only() {
    let mut after = base();
    after.system_prompt = Some("Longer replacement prompt.".into());
    after.team_instructions = None;
    let entries = diff(&base(), &after);
    assert_eq!(
        change_at(&entries, "system_prompt"),
        &RestartChange::Text {
            before_chars: Some("You are a test agent.".chars().count()),
            after_chars: Some("Longer replacement prompt.".chars().count()),
        }
    );
    assert_eq!(
        change_at(&entries, "team_instructions"),
        &RestartChange::Text {
            before_chars: Some("Team says hello.".chars().count()),
            after_chars: None,
        }
    );
}

// ── secrecy sentinels ────────────────────────────────────────────────────

/// A snapshot whose every secret-bearing leaf carries a sentinel.
fn seeded_with_sentinels() -> SpawnConfigSnapshot {
    let mut snapshot = base();
    snapshot.relay_url = RELAY_WITH_TOKEN.into();
    snapshot.args = vec![format!("--token={SECRET}")];
    snapshot.auth_tag = Some(SECRET.into());
    snapshot.env.insert("OPENAI_API_KEY".into(), SECRET.into());
    snapshot
}

/// Every sentinel-bearing leaf changed, plus an added key, so each masking
/// arm has to redact a real value.
fn rotated_sentinels() -> SpawnConfigSnapshot {
    let mut snapshot = seeded_with_sentinels();
    snapshot.relay_url = format!("{RELAY_WITH_TOKEN}2");
    snapshot.args = vec![format!("--token={SECRET}2")];
    snapshot.auth_tag = Some(format!("{SECRET}2"));
    snapshot
        .env
        .insert("OPENAI_API_KEY".into(), format!("{SECRET}2"));
    snapshot.env.insert("ADDED".into(), SECRET.into());
    snapshot
}

#[test]
fn no_sentinel_reaches_the_serialized_diff() {
    let entries = diff(&seeded_with_sentinels(), &rotated_sentinels());
    assert!(!entries.is_empty(), "fixture must actually drift");
    let wire = serde_json::to_string(&entries).expect("diff serializes");
    assert!(!wire.contains("SENTINEL"), "diff leaked a secret: {wire}");
    assert!(
        !wire.contains("token="),
        "diff leaked a query token: {wire}"
    );
}

#[test]
fn no_sentinel_reaches_snapshot_debug_output() {
    let rendered = format!("{:?}", seeded_with_sentinels());
    assert!(!rendered.contains("SENTINEL"), "Debug leaked: {rendered}");
    assert!(!rendered.contains("token="), "Debug leaked: {rendered}");
    // Large text is summarized rather than dumped.
    assert!(!rendered.contains("You are a test agent."));
    // Non-secret leaves stay legible, or the log line is useless.
    assert!(rendered.contains("goose"));
}

#[test]
fn no_sentinel_reaches_the_owning_process_debug_output() {
    // `ManagedAgentProcess` derives `Debug` and delegates to the snapshot's
    // manual impl — this pins that the derive can never become the leak path.
    #[cfg(unix)]
    let program = "/usr/bin/true";
    #[cfg(windows)]
    let program = "true";
    let child = std::process::Command::new(program)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("spawn placeholder child");
    let process = crate::managed_agents::ManagedAgentProcess {
        child,
        log_path: std::path::PathBuf::new(),
        spawn_config: seeded_with_sentinels(),
        setup_mode: false,
        adapter_availability: None,
        start_nonce: "test-nonce".to_string(),
        #[cfg(windows)]
        job: None,
    };
    let rendered = format!("{process:?}");
    assert!(
        !rendered.contains("SENTINEL"),
        "process Debug leaked a secret"
    );
    assert!(
        !rendered.contains("token="),
        "process Debug leaked a query token"
    );
}

// ── B1: the eligible vector is the single source of the badge ────────────

fn eligible(
    orphaned: bool,
    stamped: &SpawnConfigSnapshot,
    current: &SpawnConfigSnapshot,
    stamped_availability: Option<AcpAvailabilityStatus>,
    current_availability: Option<AcpAvailabilityStatus>,
) -> (bool, Vec<RestartDiffEntry>) {
    let entries = eligible_restart_diff(
        orphaned,
        Some(TrackedSpawnState {
            stamped,
            current,
            stamped_availability: stamped_availability.as_ref(),
            current_availability,
        }),
    );
    (!entries.is_empty(), entries)
}

#[test]
fn no_drift_yields_no_badge_and_no_entries() {
    let (needs_restart, entries) = eligible(false, &base(), &base(), None, None);
    assert!(!needs_restart);
    assert!(entries.is_empty());
}

#[test]
fn snapshot_drift_yields_a_badge_and_that_entry() {
    let mut current = base();
    current.model = Some("claude-4".into());
    let (needs_restart, entries) = eligible(false, &base(), &current, None, None);
    assert!(needs_restart);
    assert_eq!(fields(&entries), vec!["model"]);
}

#[test]
fn availability_drift_alone_yields_a_badge_and_its_synthetic_entry() {
    let (needs_restart, entries) = eligible(
        false,
        &base(),
        &base(),
        Some(AcpAvailabilityStatus::Available),
        Some(AcpAvailabilityStatus::AdapterOutdated),
    );
    assert!(needs_restart);
    assert_eq!(fields(&entries), vec!["adapter_availability"]);
    assert_eq!(
        change_at(&entries, "adapter_availability"),
        &RestartChange::Value {
            before: Value::String("available".into()),
            after: Value::String("adapter_outdated".into()),
        }
    );
}

#[test]
fn orphan_with_snapshot_drift_yields_no_badge_and_no_entries() {
    let mut current = base();
    current.model = Some("claude-4".into());
    let (needs_restart, entries) = eligible(true, &base(), &current, None, None);
    assert!(!needs_restart);
    assert!(entries.is_empty());
}

#[test]
fn orphan_with_availability_drift_yields_no_badge_and_no_entries() {
    let (needs_restart, entries) = eligible(
        true,
        &base(),
        &base(),
        Some(AcpAvailabilityStatus::Available),
        Some(AcpAvailabilityStatus::AdapterOutdated),
    );
    assert!(!needs_restart);
    assert!(entries.is_empty());
}

#[test]
fn unstamped_availability_is_not_drift() {
    // A runtime without a version gate stamps no availability; comparing that
    // absence against a freshly cached value must not invent a badge.
    let (needs_restart, entries) = eligible(
        false,
        &base(),
        &base(),
        None,
        Some(AcpAvailabilityStatus::AdapterOutdated),
    );
    assert!(!needs_restart);
    assert!(entries.is_empty());
}

#[test]
fn unstamped_agent_yields_no_badge_and_no_entries() {
    // A `runtime_pid`-adopted process — and any agent this workspace tracks no
    // live pair for — has no `ManagedAgentProcess`, so no spawn config was ever
    // stamped. With nothing to compare against there is no drift to report, and
    // the badge derives from that emptiness. Distinct from the case above,
    // where a real pair IS tracked and only its availability stamp is absent.
    for orphaned in [false, true] {
        let entries = eligible_restart_diff(orphaned, None);
        let needs_restart = !entries.is_empty();
        assert!(
            entries.is_empty(),
            "unstamped agent (orphaned={orphaned}) must report no changed fields"
        );
        assert!(
            !needs_restart,
            "unstamped agent (orphaned={orphaned}) must not light the badge"
        );
    }
}
