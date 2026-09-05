//! Regression coverage for the stale-disk republish class (review item 2) and
//! the overlay-resolution ordering at each write site.
//!
//! Split out of `tests.rs` to stay inside the desktop file-size ratchet. These
//! share the parent module's fixtures (`sample_record`, `write_store`) via
//! `use super::*`, so they stay one `cargo test` away from the engine they pin.

use super::*;

/// SAMI PROBE (2b): device B holds a NEWER relay head in retention (inbound,
/// pending_sync=0). A local edit then rebuilds the payload from the STALE disk
/// record and retains it. Does the projection-equality guard or LWW stop the
/// stale fields from becoming the new relay head?
#[test]
fn sami_probe_2b_stale_disk_republish_over_newer_relay_head() {
    let dir = TempDir::new().unwrap();
    let owner_keys = nostr::Keys::generate();
    let agent_keys = nostr::Keys::generate();
    let pubkey = agent_keys.public_key().to_hex();
    let owner_hex = owner_keys.public_key().to_hex();
    let conn = open_retention_db(&dir.path().join("retention.db")).unwrap();

    // Device B's disk record: stale on two fields.
    let mut disk = sample_record(&pubkey, "stale-disk-name");
    disk.private_key_nsec = agent_keys.secret_key().to_bech32().unwrap();
    disk.system_prompt = Some("STALE disk prompt".into());
    disk.parallelism = 1;

    // Inbound relay head from device A: fresher config, gen 5, far-future
    // created_at so LWW clearly favors it.
    let mut fresh = disk.clone();
    fresh.name = "FRESH relay name".into();
    fresh.system_prompt = Some("FRESH relay prompt".into());
    fresh.parallelism = 16;
    let head_created_at = nostr::Timestamp::now().as_secs() as i64 + 10_000;
    let head_payload =
        private_payload_from_record(&fresh, &owner_hex, 5, Some("aa".repeat(32))).unwrap();
    let head_event =
        private_managed_agent::build_event(&owner_keys, &head_payload, head_created_at as u64)
            .unwrap();
    // Exactly what the inbound path writes: pending_sync = 0.
    crate::managed_agents::retention::retain_inbound_event(
        &conn,
        &RetainedEvent {
            kind: KIND_PRIVATE_MANAGED_AGENT,
            pubkey: owner_hex.clone(),
            d_tag: pubkey.clone(),
            content: head_event.content.clone(),
            created_at: head_created_at,
            raw_event: head_event.as_json(),
            pending_sync: false,
        },
    )
    .unwrap();

    // A local edit on device B: update_managed_agent's tail — retain the DISK
    // record. (`update_managed_agent` passes the just-saved disk record.)
    let changed = retain_agent_record(&conn, &owner_keys, &disk).unwrap();

    let row = get_retained_event(&conn, KIND_PRIVATE_MANAGED_AGENT, &owner_hex, &pubkey)
        .unwrap()
        .unwrap();
    let (_, republished) = private_managed_agent::validate_and_decrypt(
        &nostr::Event::from_json(&row.raw_event).unwrap(),
        &owner_keys,
    )
    .unwrap();

    // DEFECT: every stale disk field became the new relay head.
    assert!(changed, "retain reported a change");
    assert_eq!(republished.config.name, "stale-disk-name");
    assert_eq!(
        republished.config.system_prompt,
        Some("STALE disk prompt".into())
    );
    assert_eq!(republished.config.parallelism, Some(1));
    // ...and it WINS LWW: monotonic_created_at bumped past the fresher head.
    assert!(
        row.created_at > head_created_at,
        "stale republish must outrank the fresher head for the defect to matter"
    );
    // ...and it is a VALIDLY CHAINED successor (gen 5 -> 6, prev = head id),
    // so no peer can distinguish it from a legitimate edit.
    assert_eq!(republished.generation, 6);
    assert_eq!(
        republished.previous_event_id,
        Some(head_event.id.to_hex()),
        "stale event chains cleanly off the head it clobbers"
    );
    // ...and it is queued for publish, not merely local.
    assert!(
        get_pending_sync(&conn)
            .unwrap()
            .iter()
            .any(|event| event.kind == KIND_PRIVATE_MANAGED_AGENT),
        "the stale 30179 is enqueued for relay publish"
    );

    // POSITIVE CONTROL: the guard this probe claims is bypassed DOES fire when
    // the input matches the head — proving the probe observes a real bypass and
    // not a guard that never no-ops. Run against a PRISTINE head (a second db),
    // because the stale write above already replaced the head in `conn`. Compare
    // the private ROW, not `retain_agent_record`'s bool: that bool is
    // `public_changed || private_changed`, so the fresh db's absent 30177 row
    // would mask the private no-op.
    let control_conn = open_retention_db(&dir.path().join("control.db")).unwrap();
    crate::managed_agents::retention::retain_inbound_event(
        &control_conn,
        &RetainedEvent {
            kind: KIND_PRIVATE_MANAGED_AGENT,
            pubkey: owner_hex.clone(),
            d_tag: pubkey.clone(),
            content: head_event.content.clone(),
            created_at: head_created_at,
            raw_event: head_event.as_json(),
            pending_sync: false,
        },
    )
    .unwrap();
    retain_agent_record(&control_conn, &owner_keys, &fresh).unwrap();
    let control_row = get_retained_event(
        &control_conn,
        KIND_PRIVATE_MANAGED_AGENT,
        &owner_hex,
        &pubkey,
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        control_row.raw_event,
        head_event.as_json(),
        "control: retaining the RESOLVED (relay-fresh) record against the same \
         head leaves the head untouched — so the defect above is the stale \
         input, not a guard that never fires"
    );
}

