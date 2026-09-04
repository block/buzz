//! Tests for the dynamic AGENTS.md section renderer, the managed-section
//! upsert, and the regeneration gate. Split from `tests.rs` to keep
//! each test file under the repository's per-file line ratchet.

use super::*;
use std::collections::HashSet;

/// Relay URL passed to render calls. Since the roster no longer filters on
/// `relay_url`, this is only echoed into the Workspace footer.
const TEST_RELAY: &str = "ws://example.com:3000";

fn make_persona(id: &str, display_name: &str) -> AgentDefinition {
    AgentDefinition {
        description: None,
        id: id.to_string(),
        display_name: display_name.to_string(),
        avatar_url: None,
        system_prompt: String::new(),
        runtime: None,
        model: None,
        provider: None,
        name_pool: vec![],
        is_builtin: false,
        is_active: true,
        shared: false,
        source_team: None,
        source_team_persona_slug: None,
        catalog_source: None,
        team_catalog_source: None,
        env_vars: std::collections::BTreeMap::new(),
        respond_to: None,
        respond_to_allowlist: Vec::new(),
        parallelism: None,
        created_at: String::new(),
        updated_at: String::new(),
    }
}

fn make_agent(name: &str, persona_id: Option<&str>) -> ManagedAgentRecord {
    ManagedAgentRecord {
        description: None,
        pubkey: String::new(),
        name: name.to_string(),
        persona_id: persona_id.map(|s| s.to_string()),
        private_key_nsec: String::new(),
        auth_tag: None,
        relay_url: TEST_RELAY.to_string(),
        avatar_url: None,
        acp_command: String::new(),
        agent_command: String::new(),
        agent_command_override: None,
        agent_args: vec![],
        mcp_command: String::new(),
        turn_timeout_seconds: 0,
        idle_timeout_seconds: None,
        max_turn_duration_seconds: None,
        parallelism: 1,
        system_prompt: None,
        model: None,
        provider: None,
        persona_source_version: None,
        start_on_app_launch: false,
        auto_restart_on_config_change: true,
        runtime_pid: None,
        backend: BackendKind::default(),
        backend_agent_id: None,
        provider_policy_pending: false,
        provider_binary_path: None,
        team_id: None,
        persona_team_dir: None,
        persona_name_in_team: None,
        created_at: String::new(),
        updated_at: String::new(),
        last_started_at: None,
        last_stopped_at: None,
        last_exit_code: None,
        last_error: None,
        last_error_code: None,
        respond_to: RespondTo::default(),
        respond_to_allowlist: vec![],
        env_vars: std::collections::BTreeMap::new(),
        display_name: None,
        slug: None,
        runtime: None,
        name_pool: Vec::new(),
        is_builtin: false,
        is_active: true,
        shared: false,
        source_team: None,
        source_team_persona_slug: None,
        catalog_source: None,
        team_catalog_source: None,
        definition_respond_to: None,
        definition_respond_to_allowlist: Vec::new(),
        definition_parallelism: None,
        relay_mesh: None,
        effort_level: None,
    }
}

#[test]
fn test_render_dynamic_section_with_agents() {
    let personas = vec![make_persona("p1", "Builder")];
    let agents = vec![make_agent("Kit", Some("p1"))];
    let output = render_dynamic_section(&personas, &agents, &HashSet::new(), TEST_RELAY);
    assert!(output.contains("| Kit | Builder | @Kit |"));
    assert!(output.contains("| Name | Persona | How to address |"));
    assert!(output.contains("## Workspace"));
}

#[test]
fn test_render_dynamic_section_empty() {
    let output = render_dynamic_section(&[], &[], &HashSet::new(), TEST_RELAY);
    assert!(output.contains("No agents deployed yet"));
}

#[test]
fn test_render_dynamic_section_agent_no_persona() {
    let personas = vec![make_persona("p1", "Builder")];
    let agents = vec![make_agent("Scout", Some("nonexistent"))];
    let output = render_dynamic_section(&personas, &agents, &HashSet::new(), TEST_RELAY);
    assert!(output.contains("| Scout | — | @Scout |"));
}