/// SAMI PROBE (item 2 cost): would the CHEAP fix — resolve the overlay once
/// inside `retain_managed_agent_pending` instead of at each call site — be
/// correct? Simulates that fix at the EDIT site: user edits one field on disk,
/// helper resolves the overlay on top, then retains.
#[test]
fn sami_probe_resolve_in_helper_would_discard_user_edits() {
    use crate::managed_agents::private_config_overlay::{PrivateConfigOverlay, PrivateConfigPatch};

    let owner_keys = nostr::Keys::generate();
    let agent_keys = nostr::Keys::generate();
    let pubkey = agent_keys.public_key().to_hex();
    let owner_hex = owner_keys.public_key().to_hex();

    // Relay head the overlay is following: parallelism 16, relay prompt.
    let mut relay = sample_record(&pubkey, "relay-name");
    relay.private_key_nsec = agent_keys.secret_key().to_bech32().unwrap();
    relay.system_prompt = Some("relay prompt".into());
    relay.parallelism = 16;
    let head_payload = private_payload_from_record(&relay, &owner_hex, 1, None).unwrap();
    let mut overlay = PrivateConfigOverlay::default();
    overlay.insert_patch(PrivateConfigPatch::from_payload(head_payload).unwrap());

    // The user edits ONE field locally: parallelism 16 -> 2. This is the
    // just-saved disk record `update_managed_agent` passes to the helper.
    let mut edited = relay.clone();
    edited.parallelism = 2;

    // The "cheap fix": helper resolves the overlay onto the record it was
    // handed, then retains that.
    let resolved = overlay.resolve_local_record(&edited);

    assert_eq!(
        resolved.parallelism, 16,
        "the cheap one-place fix SILENTLY DISCARDS the user's edit: \
         parallelism went back to the relay's 16, not the edited 2"
    );
    // Positive control: with no patch for this agent the edit survives, so the
    // discard above is the overlay winning, not a broken fixture.
    let empty = PrivateConfigOverlay::default();
    assert_eq!(
        empty.resolve_local_record(&edited).parallelism,
        2,
        "control: without an overlay patch the edit survives"
    );
}