#[test]
fn test_render_excludes_archived_agents() {
    let personas = vec![make_persona("p1", "Builder")];
    let mut live = make_agent("Live", Some("p1"));
    live.pubkey = "aa".repeat(32);
    let mut gone = make_agent("Archived", Some("p1"));
    gone.pubkey = "bb".repeat(32);
    let archived: HashSet<String> = [gone.pubkey.clone()].into_iter().collect();

    let output = render_dynamic_section(&personas, &[live, gone], &archived, TEST_RELAY);

    assert!(output.contains("| Live | Builder | @Live |"));
    assert!(
        !output.contains("Archived"),
        "archived agent must not render"
    );
}

#[test]
fn test_render_archived_match_is_case_insensitive() {
    let personas = vec![make_persona("p1", "Builder")];
    let mut gone = make_agent("Archived", Some("p1"));
    gone.pubkey = "AB".repeat(32); // uppercase hex in the record
                                   // Snapshot pubkeys are lowercased by `archived_pubkeys_from_snapshot`.
    let archived: HashSet<String> = ["ab".repeat(32)].into_iter().collect();

    let output = render_dynamic_section(&personas, &[gone], &archived, TEST_RELAY);

    assert!(
        output.contains("No agents deployed yet"),
        "all-archived roster renders the empty placeholder"
    );
}

#[test]
fn test_render_empty_archived_set_renders_all() {
    let personas = vec![make_persona("p1", "Builder")];
    let mut a = make_agent("Kit", Some("p1"));
    a.pubkey = "cc".repeat(32);
    // Fail-open: an empty snapshot (relay unreachable) must render everyone.
    let output = render_dynamic_section(&personas, &[a], &HashSet::new(), TEST_RELAY);
    assert!(output.contains("| Kit | Builder | @Kit |"));
}

#[test]
fn test_render_keeps_agent_with_legacy_foreign_relay_pin() {
    // `relay_url` is a legacy creation-era field that `effective_agent_relay_url()`
    // deliberately ignores — every agent is eligible on every community. A record
    // whose stored pin points at a now-defunct relay must still render on the
    // active workspace; only identity-archive removes an agent.
    let personas = vec![make_persona("p1", "Builder")];
    let here = make_agent("Local", Some("p1"));
    let mut elsewhere = make_agent("Foreign", Some("p1"));
    elsewhere.relay_url = "wss://defunct.communities.buzz.xyz".to_string();

    let output = render_dynamic_section(&personas, &[here, elsewhere], &HashSet::new(), TEST_RELAY);

    assert!(output.contains("| Local | Builder | @Local |"));
    assert!(
        output.contains("| Foreign | Builder | @Foreign |"),
        "a legacy foreign relay pin must not hide an agent — the pin is ignored"
    );
}

#[test]
fn test_render_keeps_snapshot_imported_agent_with_empty_relay_pin() {
    // Snapshot-imported records store `relay_url: ""` by design; they resolve
    // to the workspace relay at runtime. Such an agent must appear on the active
    // workspace, not be hidden by an empty pin.
    let personas = vec![make_persona("p1", "Builder")];
    let mut imported = make_agent("Imported", Some("p1"));
    imported.relay_url = String::new();

    let output = render_dynamic_section(&personas, &[imported], &HashSet::new(), TEST_RELAY);

    assert!(
        output.contains("| Imported | Builder | @Imported |"),
        "an empty relay_url (snapshot-import shape) must still render"
    );
}

#[test]
fn test_upsert_managed_section_with_markers() {
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("AGENTS.md");
    fs::write(
            &file,
            "# Header\n\nsome content\n\n<!-- BEGIN BUZZ MANAGED — regenerated automatically, do not edit below -->\nold section\n<!-- END BUZZ MANAGED -->\n\nafter\n",
        )
        .unwrap();

    upsert_managed_section(&file, "new section").unwrap();

    let result = fs::read_to_string(&file).unwrap();
    assert!(result.contains("<!-- BEGIN BUZZ MANAGED"));
    assert!(result.contains("<!-- END BUZZ MANAGED -->"));
    assert!(result.contains("new section"));
    assert!(!result.contains("old section"));
    assert!(result.contains("# Header"));
    assert!(result.contains("some content"));
    assert!(result.contains("after"));
}