/// SAMI PROBE (settles Eva's contested third site, `personas/update.rs:214`):
/// on a device following a NEWER relay head, does a persona RENAME republish
/// the non-name config fields from stale disk?
///
/// Eva traced that `private_payload_from_record` serializes every config field
/// from the disk record, so a name-only mutation should still clobber
/// system_prompt / parallelism / env_vars. I traced it as scoped-out ("folding
/// the overlay would fight the intended write"). Measured here rather than
/// argued: the rename mutates ONLY `name`/`display_name` (pinned faithful by
/// `rename_helper_mutates_only_name_and_display_name` in
/// `commands/personas/update/name_propagation_tests.rs`), then the retain fires
/// exactly as `update.rs:214` fires it.
#[test]
fn sami_probe_rename_republishes_nonname_fields_from_stale_disk() {
    let dir = TempDir::new().unwrap();
    let owner_keys = nostr::Keys::generate();
    let agent_keys = nostr::Keys::generate();
    let pubkey = agent_keys.public_key().to_hex();
    let owner_hex = owner_keys.public_key().to_hex();
    let conn = open_retention_db(&dir.path().join("retention.db")).unwrap();

    // Device B's disk record, stale on the NON-name fields. Its `name` still
    // equals the old persona display_name, which is what makes the rename
    // propagate to it at all.
    let mut disk = sample_record(&pubkey, "Paul");
    disk.display_name = Some("Paul".into());
    disk.private_key_nsec = agent_keys.secret_key().to_bech32().unwrap();
    disk.system_prompt = Some("STALE disk prompt".into());
    disk.parallelism = 1;
    disk.env_vars
        .insert("STALE_KEY".into(), "stale-value".into());

    // Device A's newer relay head: same agent, fresher non-name config.
    let mut fresh = disk.clone();
    fresh.system_prompt = Some("FRESH relay prompt".into());
    fresh.parallelism = 16;
    fresh.env_vars.clear();
    fresh
        .env_vars
        .insert("FRESH_KEY".into(), "fresh-value".into());
    let head_created_at = nostr::Timestamp::now().as_secs() as i64 + 10_000;
    let head_payload =
        private_payload_from_record(&fresh, &owner_hex, 5, Some("aa".repeat(32))).unwrap();
    let head_event =
        private_managed_agent::build_event(&owner_keys, &head_payload, head_created_at as u64)
            .unwrap();
    crate::managed_agents::retention::retain_inbound_event(
        &conn,
        &RetainedEvent {
            kind: KIND_PRIVATE_MANAGED_AGENT,
            pubkey: owner_hex.clone(),
            d_tag: pubkey.clone(),
            content: head_event.content.clone(),
            created_at: head_created_at,
            raw_event: head_event.as_json(),
            pending_sync: false,
        },
    )
    .unwrap();

    // The rename: `propagate_persona_name_rename` mutates name + display_name
    // and NOTHING else, then `update.rs:214` retains that disk record.
    let mut renamed = disk.clone();
    renamed.name = "Paul Atreides".into();
    renamed.display_name = Some("Paul Atreides".into());
    retain_agent_record(&conn, &owner_keys, &renamed).unwrap();

    let row = get_retained_event(&conn, KIND_PRIVATE_MANAGED_AGENT, &owner_hex, &pubkey)
        .unwrap()
        .unwrap();
    let (_, republished) = private_managed_agent::validate_and_decrypt(
        &nostr::Event::from_json(&row.raw_event).unwrap(),
        &owner_keys,
    )
    .unwrap();

    // The intended write DID land.
    assert_eq!(republished.config.name, "Paul Atreides");
    // EVA IS RIGHT: the non-name fields came back from STALE DISK, not the head.
    assert_eq!(
        republished.config.system_prompt,
        Some("STALE disk prompt".into()),
        "rename republished the stale disk prompt over the fresher relay head"
    );
    assert_eq!(republished.config.parallelism, Some(1));
    assert_eq!(
        republished
            .config
            .env_vars
            .get("STALE_KEY")
            .map(String::as_str),
        Some("stale-value"),
        "stale env var resurrected"
    );
    assert!(
        !republished.config.env_vars.contains_key("FRESH_KEY"),
        "the head's env var was DROPPED, so this is a replace not a merge"
    );
    // ...and it outranks the fresher head, chained cleanly: same clobber class
    // as `agent_models.rs:867`.
    assert!(row.created_at > head_created_at);
    assert_eq!(republished.generation, 6);
    assert_eq!(republished.previous_event_id, Some(head_event.id.to_hex()));

    // POSITIVE CONTROL: a rename applied to the RESOLVED (relay-fresh) record
    // preserves every non-name field, so the defect above is the stale input,
    // not something inherent to renaming. Pristine head in a second db, and
    // compare the private ROW bytes (retain_agent_record's bool is
    // public_changed || private_changed, so a fresh db's absent 30177 row makes
    // it true regardless of the private outcome).
    let control_conn = open_retention_db(&dir.path().join("control.db")).unwrap();
    crate::managed_agents::retention::retain_inbound_event(
        &control_conn,
        &RetainedEvent {
            kind: KIND_PRIVATE_MANAGED_AGENT,
            pubkey: owner_hex.clone(),
            d_tag: pubkey.clone(),
            content: head_event.content.clone(),
            created_at: head_created_at,
            raw_event: head_event.as_json(),
            pending_sync: false,
        },
    )
    .unwrap();
    let mut resolved_then_renamed = fresh.clone();
    resolved_then_renamed.name = "Paul Atreides".into();
    resolved_then_renamed.display_name = Some("Paul Atreides".into());
    retain_agent_record(&control_conn, &owner_keys, &resolved_then_renamed).unwrap();
    let control_row = get_retained_event(
        &control_conn,
        KIND_PRIVATE_MANAGED_AGENT,
        &owner_hex,
        &pubkey,
    )
    .unwrap()
    .unwrap();
    let (_, control) = private_managed_agent::validate_and_decrypt(
        &nostr::Event::from_json(&control_row.raw_event).unwrap(),
        &owner_keys,
    )
    .unwrap();
    assert_eq!(
        control.config.name, "Paul Atreides",
        "control: rename landed"
    );
    assert_eq!(
        control.config.system_prompt,
        Some("FRESH relay prompt".into()),
        "control: resolve-then-rename preserves the head's prompt"
    );
    assert_eq!(control.config.parallelism, Some(16));
    assert_eq!(
        control.config.env_vars.get("FRESH_KEY").map(String::as_str),
        Some("fresh-value"),
        "control: resolve-then-rename preserves the head's env vars"
    );
}