#[test]
fn test_upsert_managed_section_without_markers() {
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("AGENTS.md");
    fs::write(&file, "# Header\n\nexisting content\n").unwrap();

    upsert_managed_section(&file, "injected section").unwrap();

    let result = fs::read_to_string(&file).unwrap();
    assert!(result.contains("# Header"));
    assert!(result.contains("existing content"));
    assert!(result.contains("<!-- BEGIN BUZZ MANAGED"));
    assert!(result.contains("<!-- END BUZZ MANAGED -->"));
    assert!(result.contains("injected section"));
    let begin_pos = result.find("<!-- BEGIN BUZZ MANAGED").unwrap();
    let header_pos = result.find("# Header").unwrap();
    assert!(
        header_pos < begin_pos,
        "original content should precede the managed section"
    );
}

#[test]
fn test_upsert_managed_section_no_tmp_leftover() {
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("AGENTS.md");
    fs::write(&file, "# Header\n").unwrap();

    upsert_managed_section(&file, "content").unwrap();

    // Verify no stray temp files in the directory
    let entries: Vec<_> = fs::read_dir(tmp.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .collect();
    assert_eq!(
        entries.len(),
        1,
        "only AGENTS.md should remain, no temp files"
    );
    assert_eq!(entries[0].file_name(), "AGENTS.md");
}

#[test]
fn test_upsert_end_before_begin() {
    // An END marker that precedes a BEGIN marker forms no valid ordered pair.
    // find_managed_markers returns None (BEGIN found, but no END after it),
    // so the orphan BEGIN line is stripped and a new block is appended.
    // The stray END line and content between END and BEGIN remain in the file
    // because strip_orphan_begin_marker only removes the BEGIN line itself.
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("AGENTS.md");
    fs::write(
            &file,
            "# Header\n\n<!-- END BUZZ MANAGED -->\nsome middle content\n<!-- BEGIN BUZZ MANAGED — regenerated automatically, do not edit below -->\nold section\n",
        )
        .unwrap();

    upsert_managed_section(&file, "new section").unwrap();

    let result = fs::read_to_string(&file).unwrap();

    assert!(result.contains("# Header"), "original header must survive");
    assert!(
        result.contains("new section"),
        "new content must be present"
    );
    assert!(
        result.contains("some middle content"),
        "content between markers must survive"
    );

    // Exactly one BEGIN marker in the output (the orphan was stripped, new one appended).
    assert_eq!(
        result.matches(BEGIN_MARKER).count(),
        1,
        "exactly one BEGIN marker after orphan cleanup"
    );

    // The single BEGIN marker must have a matching END marker after it.
    let begin_pos = result
        .find(BEGIN_MARKER)
        .expect("BEGIN marker must be present");
    let end_pos = result[begin_pos..].find(END_MARKER).map(|p| begin_pos + p);
    assert!(
        end_pos.is_some(),
        "an END marker must appear after the appended BEGIN marker"
    );
}

#[test]
fn test_upsert_begin_only_no_end() {
    // A file with BEGIN but no END has an orphan marker.
    // find_managed_markers returns None (no END found after BEGIN),
    // so strip_orphan_begin_marker removes the BEGIN line.
    // Content that followed the orphan BEGIN is preserved (only the marker line is stripped,
    // not the body that came after it).
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("AGENTS.md");
    fs::write(
            &file,
            "# Header\n\nsome content\n\n<!-- BEGIN BUZZ MANAGED — regenerated automatically, do not edit below -->\norphaned section without end marker\n",
        )
        .unwrap();

    upsert_managed_section(&file, "fresh section").unwrap();

    let result = fs::read_to_string(&file).unwrap();

    assert!(result.contains("# Header"), "original header must survive");
    assert!(
        result.contains("some content"),
        "original body must survive"
    );
    assert!(
        result.contains("fresh section"),
        "new content must be present"
    );

    let begin_pos = result
        .find(BEGIN_MARKER)
        .expect("BEGIN marker must be present");
    let end_pos = result.find(END_MARKER).expect("END marker must be present");
    assert!(
        begin_pos < end_pos,
        "the appended BEGIN marker must precede the appended END marker"
    );

    // Exactly one BEGIN marker after orphan cleanup.
    assert_eq!(
        result.matches(BEGIN_MARKER).count(),
        1,
        "exactly one BEGIN marker after orphan cleanup"
    );
}

#[test]
fn test_upsert_duplicate_markers() {
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("AGENTS.md");
    fs::write(
            &file,
            "# Header\n\n<!-- BEGIN BUZZ MANAGED — regenerated automatically, do not edit below -->\nfirst block\n<!-- END BUZZ MANAGED -->\n\nbetween blocks\n\n<!-- BEGIN BUZZ MANAGED — regenerated automatically, do not edit below -->\nsecond block\n<!-- END BUZZ MANAGED -->\n",
        )
        .unwrap();

    upsert_managed_section(&file, "replaced").unwrap();

    let result = fs::read_to_string(&file).unwrap();

    assert!(
        result.contains("replaced"),
        "replacement content must be present"
    );
    assert!(
        !result.contains("first block"),
        "first block must be replaced"
    );
    assert!(
        result.contains("second block"),
        "second pair content must survive"
    );
    assert!(
        result.contains("between blocks"),
        "text between pairs must survive"
    );
}

#[test]
fn test_upsert_marker_in_code_block() {
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("AGENTS.md");
    // Indented by 4 spaces — not at column 0, so should NOT match as a real marker.
    fs::write(
        &file,
        "# Header\n\n    <!-- BEGIN BUZZ MANAGED — some indented marker -->\n\nReal content here\n",
    )
    .unwrap();

    upsert_managed_section(&file, "appended content").unwrap();

    let result = fs::read_to_string(&file).unwrap();

    assert!(
        result.contains("    <!-- BEGIN BUZZ MANAGED — some indented marker -->"),
        "indented marker inside code block must be preserved verbatim"
    );
    assert!(
        result.contains("appended content"),
        "new content must be appended"
    );
    assert!(
        result.contains("Real content here"),
        "existing body must survive"
    );

    // The real markers appended at the end must be at line-start (column 0).
    let begin_pos = result
        .find("<!-- BEGIN BUZZ MANAGED — regenerated")
        .expect("regenerated BEGIN marker must be present");
    assert!(
        begin_pos == 0 || result.as_bytes()[begin_pos - 1] == b'\n',
        "appended BEGIN marker must be at line start"
    );
}

#[test]
fn test_render_pipe_in_agent_name() {
    let personas = vec![make_persona("p1", "Builder")];
    let agents = vec![make_agent("Kit|Pro", Some("p1"))];
    let output = render_dynamic_section(&personas, &agents, &HashSet::new(), TEST_RELAY);

    assert!(
        output.contains("Kit\\|Pro"),
        "pipe in agent name must be escaped as \\|"
    );
    // An unescaped bare `|` immediately adjacent to "Kit|Pro" would break table parsing.
    assert!(
        !output.contains("| Kit|Pro |"),
        "unescaped pipe in agent name must not appear as a cell boundary"
    );

    // The row must start and end with `|` and the escaped name and address must appear.
    let kit_row = output
        .lines()
        .find(|l| l.contains("Kit\\|Pro"))
        .expect("Kit\\|Pro row must be present");
    assert!(kit_row.starts_with('|'), "row must start with |");
    assert!(kit_row.ends_with('|'), "row must end with |");
    assert!(
        kit_row.contains("@Kit\\|Pro"),
        "address cell must use escaped name"
    );
}

#[test]
fn test_render_newline_in_persona_name() {
    let personas = vec![make_persona("p1", "Builder\nExpert")];
    let agents = vec![make_agent("Scout", Some("p1"))];
    let output = render_dynamic_section(&personas, &agents, &HashSet::new(), TEST_RELAY);

    assert!(
        output.contains("Builder Expert"),
        "newline in persona display_name must be replaced with a space"
    );

    // The table row for Scout must be a single line (no embedded newline).
    let scout_row = output
        .lines()
        .find(|l| l.contains("Scout"))
        .expect("Scout row must be present");
    assert!(
        scout_row.contains("Builder Expert"),
        "persona name with newline replaced by space must appear on the Scout row"
    );
}

#[test]
fn test_upsert_idempotent() {
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("AGENTS.md");
    fs::write(
            &file,
            "# Header\n\n<!-- BEGIN BUZZ MANAGED — regenerated automatically, do not edit below -->\nexisting section\n<!-- END BUZZ MANAGED -->\n",
        )
        .unwrap();

    upsert_managed_section(&file, "same content").unwrap();
    let after_first = fs::read_to_string(&file).unwrap();

    upsert_managed_section(&file, "same content").unwrap();
    let after_second = fs::read_to_string(&file).unwrap();

    assert_eq!(
        after_first, after_second,
        "upsert must be idempotent: second call must not alter the file"
    );
}

/// Write an AGENTS.md skeleton with an empty managed section and return its path.
fn agents_md_with_markers(dir: &Path) -> PathBuf {
    let file = dir.join("AGENTS.md");
    fs::write(
        &file,
        "# Header\n\n<!-- BEGIN BUZZ MANAGED — regenerated automatically, do not edit below -->\n\n<!-- END BUZZ MANAGED -->\n",
    )
    .unwrap();
    file
}

#[test]
fn commit_newer_generation_wins_over_a_stale_finisher() {
    // Models the CRUD race: generation A snapshots pre-edit state and its relay
    // fetch is slow; generation B snapshots post-edit state and commits first.
    // When A finally finishes and commits LAST, its lower generation is dropped
    // so the file still reflects B. Ordering of *finishing* is the only variable —
    // the generation, claimed at request time, decides the winner.
    let gate = NestRegenGate::new();
    let tmp = tempfile::tempdir().unwrap();
    let file = agents_md_with_markers(tmp.path());

    let gen_a = gate.claim(); // pre-edit request
    let gen_b = gate.claim(); // post-edit request
    assert!(gen_a < gen_b);

    // B (newer) commits first.
    assert!(gate.commit(&file, "post-edit roster", gen_b).unwrap());
    // A (older) finishes last and must be dropped.
    assert!(
        !gate.commit(&file, "pre-edit roster", gen_a).unwrap(),
        "a stale (lower-generation) render must not overwrite a newer one"
    );

    let content = fs::read_to_string(&file).unwrap();
    assert!(content.contains("post-edit roster"));
    assert!(
        !content.contains("pre-edit roster"),
        "final file must reflect the newer generation, not the stale finisher"
    );
}

#[test]
fn commit_boot_fallback_relay_cannot_bury_apply_workspace_relay() {
    // Models boot→apply_workspace relay switching: the boot regen (generation 1,
    // fallback relay) is claimed first but finishes last; the apply_workspace
    // regen (generation 2, workspace relay) commits first. The workspace relay
    // render must survive even though the fallback-relay task writes afterward.
    let gate = NestRegenGate::new();
    let tmp = tempfile::tempdir().unwrap();
    let file = agents_md_with_markers(tmp.path());

    let boot_gen = gate.claim(); // boot, fallback relay
    let apply_gen = gate.claim(); // apply_workspace, workspace relay

    // apply_workspace's render lands first.
    assert!(gate
        .commit(
            &file,
            "## Workspace\n- Relay: wss://workspace.example",
            apply_gen,
        )
        .unwrap());
    // Boot's slower fallback-relay render finishes last and is dropped.
    assert!(!gate
        .commit(
            &file,
            "## Workspace\n- Relay: wss://fallback.example",
            boot_gen,
        )
        .unwrap());

    let content = fs::read_to_string(&file).unwrap();
    assert!(content.contains("wss://workspace.example"));
    assert!(
        !content.contains("wss://fallback.example"),
        "the fallback-relay boot render must not overwrite the workspace-relay render"
    );
}

#[test]
fn commit_failed_newer_request_still_supersedes_older_snapshot() {
    // Carl 4954831197, case 1: a newer request that never writes must still
    // permanently supersede an older snapshot. gen1 (pre-edit) is claimed and
    // its relay work is slow; an edit claims gen2 (post-edit); gen2 then FAILS
    // during its relay work, so it never commits. When gen1 finally finishes,
    // it must NOT publish its obsolete roster — gating on highest-*requested*
    // (advanced by gen2's claim) drops it, whereas gating on highest-*written*
    // (0, since gen2 never wrote) would wrongly let gen1 publish.
    let gate = NestRegenGate::new();
    let tmp = tempfile::tempdir().unwrap();
    let file = agents_md_with_markers(tmp.path());

    let gen1 = gate.claim(); // pre-edit request
    let gen2 = gate.claim(); // post-edit request
    assert!(gen1 < gen2);

    // gen2 fails during relay work and never reaches commit — nothing written.

    // gen1 finishes last; its stale render must be dropped.
    assert!(
        !gate.commit(&file, "pre-edit roster", gen1).unwrap(),
        "an older snapshot must not publish once a newer generation was requested, \
         even if that newer generation failed before writing"
    );

    let content = fs::read_to_string(&file).unwrap();
    assert!(
        !content.contains("pre-edit roster"),
        "the obsolete pre-edit roster must never become authoritative"
    );
}

#[test]
fn commit_claim_at_the_older_tasks_cutover_supersedes_it() {
    // Carl 4954831197, case 2: a claim arriving at the older task's commit
    // cutover must not slip between the eligibility compare and the write.
    // gen1 becomes eligible and enters `commit`; while it holds the lock
    // (after the compare, before the write) a claim is attempted. The correct
    // single-lock gate shares `highest_requested` between `claim` and
    // `commit`, so that claim cannot acquire the lock until gen1's write
    // releases it — the flawed separate-watermark/separate-write-lock design
    // Carl warned about would let the claim proceed immediately.
    //
    // Determinism: the under-lock hook calls `try_claim`, a non-blocking claim
    // against the exact lock `claim` takes, and asserts it reports the lock
    // held (`None`). This is a direct statement about the gate's locking with
    // no thread, channel, or sleep — the correct design necessarily returns
    // `None` and the separate-watermark design necessarily returns `Some`, so
    // the discriminator cannot be flipped by scheduler timing.
    let gate = NestRegenGate::new();
    let tmp = tempfile::tempdir().unwrap();
    let file = agents_md_with_markers(tmp.path());

    let gen1 = gate.claim();

    let wrote_gen1 = gate
        .commit_hooked(&file, "gen1 roster", gen1, || {
            // We are past the eligibility compare and hold the lock. A claim
            // attempted now must find the shared lock held — proving the
            // compare and the write are atomic against any new claim.
            assert!(
                gate.try_claim().is_none(),
                "a claim must not acquire the gate while an older commit holds \
                 the shared lock between its eligibility check and its write — \
                 the eligibility compare is not atomic with the write \
                 (separate-watermark design)"
            );
        })
        .unwrap();
    assert!(
        wrote_gen1,
        "gen1 was still the highest request when it entered commit, so its write \
         is legitimate; the newer request only lands after the lock releases"
    );

    // The lock is free once commit returns, so a newer request now claims and
    // may publish over gen1.
    let gen2 = gate.claim();
    assert!(gen1 < gen2);
    assert!(gate.commit(&file, "gen2 roster", gen2).unwrap());

    let content = fs::read_to_string(&file).unwrap();
    assert!(content.contains("gen2 roster"));
    assert!(!content.contains("gen1 roster"));
}

#[test]
fn commit_equal_generation_is_allowed() {
    // The gate rejects only strictly-lower generations. Re-committing the same
    // generation (e.g. a retried request) is permitted and refreshes the file.
    let gate = NestRegenGate::new();
    let tmp = tempfile::tempdir().unwrap();
    let file = agents_md_with_markers(tmp.path());

    let gen = gate.claim();
    assert!(gate.commit(&file, "first", gen).unwrap());
    assert!(
        gate.commit(&file, "second", gen).unwrap(),
        "an equal generation must still be allowed to write"
    );

    let content = fs::read_to_string(&file).unwrap();
    assert!(content.contains("second"));
}

#[test]
fn commit_poisoned_lock_returns_error_instead_of_panicking() {
    // A poisoned gate lock must degrade to an io::Error so the fire-and-forget
    // caller warns and continues, never panicking the desktop process (root
    // AGENTS.md: no new expect() in production paths). Poison the lock by
    // panicking a thread while it holds the guard, then assert commit yields
    // Err rather than unwinding.
    let gate = std::sync::Arc::new(NestRegenGate::new());
    let tmp = tempfile::tempdir().unwrap();
    let file = agents_md_with_markers(tmp.path());
    let gen = gate.claim();

    let poisoner = gate.clone();
    let _ = std::thread::spawn(move || {
        let _guard = poisoner.highest_requested.lock().unwrap();
        panic!("poison the gate lock");
    })
    .join();

    let result = gate.commit(&file, "after poison", gen);
    assert!(
        result.is_err(),
        "a poisoned lock must surface as an error, not a panic"
    );
}