/// SAMI FIX VERIFICATION for the rename site (`personas/update.rs:214`):
/// the shipped shape is resolve-overlay → re-apply name/display_name → retain.
/// Assert that on the SAME fixture that produces the clobber above, this
/// ordering republishes the head's non-name fields while still landing the
/// rename. Mirrors the production expression exactly (`resolved.name` and
/// `resolved.display_name` re-copied from the renamed disk record).
#[test]
fn sami_fix_rename_over_resolved_record_preserves_relay_fields() {
    use crate::managed_agents::private_config_overlay::{PrivateConfigOverlay, PrivateConfigPatch};

    let dir = TempDir::new().unwrap();
    let owner_keys = nostr::Keys::generate();
    let agent_keys = nostr::Keys::generate();
    let pubkey = agent_keys.public_key().to_hex();
    let owner_hex = owner_keys.public_key().to_hex();
    let conn = open_retention_db(&dir.path().join("retention.db")).unwrap();

    let mut disk = sample_record(&pubkey, "Paul");
    disk.display_name = Some("Paul".into());
    disk.private_key_nsec = agent_keys.secret_key().to_bech32().unwrap();
    disk.system_prompt = Some("STALE disk prompt".into());
    disk.parallelism = 1;
    disk.env_vars
        .insert("STALE_KEY".into(), "stale-value".into());

    let mut fresh = disk.clone();
    fresh.system_prompt = Some("FRESH relay prompt".into());
    fresh.parallelism = 16;
    fresh.env_vars.clear();
    fresh
        .env_vars
        .insert("FRESH_KEY".into(), "fresh-value".into());
    let head_created_at = nostr::Timestamp::now().as_secs() as i64 + 10_000;
    let head_payload =
        private_payload_from_record(&fresh, &owner_hex, 5, Some("aa".repeat(32))).unwrap();
    let head_event =
        private_managed_agent::build_event(&owner_keys, &head_payload, head_created_at as u64)
            .unwrap();
    crate::managed_agents::retention::retain_inbound_event(
        &conn,
        &RetainedEvent {
            kind: KIND_PRIVATE_MANAGED_AGENT,
            pubkey: owner_hex.clone(),
            d_tag: pubkey.clone(),
            content: head_event.content.clone(),
            created_at: head_created_at,
            raw_event: head_event.as_json(),
            pending_sync: false,
        },
    )
    .unwrap();

    // The overlay this device is following (what boot hydration installs).
    let mut overlay = PrivateConfigOverlay::default();
    overlay.insert_patch(PrivateConfigPatch::from_payload(head_payload).unwrap());

    // Production's renamed disk record...
    let mut renamed = disk.clone();
    renamed.name = "Paul Atreides".into();
    renamed.display_name = Some("Paul Atreides".into());
    // ...then the FIX: resolve, re-apply the rename, retain.
    let mut resolved = overlay.resolve_local_record(&renamed);
    resolved.name.clone_from(&renamed.name);
    resolved.display_name.clone_from(&renamed.display_name);
    retain_agent_record(&conn, &owner_keys, &resolved).unwrap();

    let row = get_retained_event(&conn, KIND_PRIVATE_MANAGED_AGENT, &owner_hex, &pubkey)
        .unwrap()
        .unwrap();
    let (_, published) = private_managed_agent::validate_and_decrypt(
        &nostr::Event::from_json(&row.raw_event).unwrap(),
        &owner_keys,
    )
    .unwrap();

    // The rename still lands (the fix must not eat the intended write).
    assert_eq!(published.config.name, "Paul Atreides");
    // ...and every non-name field is now the RELAY head's, not stale disk.
    assert_eq!(
        published.config.system_prompt,
        Some("FRESH relay prompt".into()),
        "fix: the head's prompt survives the rename"
    );
    assert_eq!(published.config.parallelism, Some(16));
    assert_eq!(
        published
            .config
            .env_vars
            .get("FRESH_KEY")
            .map(String::as_str),
        Some("fresh-value"),
        "fix: the head's env var survives"
    );
    assert!(
        !published.config.env_vars.contains_key("STALE_KEY"),
        "fix: the stale disk env var is NOT resurrected"
    );

    // NEGATIVE CONTROL: with no overlay patch (e.g. pre-hydration, or an agent
    // the relay has never described) the same code path must fall through to
    // the disk record unchanged — the fix must not blank config on a device
    // that legitimately has no relay head to follow.
    let empty = PrivateConfigOverlay::default();
    let mut fallback = empty.resolve_local_record(&renamed);
    fallback.name.clone_from(&renamed.name);
    fallback.display_name.clone_from(&renamed.display_name);
    assert_eq!(
        fallback.system_prompt,
        Some("STALE disk prompt".into()),
        "control: with no patch the disk value is preserved, not cleared"
    );
    assert_eq!(fallback.parallelism, 1);
    assert_eq!(
        fallback.name, "Paul Atreides",
        "control: rename still lands"
    );
}

/// EVA PROBE (item 2, third-party audit of `agents.rs:276`): the
/// persona-snapshot re-apply in `start_local_agent_pairs_with_preflight`
/// loads the DISK record, calls `apply_persona_snapshot` (which overwrites
/// only the definition quad: system_prompt/model/provider/runtime), saves,
/// and retains. On a device following a NEWER relay head, do the NON-quad
/// fields (parallelism, env overrides, name...) republish from stale disk?
#[test]
fn eva_probe_pair_start_snapshot_reapply_republishes_stale_nonquad_fields() {
    let dir = TempDir::new().unwrap();
    let owner_keys = nostr::Keys::generate();
    let agent_keys = nostr::Keys::generate();
    let pubkey = agent_keys.public_key().to_hex();
    let owner_hex = owner_keys.public_key().to_hex();
    let conn = open_retention_db(&dir.path().join("retention.db")).unwrap();

    // Device B's disk record: stale on non-quad fields.
    let mut disk = sample_record(&pubkey, "stale-disk-name");
    disk.private_key_nsec = agent_keys.secret_key().to_bech32().unwrap();
    disk.parallelism = 1;
    disk.env_vars = BTreeMap::from([("STALE_KEY".to_string(), "stale".to_string())]);
    disk.persona_id = Some("test-persona".to_string());

    // Fresher relay head from device A: gen 5, future created_at.
    let mut fresh = disk.clone();
    fresh.name = "FRESH relay name".into();
    fresh.parallelism = 16;
    fresh.env_vars = BTreeMap::from([("FRESH_KEY".to_string(), "fresh".to_string())]);
    let head_created_at = nostr::Timestamp::now().as_secs() as i64 + 10_000;
    let head_payload =
        private_payload_from_record(&fresh, &owner_hex, 5, Some("aa".repeat(32))).unwrap();
    let head_event =
        private_managed_agent::build_event(&owner_keys, &head_payload, head_created_at as u64)
            .unwrap();
    crate::managed_agents::retention::retain_inbound_event(
        &conn,
        &crate::managed_agents::retention::RetainedEvent {
            kind: KIND_PRIVATE_MANAGED_AGENT,
            pubkey: owner_hex.clone(),
            d_tag: pubkey.clone(),
            content: head_event.content.clone(),
            created_at: head_created_at,
            raw_event: head_event.as_json(),
            pending_sync: false,
        },
    )
    .unwrap();

    // The site's exact sequence (agents.rs:268-277): persona snapshot applied
    // to the DISK record, then retain. The snapshot only touches the quad.
    let persona = crate::managed_agents::AgentDefinition {
        id: "test-persona".to_string(),
        display_name: "Test Persona".to_string(),
        avatar_url: None,
        description: None,
        system_prompt: "Persona prompt.".to_string(),
        runtime: Some("goose".to_string()),
        model: Some("claude-opus-4".to_string()),
        provider: Some("anthropic".to_string()),
        name_pool: Vec::new(),
        is_builtin: false,
        is_active: true,
        shared: false,
        source_team: None,
        source_team_persona_slug: None,
        catalog_source: None,
        team_catalog_source: None,
        env_vars: BTreeMap::new(),
        respond_to: None,
        respond_to_allowlist: Vec::new(),
        parallelism: None,
        created_at: "2025-01-01T00:00:00Z".to_string(),
        updated_at: "2025-01-01T00:00:00Z".to_string(),
    };
    let mut site_record = disk.clone();
    crate::managed_agents::persona_events::apply_persona_snapshot(&mut site_record, &persona);
    let changed = retain_agent_record(&conn, &owner_keys, &site_record).unwrap();

    let row = get_retained_event(&conn, KIND_PRIVATE_MANAGED_AGENT, &owner_hex, &pubkey)
        .unwrap()
        .unwrap();
    let (_, republished) = private_managed_agent::validate_and_decrypt(
        &nostr::Event::from_json(&row.raw_event).unwrap(),
        &owner_keys,
    )
    .unwrap();

    // DEFECT (if these pass): non-quad stale fields became the new head.
    assert!(changed, "retain reported a change");
    assert_eq!(republished.config.name, "stale-disk-name");
    assert_eq!(republished.config.parallelism, Some(1));
    assert!(
        republished.config.env_vars.contains_key("STALE_KEY"),
        "stale env override resurrected"
    );
    assert!(
        !republished.config.env_vars.contains_key("FRESH_KEY"),
        "head's env dropped — replace, not merge"
    );
    assert!(
        row.created_at > head_created_at,
        "stale write outranks head"
    );
    assert_eq!(republished.generation, 6, "validly chained gen bump");
    assert_eq!(
        republished.previous_event_id,
        Some(head_event.id.to_hex()),
        "chains cleanly off the head it clobbers"
    );
}

/// SAMI FIX VERIFICATION for `agents.rs:276` (red-first probe above is Eva's).
/// The shipped shape is resolve-overlay → `apply_persona_snapshot` → retain.
/// Both halves of that ordering are asserted, because each direction has its
/// own failure mode:
///   * resolve BEFORE the snapshot → non-quad fields come from the relay head
///     (fixes the stale republish), and
///   * snapshot AFTER the resolve → the definition quad stays
///     definition-authoritative rather than being clobbered by the overlay.
///
/// A test asserting only the first half would pass with the calls in the wrong
/// order, since the overlay also carries system_prompt/model/provider/runtime.
#[test]
fn sami_fix_pair_start_resolve_then_snapshot_keeps_quad_definition_authoritative() {
    use crate::managed_agents::private_config_overlay::{PrivateConfigOverlay, PrivateConfigPatch};

    let dir = TempDir::new().unwrap();
    let owner_keys = nostr::Keys::generate();
    let agent_keys = nostr::Keys::generate();
    let pubkey = agent_keys.public_key().to_hex();
    let owner_hex = owner_keys.public_key().to_hex();
    let conn = open_retention_db(&dir.path().join("retention.db")).unwrap();

    let mut disk = sample_record(&pubkey, "stale-disk-name");
    disk.private_key_nsec = agent_keys.secret_key().to_bech32().unwrap();
    disk.parallelism = 1;
    disk.env_vars = BTreeMap::from([("STALE_KEY".to_string(), "stale".to_string())]);
    disk.persona_id = Some("test-persona".to_string());

    // Relay head: fresher non-quad fields AND a quad the persona disagrees
    // with, so the two halves of the ordering are separable.
    let mut fresh = disk.clone();
    fresh.name = "FRESH relay name".into();
    fresh.parallelism = 16;
    fresh.env_vars = BTreeMap::from([("FRESH_KEY".to_string(), "fresh".to_string())]);
    fresh.system_prompt = Some("RELAY prompt (must lose to the persona)".into());
    fresh.model = Some("relay-model".into());
    let head_created_at = nostr::Timestamp::now().as_secs() as i64 + 10_000;
    let head_payload =
        private_payload_from_record(&fresh, &owner_hex, 5, Some("aa".repeat(32))).unwrap();
    let head_event =
        private_managed_agent::build_event(&owner_keys, &head_payload, head_created_at as u64)
            .unwrap();
    crate::managed_agents::retention::retain_inbound_event(
        &conn,
        &crate::managed_agents::retention::RetainedEvent {
            kind: KIND_PRIVATE_MANAGED_AGENT,
            pubkey: owner_hex.clone(),
            d_tag: pubkey.clone(),
            content: head_event.content.clone(),
            created_at: head_created_at,
            raw_event: head_event.as_json(),
            pending_sync: false,
        },
    )
    .unwrap();

    let mut overlay = PrivateConfigOverlay::default();
    overlay.insert_patch(PrivateConfigPatch::from_payload(head_payload).unwrap());

    let persona = crate::managed_agents::AgentDefinition {
        id: "test-persona".to_string(),
        display_name: "Test Persona".to_string(),
        avatar_url: None,
        description: None,
        system_prompt: "PERSONA prompt.".to_string(),
        runtime: Some("goose".to_string()),
        model: Some("persona-model".to_string()),
        provider: Some("anthropic".to_string()),
        name_pool: Vec::new(),
        is_builtin: false,
        is_active: true,
        shared: false,
        source_team: None,
        source_team_persona_slug: None,
        catalog_source: None,
        team_catalog_source: None,
        env_vars: BTreeMap::new(),
        respond_to: None,
        respond_to_allowlist: Vec::new(),
        parallelism: None,
        created_at: "2025-01-01T00:00:00Z".to_string(),
        updated_at: "2025-01-01T00:00:00Z".to_string(),
    };

    // The FIXED site sequence: resolve, then snapshot, then retain.
    let mut site_record = overlay.resolve_local_record(&disk);
    crate::managed_agents::persona_events::apply_persona_snapshot(&mut site_record, &persona);
    retain_agent_record(&conn, &owner_keys, &site_record).unwrap();

    let row = get_retained_event(&conn, KIND_PRIVATE_MANAGED_AGENT, &owner_hex, &pubkey)
        .unwrap()
        .unwrap();
    let (_, published) = private_managed_agent::validate_and_decrypt(
        &nostr::Event::from_json(&row.raw_event).unwrap(),
        &owner_keys,
    )
    .unwrap();

    // HALF 1 — resolve-before: non-quad fields are the RELAY head's, not stale disk.
    assert_eq!(published.config.name, "FRESH relay name");
    assert_eq!(published.config.parallelism, Some(16));
    assert!(
        published.config.env_vars.contains_key("FRESH_KEY"),
        "head's env override survives"
    );
    assert!(
        !published.config.env_vars.contains_key("STALE_KEY"),
        "stale disk env override is NOT resurrected"
    );

    // HALF 2 — snapshot-after: the definition quad is the PERSONA's, not the
    // overlay's. This is the assertion that fails if the two calls are swapped.
    assert_eq!(
        published.config.system_prompt,
        Some("PERSONA prompt.".into()),
        "definition quad stays definition-authoritative after the resolve"
    );
    assert_eq!(published.config.model, Some("persona-model".into()));

    // NEGATIVE CONTROL: with no overlay patch the site must fall through to the
    // disk record — the fix must not blank config on a device that has no relay
    // head to follow.
    let empty = PrivateConfigOverlay::default();
    let mut fallback = empty.resolve_local_record(&disk);
    crate::managed_agents::persona_events::apply_persona_snapshot(&mut fallback, &persona);
    assert_eq!(
        fallback.parallelism, 1,
        "control: with no patch the disk value is preserved, not cleared"
    );
    assert!(
        fallback.env_vars.contains_key("STALE_KEY"),
        "control: disk env override preserved when there is no relay head"
    );
    assert_eq!(
        fallback.system_prompt,
        Some("PERSONA prompt.".into()),
        "control: quad still definition-authoritative"
    );
}

// ── Review item 5: boot reconcile as a stale-republish site ─────────────────
//
// `reconcile_agents_in_dir_at` reads `managed-agents.json` raw and cannot
// resolve the overlay: `hydrate_private_config_overlay` runs AFTER this leg
// (`event_sync.rs:19-20`) and reads the rows this leg writes. On a following
// device, disk is stale by construction, so rebuilding the 30179 from disk is
// the item-2 clobber with no user action at all.

/// Builds the follower fixture: a stale disk store plus a NEWER inbound 30179
/// head (`pending_sync = 0`, far-future `created_at`). Returns the head event.
fn seed_follower_with_newer_head(
    dir: &TempDir,
    owner_keys: &nostr::Keys,
    disk: &ManagedAgentRecord,
    fresh: &ManagedAgentRecord,
    generation: u64,
    created_at: i64,
) -> nostr::Event {
    let owner_hex = owner_keys.public_key().to_hex();
    let payload =
        private_payload_from_record(fresh, &owner_hex, generation, Some("aa".repeat(32))).unwrap();
    let event =
        private_managed_agent::build_event(owner_keys, &payload, created_at as u64).unwrap();
    let conn = open_retention_db(&dir.path().join("retention.db")).unwrap();
    crate::managed_agents::retention::retain_inbound_event(
        &conn,
        &RetainedEvent {
            kind: KIND_PRIVATE_MANAGED_AGENT,
            pubkey: owner_hex,
            d_tag: disk.pubkey.clone(),
            content: event.content.clone(),
            created_at,
            raw_event: event.as_json(),
            pending_sync: false,
        },
    )
    .unwrap();
    event
}

fn retained_private_row(dir: &TempDir, owner_keys: &nostr::Keys, pubkey: &str) -> RetainedEvent {
    let conn = open_retention_db(&dir.path().join("retention.db")).unwrap();
    get_retained_event(
        &conn,
        KIND_PRIVATE_MANAGED_AGENT,
        &owner_keys.public_key().to_hex(),
        pubkey,
    )
    .unwrap()
    .unwrap()
}

/// Item 5 FIX: boot reconcile must leave an existing 30179 head alone.
///
/// Red-first against `retain_agent_record` at boot: the probe measured
/// `name="stale-disk-name"`, `parallelism=Some(1)`, gen 5→6, `prev` = the
/// clobbered head, `created_at` = head+1 (so it wins LWW), `pending_sync=true`
/// — every stale disk field published over device A's newer config at launch,
/// with no user action.
#[test]
fn boot_reconcile_leaves_existing_private_head_intact() {
    let dir = TempDir::new().unwrap();
    let owner_keys = nostr::Keys::generate();
    let agent_keys = nostr::Keys::generate();
    let pubkey = agent_keys.public_key().to_hex();

    let mut disk = sample_record(&pubkey, "stale-disk-name");
    disk.private_key_nsec = agent_keys.secret_key().to_bech32().unwrap();
    disk.system_prompt = Some("STALE disk prompt".into());
    disk.parallelism = 1;
    disk.env_vars = BTreeMap::from([("STALE_KEY".to_string(), "stale".to_string())]);
    write_store(&dir, &[disk.clone()]);

    let mut fresh = disk.clone();
    fresh.name = "FRESH relay name".into();
    fresh.system_prompt = Some("FRESH relay prompt".into());
    fresh.parallelism = 16;
    fresh.env_vars = BTreeMap::from([("FRESH_KEY".to_string(), "fresh".to_string())]);
    let head_created_at = nostr::Timestamp::now().as_secs() as i64 + 10_000;
    let head_event =
        seed_follower_with_newer_head(&dir, &owner_keys, &disk, &fresh, 5, head_created_at);

    // BOOT. No user action.
    reconcile_agents_in_dir(dir.path(), &owner_keys).unwrap();

    let row = retained_private_row(&dir, &owner_keys, &pubkey);
    assert_eq!(
        row.raw_event,
        head_event.as_json(),
        "boot reconcile must not rebuild the 30179 from stale disk over an \
         existing head — byte-identical, so no gen bump and no re-encryption"
    );
    // Not merely equal-by-content: nothing was queued for publish either.
    let conn = open_retention_db(&dir.path().join("retention.db")).unwrap();
    assert!(
        get_pending_sync(&conn)
            .unwrap()
            .iter()
            .all(|event| event.kind != KIND_PRIVATE_MANAGED_AGENT),
        "no stale 30179 enqueued for relay publish"
    );
}

/// The requirement item 1 exists to serve, preserved: an agent with NO retained
/// 30179 head still publishes its first one at boot. Without this arm the fix
/// above is satisfied by never publishing a 30179 at boot at all — which is the
/// item-1 bug restored.
#[test]
fn boot_reconcile_still_publishes_first_private_config() {
    let dir = TempDir::new().unwrap();
    let owner_keys = nostr::Keys::generate();
    let agent_keys = nostr::Keys::generate();
    let pubkey = agent_keys.public_key().to_hex();

    let mut record = sample_record(&pubkey, "untouched-agent");
    record.private_key_nsec = agent_keys.secret_key().to_bech32().unwrap();
    record.parallelism = 7;
    write_store(&dir, &[record]);

    assert_eq!(reconcile_agents_in_dir(dir.path(), &owner_keys).unwrap(), 1);

    let row = retained_private_row(&dir, &owner_keys, &pubkey);
    let (_, payload) = private_managed_agent::validate_and_decrypt(
        &nostr::Event::from_json(&row.raw_event).unwrap(),
        &owner_keys,
    )
    .unwrap();
    assert_eq!(payload.generation, 1);
    assert_eq!(payload.previous_event_id, None);
    assert_eq!(payload.config.parallelism, Some(7));
    assert!(row.pending_sync, "first 30179 is queued for publish");
}

/// The 30177 leg must be untouched by the 30179 gate: an edited record whose
/// PUBLIC projection changed still republishes at boot even though a private
/// head exists. This is what keeps the upgrade republish waves
/// (`slimming_republish_wave_is_one_time`) working, and it fails if the gate is
/// written at the wrong level (skipping the whole record instead of the 30179).
#[test]
fn boot_reconcile_still_republishes_public_projection_with_private_head_present() {
    let dir = TempDir::new().unwrap();
    let owner_keys = nostr::Keys::generate();
    let agent_keys = nostr::Keys::generate();
    let pubkey = agent_keys.public_key().to_hex();

    let mut disk = sample_record(&pubkey, "public-name-v2");
    disk.private_key_nsec = agent_keys.secret_key().to_bech32().unwrap();
    write_store(&dir, &[disk.clone()]);

    let mut fresh = disk.clone();
    fresh.parallelism = 16;
    let head_created_at = nostr::Timestamp::now().as_secs() as i64 + 10_000;
    seed_follower_with_newer_head(&dir, &owner_keys, &disk, &fresh, 5, head_created_at);

    assert_eq!(
        reconcile_agents_in_dir(dir.path(), &owner_keys).unwrap(),
        1,
        "the 30177 identity projection still reconciles at boot"
    );
    let conn = open_retention_db(&dir.path().join("retention.db")).unwrap();
    let public_row = get_retained_event(
        &conn,
        KIND_MANAGED_AGENT,
        &owner_keys.public_key().to_hex(),
        &pubkey,
    )
    .unwrap()
    .unwrap();
    assert!(public_row.content.contains("public-name-v2"));
    assert!(public_row.pending_sync);
}

/// WRONG-FIX PROBE (permanent): the tempting alternative is to resolve the
/// overlay inside boot reconcile (swapping the hydrate/reconcile order in
/// `run_event_sync`). It is wrong for the same reason the centralized resolve
/// was wrong at the edit site — but with a worse blast radius, because boot
/// touches EVERY agent rather than the one being edited.
///
/// A local edit made while the relay was unreachable lives on disk AND in a
/// `pending_sync` 30179 that never flushed. Resolving disk through an overlay
/// hydrated from the last-known head would rebuild the payload from that older
/// head and discard the edit — at launch, silently, for every agent.
#[test]
fn probe_resolving_overlay_at_boot_would_discard_unflushed_local_edits() {
    use crate::managed_agents::private_config_overlay::{PrivateConfigOverlay, PrivateConfigPatch};

    let owner_keys = nostr::Keys::generate();
    let agent_keys = nostr::Keys::generate();
    let pubkey = agent_keys.public_key().to_hex();
    let owner_hex = owner_keys.public_key().to_hex();

    // The last head this device saw, which is what a boot-time overlay would
    // hydrate from.
    let mut head = sample_record(&pubkey, "head-name");
    head.private_key_nsec = agent_keys.secret_key().to_bech32().unwrap();
    head.parallelism = 16;
    let head_payload = private_payload_from_record(&head, &owner_hex, 3, None).unwrap();
    let mut overlay = PrivateConfigOverlay::default();
    overlay.insert_patch(PrivateConfigPatch::from_payload(head_payload).unwrap());

    // The user's offline edit, on disk and not yet flushed to the relay.
    let mut disk = head.clone();
    disk.parallelism = 2;

    assert_eq!(
        overlay.resolve_local_record(&disk).parallelism,
        16,
        "resolving at boot DISCARDS the unflushed local edit (2 -> 16); this is \
         why the fix is a head-presence gate, not a resolve"
    );
    // Positive control: with no patch the edit survives, so the discard above
    // is the overlay winning rather than a broken fixture.
    assert_eq!(
        PrivateConfigOverlay::default()
            .resolve_local_record(&disk)
            .parallelism,
        2,
        "control: without an overlay patch the offline edit survives"
    );
}
