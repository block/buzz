//! Signed NIP-BW record production for issue creation, text/criteria edits and
//! triage actions (P4D). This is deliberately the *only* write path for the
//! three record types it accepts: it signs and submits, it never decides
//! authorization itself. Every candidate is validated by the same offline
//! Core `Consumer` the read bridge (`bw_projection`, `commands::project_bw`)
//! already uses, through the already-tested SDK `Publication` boundary
//! (`crates/buzz-sdk/src/bw.rs`): role, causality, policy-binding and
//! reference checks all happen there, not in this file. A direct call with a
//! disallowed record type, a field a role cannot touch, a stale `previous` or
//! a pre-cutover enrollment is refused here exactly as it would be refused if
//! a UI button offered it — there is no separate, weaker gate for scripted
//! callers.
//!
//! Legacy 1630-1633 issue status kinds are untouched and unreferenced here;
//! see `project_issue_status.rs`. This module never routes through them.
//!
//! P4E adds the `issue-relation` record type (parent/child/blocks/duplicate
//! edges — still fully decided by Core's own cycle/conflict checks, never
//! simplified here) and a narrow, explicit-action-only extension for the
//! `implemented` issue-state transition: NIP-BW.md requires that transition's
//! `commit`/`remote_readback` to match an *externally observed* canonical
//! Relay Git head, never a caller's claim. This command resolves that one
//! fact itself (`git ls-remote` against the repository's own owner-signed
//! clone URL, read at the moment of this explicit "mark implemented"
//! submission — never from rendering the issue) and overwrites whatever the
//! caller sent for those two fields before Core ever sees the candidate, the
//! same way `assemble_bw_tags` already overwrites a caller-supplied `policy`.

use super::project_bw::load as load_bw_input;
use super::project_bw_signer::explicit_writer_signer;
use super::project_git_exec::{
    build_git_auth_config, run_git, validate_workspace_clone_url, GitAuthConfig,
};
use super::project_git_workflow::project_owner_identity;
use crate::app_state::AppState;
use crate::bw_projection;
use crate::relay::{query_relay, submit_signed_event_with_keys};
use buzz_core_pkg::bw::parse_json;
use buzz_sdk_pkg::bw::{record as bw_record, Publication, RecordType};
use nostr::{EventBuilder, Kind, Tag, Timestamp};
use serde::Deserialize;
use serde_json::{json, Value};
use tauri::{AppHandle, State};

/// The four P4D/P4E-scoped 46100 record types this command will sign. Every
/// other BW record type (release/build/test/verdict machinery) belongs to a
/// later phase and is deliberately not reachable through this command, even
/// though Core would refuse an unauthorized attempt at any of them anyway.
fn record_type(name: &str) -> Result<RecordType, String> {
    match name {
        "issue-state" => Ok(RecordType::IssueState),
        "issue-update" => Ok(RecordType::IssueUpdate),
        "triage-action" => Ok(RecordType::TriageAction),
        "issue-relation" => Ok(RecordType::IssueRelation),
        other => Err(format!(
            "Unsupported BW record type for this command: {other}"
        )),
    }
}

/// Defensive shape gate before a stream name reaches a `git` argv, mirroring
/// (not replacing) NIP-BW.md's own `stream` grammar
/// (`crates/buzz-core/src/bw/shape.rs`). This never decides BW validity —
/// Core's own shape/causality checks still run on the assembled candidate —
/// it only keeps a hostile stream value from being interpreted as a git flag
/// or an out-of-repo ref before that point.
fn validate_stream_for_git(stream: &str) -> Result<(), String> {
    let bytes = stream.as_bytes();
    let ok = !stream.is_empty()
        && stream.len() <= 128
        && bytes[0].is_ascii_alphanumeric()
        && bytes.iter().all(|b| {
            b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'_' | b'/' | b'-')
        })
        && !stream.contains("..")
        && !stream.contains("@{")
        && !stream.ends_with(['.', '/'])
        && stream
            .split('/')
            .all(|part| !part.is_empty() && !part.starts_with('.') && !part.ends_with(".lock"));
    if ok {
        Ok(())
    } else {
        Err("Invalid BW stream name.".to_string())
    }
}

/// The owner-signed repository genesis's `clone` URL — read only from the
/// exact record Core's own `activation()` already authenticated (`genesis`),
/// never from any other same-coordinate 30617 event that happened to be
/// fetched alongside it.
fn genesis_clone_url(bw_input: &bw_projection::Input, genesis_id: &str) -> Result<String, String> {
    bw_input
        .events
        .iter()
        .find_map(|raw| {
            let wire = parse_json(raw.as_bytes()).ok()?;
            if wire["id"].as_str() != Some(genesis_id) {
                return None;
            }
            wire["tags"].as_array()?.iter().find_map(|tag| {
                let tag = tag.as_array()?;
                (tag.first()?.as_str()? == "clone")
                    .then(|| tag.get(1)?.as_str())
                    .flatten()
                    .map(str::to_owned)
            })
        })
        .ok_or_else(|| "Repository announcement has no clone URL on record.".to_string())
}

/// Read the real current commit of `stream` from the repository's own git
/// hosting. A blocking `git` subprocess call, run only for this one explicit
/// write (never from a render): the only way NIP-BW.md's `implemented`
/// transition can carry a fact instead of a claim.
fn resolve_stream_head_blocking(
    clone_url: &str,
    stream: &str,
    auth: &GitAuthConfig,
) -> Result<String, String> {
    let refname = format!("refs/heads/{stream}");
    let output = run_git(
        &[
            "ls-remote",
            "--exit-code",
            "--end-of-options",
            clone_url,
            refname.as_str(),
        ],
        None,
        auth,
    )
    .map_err(|error| format!("Could not read the repository's current {stream} head: {error}"))?;
    // `git ls-remote <url> <pattern>` matches a pattern against the *tail* of
    // a ref, anchored at either the start of the ref or a `/` boundary
    // (git-ls-remote(1)) — so a ref an attacker pushed as
    // `refs/heads/x/refs/heads/<stream>` also matches this same pattern and
    // can sort before the real branch in the output. Anyone who can push to
    // the repository (the assigned writer, by construction) can otherwise
    // pick the "externally observed" commit for their own implemented claim.
    // Only a line whose own ref is byte-for-byte `refs/heads/<stream>` is
    // ever accepted; any other match is a shadow, not the branch.
    output
        .lines()
        .find_map(|line| {
            let mut parts = line.split_whitespace();
            let sha = parts.next()?;
            let matched_ref = parts.next()?;
            (matched_ref == refname).then(|| sha.to_ascii_lowercase())
        })
        .filter(|sha| sha.len() == 40 && sha.bytes().all(|b| b.is_ascii_hexdigit()))
        .ok_or_else(|| format!("Could not resolve a commit for stream {stream}."))
}

/// Extra, record-specific tags supplied by the caller (`issue`, `previous`,
/// `target` is carried in content, `delegation` is requested via `delegate`
/// below, never supplied directly). `record`, `a` and `policy` are always
/// injected here from the freshly resolved activation, never taken from the
/// caller, so a stale or guessed policy id can never be smuggled in.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubmitBwRecordInput {
    repo: String,
    record: String,
    #[serde(default)]
    tags: Vec<Vec<String>>,
    content: Value,
    #[serde(default)]
    delegate: bool,
    /// Optional explicit signer for the two writer-owned state transitions.
    /// It must resolve to the current identity or a locally managed agent;
    /// every candidate still passes Core's exact role/causality checks.
    signer_pubkey: Option<String>,
}

const RESERVED_CALLER_TAGS: [&str; 4] = ["record", "a", "policy", "delegation"];

/// Assemble the full tag set for a BW candidate from its resolved policy id
/// and the caller's record-specific extra tags. Pure and independently
/// testable: this is the exact shape the Core `Consumer` will see.
fn assemble_bw_tags(
    policy: &str,
    repo: &str,
    delegate: bool,
    extra: &[Vec<String>],
) -> Result<Vec<Vec<String>>, String> {
    for tag in extra {
        if tag.len() != 2 {
            return Err("BW record tags must each be a [name, value] pair.".into());
        }
        if RESERVED_CALLER_TAGS.contains(&tag[0].as_str()) {
            return Err(format!(
                "The `{}` tag is derived automatically and cannot be supplied directly.",
                tag[0]
            ));
        }
    }
    let mut tags = vec![
        vec!["a".to_string(), repo.to_string()],
        vec!["policy".to_string(), policy.to_string()],
    ];
    if delegate {
        tags.push(vec!["delegation".to_string(), policy.to_string()]);
    }
    tags.extend(extra.iter().cloned());
    Ok(tags)
}

/// Sign, submit and readback-confirm one BW-record candidate against the
/// live repository history, and only then fold it into the returned
/// projection. Refuses (never bypasses) unauthorized, stale or malformed
/// attempts with Core's own `bw:<outcome>:<stage>:<code>` reason.
#[tauri::command]
pub async fn submit_project_bw_record(
    input: SubmitBwRecordInput,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    let kind = record_type(&input.record)?;
    let explicit_signer = explicit_writer_signer(
        &input.record,
        &input.content,
        input.signer_pubkey.as_deref(),
    )?;
    let bw_input = load_bw_input(&state, &input.repo).await?;
    let (mut consumer, _) = bw_projection::replay(&bw_input);
    // `bw_input.now` is captured before `load_bw_input`'s own network round
    // trips (`project_bw.rs::load` pages every BW-relevant kind, then
    // resolves referenced ids in further round trips). Against a real relay
    // that can take longer than the zero-tolerance window Core's
    // `references:future` check allows, so a candidate signed after this
    // point could otherwise look like it arrived from the future purely
    // because of how long the load took. Refresh to the instant we are
    // about to sign at before evaluating anything against it.
    let created_at = Timestamp::now();
    consumer.observe(bw_input.external.clone(), created_at.as_secs());
    let activation = consumer
        .activation()
        .map_err(|refusal| refusal.to_string())?;
    let signing_identity = explicit_signer
        .as_deref()
        .map(|signer| project_owner_identity(&app, &state, signer))
        .transpose()?;
    let keys = match signing_identity.as_ref() {
        Some(identity) => identity.keys.clone(),
        None => state.signing_keys()?,
    };
    let auth_tag = signing_identity
        .as_ref()
        .and_then(|identity| identity.auth_tag.as_deref());

    let mut content = input.content;
    if matches!(kind, RecordType::IssueState)
        && content.get("state").and_then(Value::as_str) == Some("implemented")
    {
        let stream = content
            .get("stream")
            .and_then(Value::as_str)
            .ok_or_else(|| "A BW implemented transition requires a stream.".to_string())?
            .to_string();
        validate_stream_for_git(&stream)?;
        let clone_url = genesis_clone_url(&bw_input, &activation.genesis)?;
        validate_workspace_clone_url(&clone_url, &state)?;
        let auth = build_git_auth_config(&state)?;
        let head = {
            let clone_url = clone_url.clone();
            let stream = stream.clone();
            tauri::async_runtime::spawn_blocking(move || {
                resolve_stream_head_blocking(&clone_url, &stream, &auth)
            })
            .await
            .map_err(|error| format!("git readback task failed: {error}"))??
        };
        // The externally observed fact overwrites whatever the caller sent
        // for `commit`/`remote_readback` — NIP-BW.md: "The signed claim
        // alone proves no remote fact." `observed_at` is pinned to this same
        // `created_at` so the age/ordering check Core applies
        // (`external.rs`: observed_at <= created_at, age <= 300s) is met by
        // construction, not by a race between this read and signing.
        let remote_readback = json!({
            "repo": input.repo,
            "stream": stream,
            "head": head,
            "observed_at": created_at.as_secs(),
        });
        content["commit"] = json!(head);
        content["remote_readback"] = remote_readback.clone();
        let mut evidence = bw_input.external.clone();
        evidence.git_readbacks = vec![remote_readback];
        consumer.observe(evidence, created_at.as_secs());
    }

    let tags = assemble_bw_tags(&activation.policy, &input.repo, input.delegate, &input.tags)?;
    let draft = bw_record(kind, tags, content).map_err(|error| error.to_string())?;
    let dry = draft.dry_run().map_err(|error| error.to_string())?;

    let candidate = json!({
        "pubkey": keys.public_key().to_hex(),
        "created_at": created_at.as_secs(),
        "kind": dry["kind"],
        "tags": dry["tags"],
        "content": dry["content"],
    });
    let publication =
        Publication::prepare(&consumer, candidate).map_err(|refusal| refusal.to_string())?;

    let parsed_tags = dry["tags"]
        .as_array()
        .ok_or_else(|| "bw:shape:tags".to_string())?
        .iter()
        .map(|tag| {
            let parts: Vec<&str> = tag
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .collect();
            Tag::parse(parts).map_err(|error| error.to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;
    let content = dry["content"]
        .as_str()
        .ok_or_else(|| "bw:shape:content".to_string())?
        .to_string();
    let event_kind = dry["kind"]
        .as_u64()
        .ok_or_else(|| "bw:shape:kind".to_string())? as u16;

    let event = EventBuilder::new(Kind::Custom(event_kind), content)
        .tags(parsed_tags)
        .custom_created_at(created_at)
        .sign_with_keys(&keys)
        .map_err(|error| format!("sign bw record: {error}"))?;

    publication
        .seal(&event)
        .map_err(|error| error.to_string())?;
    submit_signed_event_with_keys(&event, &state, &keys, auth_tag).await?;

    let readback = query_relay(
        &state,
        &[json!({"ids": [publication.event_id()], "limit": 1})],
    )
    .await?
    .into_iter()
    .find(|candidate| candidate.id.to_hex() == publication.event_id());

    publication
        .confirm(&mut consumer, &event, readback.as_ref())
        .map_err(|error| error.to_string())?;

    Ok(json!({
        "eventId": publication.event_id(),
        "projection": consumer.projection(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use buzz_core_pkg::bw::Consumer;

    // These tests exercise the exact sequence `submit_project_bw_record` runs
    // to turn a caller's record/tags/content into a validated candidate —
    // `record_type`, `assemble_bw_tags`, `bw_record`, `Publication::prepare`
    // — against the real pinned NIP-BW corpus, with no network and no mock
    // role logic of our own. A caller that reached this exact code path
    // directly (a scripted request with no UI at all, e.g. bypassing a
    // hidden triage button) is refused here precisely because Core's own
    // role/causality/policy checks run unconditionally, before any signing
    // or submission — there is no separate, weaker gate for that caller.
    fn fixtures() -> Value {
        serde_json::from_str(include_str!("../../../../docs/nips/NIP-BW.fixtures.json"))
            .expect("fixtures")
    }
    fn case(f: &Value, name: &str, labels: &[&str]) -> Consumer {
        let c = f["cases"]
            .as_array()
            .expect("cases")
            .iter()
            .find(|c| c["name"] == name)
            .expect("case");
        let mut consumer = Consumer::new(
            serde_json::from_value(c["trust"].clone()).expect("trust"),
            serde_json::from_value(c["external"].clone()).expect("evidence"),
            c["now"].as_u64().expect("now"),
        );
        for label in labels {
            consumer.ingest(&serde_json::to_vec(&f["events"][label]["event"]).expect("event"));
        }
        consumer
    }
    /// Assemble the exact candidate `submit_project_bw_record` would sign for
    /// one fixture event: same pubkey/created_at/content, tags rebuilt through
    /// `assemble_bw_tags`/`bw_record` (not lifted verbatim from the fixture),
    /// so this genuinely exercises this file's own tag assembly.
    fn candidate_for(
        f: &Value,
        label: &str,
        record: RecordType,
        policy: &str,
        repo: &str,
    ) -> Value {
        let e = &f["events"][label]["event"];
        let issue = e["tags"]
            .as_array()
            .expect("tags")
            .iter()
            .find(|t| t[0] == "issue")
            .expect("issue tag")[1]
            .as_str()
            .expect("issue id")
            .to_owned();
        let content: Value =
            serde_json::from_str(e["content"].as_str().expect("content")).expect("json content");
        let tags = assemble_bw_tags(policy, repo, false, &[vec!["issue".into(), issue]])
            .expect("assemble tags");
        let draft = bw_record(record, tags, content).expect("draft");
        let dry = draft.dry_run().expect("dry run");
        json!({
            "pubkey": e["pubkey"],
            "created_at": e["created_at"],
            "kind": dry["kind"],
            "tags": dry["tags"],
            "content": dry["content"],
        })
    }

    #[test]
    fn direct_enroll_candidate_succeeds_for_a_conscious_post_cutover_root() {
        let f = fixtures();
        let consumer = case(&f, "issue-update-positive", &["repo", "policy", "root_a"]);
        let activation = consumer.activation().expect("activation");
        let candidate = candidate_for(
            &f,
            "enroll_a",
            RecordType::IssueState,
            &activation.policy,
            &activation.repo,
        );
        Publication::prepare(&consumer, candidate).expect("enroll accepted");
    }

    #[test]
    fn direct_enroll_candidate_rejects_a_pre_cutover_root_with_no_ui_involved() {
        let f = fixtures();
        let consumer = case(&f, "cutover-old-root", &["repo", "policy", "old-root"]);
        let activation = consumer.activation().expect("activation");
        let candidate = candidate_for(
            &f,
            "old-enroll",
            RecordType::IssueState,
            &activation.policy,
            &activation.repo,
        );
        let refusal = Publication::prepare(&consumer, candidate).expect_err("pre-cutover");
        assert_eq!(
            refusal.to_string(),
            "invalid input: bw:reject:policy:cutover"
        );
    }

    #[test]
    fn direct_issue_update_candidate_rejects_an_unauthorized_signer() {
        let f = fixtures();
        let consumer = case(
            &f,
            "issue-update-wrong-role",
            &["repo", "policy", "root_a", "enroll_a"],
        );
        let activation = consumer.activation().expect("activation");
        let candidate = candidate_for(
            &f,
            "issue-update-wrong-role",
            RecordType::IssueUpdate,
            &activation.policy,
            &activation.repo,
        );
        let refusal = Publication::prepare(&consumer, candidate).expect_err("wrong role");
        assert_eq!(
            refusal.to_string(),
            "invalid input: bw:reject:role:unauthorized"
        );
    }

    #[test]
    fn direct_issue_update_candidate_accepts_the_reporters_own_text_edit() {
        let f = fixtures();
        let consumer = case(
            &f,
            "issue-update-positive",
            &["repo", "policy", "root_a", "enroll_a"],
        );
        let activation = consumer.activation().expect("activation");
        let candidate = candidate_for(
            &f,
            "update_a",
            RecordType::IssueUpdate,
            &activation.policy,
            &activation.repo,
        );
        Publication::prepare(&consumer, candidate).expect("text edit accepted");
    }

    #[test]
    fn direct_triage_action_candidate_rejects_an_unauthorized_signer() {
        let f = fixtures();
        let consumer = case(
            &f,
            "triage-action-wrong-role",
            &["repo", "policy", "root_a", "enroll_a", "update_a"],
        );
        let activation = consumer.activation().expect("activation");
        let candidate = candidate_for(
            &f,
            "triage-action-wrong-role",
            RecordType::TriageAction,
            &activation.policy,
            &activation.repo,
        );
        let refusal = Publication::prepare(&consumer, candidate).expect_err("wrong role");
        assert_eq!(
            refusal.to_string(),
            "invalid input: bw:reject:role:unauthorized"
        );
    }

    #[test]
    fn direct_triage_action_candidate_accepts_the_owners_accept_to_backlog() {
        let f = fixtures();
        let consumer = case(
            &f,
            "triage-action-positive",
            &["repo", "policy", "root_a", "enroll_a", "update_a"],
        );
        let activation = consumer.activation().expect("activation");
        let candidate = candidate_for(
            &f,
            "accept_a",
            RecordType::TriageAction,
            &activation.policy,
            &activation.repo,
        );
        Publication::prepare(&consumer, candidate).expect("accept accepted");
    }

    #[test]
    fn unsupported_record_types_are_refused_before_any_network_call() {
        assert!(record_type("release-set").is_err());
        assert!(record_type("build-request").is_err());
        assert!(record_type("member-verdict").is_err());
        assert!(record_type("issue-state").is_ok());
        assert!(record_type("issue-update").is_ok());
        assert!(record_type("triage-action").is_ok());
        assert!(record_type("issue-relation").is_ok());
    }

    #[test]
    fn assembled_tags_always_carry_the_resolved_policy_not_a_caller_supplied_one() {
        let tags = assemble_bw_tags(
            "policy-id",
            "30617:owner:repo",
            false,
            &[vec!["issue".into(), "issue-id".into()]],
        )
        .expect("tags");
        assert_eq!(
            tags,
            vec![
                vec!["a".to_string(), "30617:owner:repo".to_string()],
                vec!["policy".to_string(), "policy-id".to_string()],
                vec!["issue".to_string(), "issue-id".to_string()],
            ]
        );
    }

    #[test]
    fn delegation_tag_equals_the_current_policy_and_is_never_caller_supplied() {
        let tags = assemble_bw_tags("policy-id", "30617:owner:repo", true, &[]).expect("tags");
        assert!(tags.contains(&vec!["delegation".to_string(), "policy-id".to_string()]));

        let rejected = assemble_bw_tags(
            "policy-id",
            "30617:owner:repo",
            false,
            &[vec!["delegation".into(), "forged-policy".into()]],
        );
        assert!(rejected.is_err());
    }

    #[test]
    fn callers_cannot_smuggle_record_a_or_policy_through_extra_tags() {
        for reserved in ["record", "a", "policy"] {
            let result = assemble_bw_tags(
                "policy-id",
                "30617:owner:repo",
                false,
                &[vec![reserved.into(), "x".into()]],
            );
            assert!(result.is_err(), "{reserved} should be rejected");
        }
    }

    #[test]
    fn malformed_extra_tags_are_rejected() {
        assert!(assemble_bw_tags(
            "policy-id",
            "30617:owner:repo",
            false,
            &[vec!["issue".into()]],
        )
        .is_err());
    }

    // P4E: relations/parent-leaf-cycle handling and the implemented handoff's
    // external-evidence requirement. Relation authority, cycle detection and
    // leaf eligibility are still decided exclusively by Core
    // (`crates/buzz-core/src/bw/semantics.rs::causality`,
    // `is_executable_leaf`) — this command only ever assembles the candidate.

    #[test]
    fn direct_issue_relation_candidate_accepts_a_coordinator_signed_parent_of_edge() {
        let f = fixtures();
        let consumer = case(
            &f,
            "issue-relation-positive",
            &[
                "repo",
                "policy",
                "root_a",
                "enroll_a",
                "update_a",
                "accept_a",
                "backlog_a",
                "assign_a",
                "ready_a",
                "dev_a",
                "implemented_a",
                "root_b",
                "enroll_b",
                "update_b",
                "accept_b",
                "backlog_b",
                "assign_b",
                "ready_b",
                "dev_b",
                "implemented_b",
            ],
        );
        let activation = consumer.activation().expect("activation");
        let candidate = candidate_for(
            &f,
            "relation",
            RecordType::IssueRelation,
            &activation.policy,
            &activation.repo,
        );
        Publication::prepare(&consumer, candidate).expect("relation accepted");
    }

    #[test]
    fn direct_issue_relation_candidate_rejects_an_unauthorized_signer() {
        let f = fixtures();
        let consumer = case(
            &f,
            "issue-relation-negative",
            &[
                "repo",
                "policy",
                "root_a",
                "enroll_a",
                "update_a",
                "accept_a",
                "backlog_a",
                "assign_a",
                "ready_a",
                "dev_a",
                "implemented_a",
                "root_b",
                "enroll_b",
                "update_b",
                "accept_b",
                "backlog_b",
                "assign_b",
                "ready_b",
                "dev_b",
                "implemented_b",
            ],
        );
        let activation = consumer.activation().expect("activation");
        let candidate = candidate_for(
            &f,
            "relation-role",
            RecordType::IssueRelation,
            &activation.policy,
            &activation.repo,
        );
        let refusal = Publication::prepare(&consumer, candidate).expect_err("wrong role");
        assert_eq!(
            refusal.to_string(),
            "invalid input: bw:reject:role:unauthorized"
        );
    }

    #[test]
    fn validate_stream_for_git_accepts_ordinary_branch_names_and_rejects_argv_or_traversal_hazards()
    {
        assert!(validate_stream_for_git("windows-integration").is_ok());
        assert!(validate_stream_for_git("feature/p4e").is_ok());
        // Would otherwise be interpreted as a git flag rather than a ref.
        assert!(validate_stream_for_git("--upload-pack=/tmp/evil").is_err());
        assert!(validate_stream_for_git("").is_err());
        assert!(validate_stream_for_git("feature/..").is_err());
        assert!(validate_stream_for_git("feature/x.lock").is_err());
        assert!(validate_stream_for_git("feature@{1}").is_err());
        assert!(validate_stream_for_git("trailing/").is_err());
        assert!(validate_stream_for_git(&"a".repeat(129)).is_err());
    }

    #[test]
    fn genesis_clone_url_reads_only_the_activation_authenticated_genesis() {
        let genesis_id = "g".repeat(64);
        let foreign_id = "f".repeat(64);
        let bw_input = bw_projection::Input {
            trust: serde_json::from_value(json!({
                "community": "https://relay.example.invalid",
                "repo": "30617:owner:repo",
                "owner": "owner",
            }))
            .expect("trust"),
            external: serde_json::from_value(json!({
                "git_readbacks": [], "git_ancestry": [], "provider_readbacks": [],
                "downloads": [], "host_authorization": {"allowed": false},
            }))
            .expect("evidence"),
            now: 0,
            events: vec![
                json!({
                    "id": foreign_id, "pubkey": "owner", "created_at": 0, "kind": 30617,
                    "tags": [["d", "repo"], ["clone", "https://wrong.example.invalid"]],
                    "content": "", "sig": "0".repeat(128),
                })
                .to_string(),
                json!({
                    "id": genesis_id, "pubkey": "owner", "created_at": 0, "kind": 30617,
                    "tags": [["d", "repo"], ["clone", "https://relay.example.invalid/repo.git"]],
                    "content": "", "sig": "0".repeat(128),
                })
                .to_string(),
            ],
        };
        assert_eq!(
            genesis_clone_url(&bw_input, &genesis_id).expect("clone url"),
            "https://relay.example.invalid/repo.git"
        );
        assert!(genesis_clone_url(&bw_input, &"0".repeat(64)).is_err());
    }

    #[test]
    fn resolve_stream_head_blocking_reads_the_real_current_commit() {
        use super::super::project_git_exec::{build_test_git_auth_config, run_git};

        let auth = build_test_git_auth_config().expect("build test git config");
        let root = tempfile::tempdir().expect("create test directory");
        let remote = root.path().join("remote.git");
        let worktree = root.path().join("worktree");
        let remote_path = remote.to_str().expect("remote path");
        let worktree_path = worktree.to_str().expect("worktree path");

        run_git(&["init", "--bare", "--", remote_path], None, &auth).expect("init remote");
        run_git(&["init", "--", worktree_path], None, &auth).expect("init worktree");
        std::fs::write(worktree.join("README.md"), "p4e\n").expect("write fixture");
        run_git(&["add", "README.md"], Some(&worktree), &auth).expect("stage fixture");
        run_git(
            &[
                "-c",
                "user.name=Buzz Test",
                "-c",
                "user.email=test@example.com",
                "commit",
                "-m",
                "Initial commit",
            ],
            Some(&worktree),
            &auth,
        )
        .expect("commit fixture");
        run_git(
            &["branch", "-M", "windows-integration"],
            Some(&worktree),
            &auth,
        )
        .expect("rename branch");
        run_git(
            &["remote", "add", "origin", remote_path],
            Some(&worktree),
            &auth,
        )
        .expect("add remote");
        run_git(
            &["push", "origin", "windows-integration"],
            Some(&worktree),
            &auth,
        )
        .expect("push branch");
        let commit = run_git(&["rev-parse", "HEAD"], Some(&worktree), &auth)
            .expect("resolve fixture commit")
            .trim()
            .to_ascii_lowercase();

        let head = resolve_stream_head_blocking(remote_path, "windows-integration", &auth)
            .expect("resolve head");
        assert_eq!(head, commit);
        assert!(resolve_stream_head_blocking(remote_path, "no-such-branch", &auth).is_err());
    }

    /// `git ls-remote <url> <pattern>` matches a pattern against the tail of
    /// a ref, anchored at the start of the ref or a `/` boundary — so a ref
    /// someone pushed as `refs/heads/x/refs/heads/<stream>` also matches the
    /// pattern `refs/heads/<stream>` and, sorted lexically, lands before the
    /// real branch. Anyone with push access to the repository (the assigned
    /// writer, by construction) could otherwise pick their own commit for
    /// the "externally observed" implemented readback. This proves the
    /// resolver rejects that shadow ref and still resolves the real one.
    #[test]
    fn resolve_stream_head_blocking_ignores_a_shadow_ref_that_matches_the_ls_remote_pattern() {
        use super::super::project_git_exec::{build_test_git_auth_config, run_git};

        let auth = build_test_git_auth_config().expect("build test git config");
        let root = tempfile::tempdir().expect("create test directory");
        let remote = root.path().join("remote.git");
        let worktree = root.path().join("worktree");
        let remote_path = remote.to_str().expect("remote path");
        let worktree_path = worktree.to_str().expect("worktree path");

        run_git(&["init", "--bare", "--", remote_path], None, &auth).expect("init remote");
        run_git(&["init", "--", worktree_path], None, &auth).expect("init worktree");
        run_git(
            &[
                "-c",
                "user.name=Buzz Test",
                "-c",
                "user.email=test@example.com",
                "commit",
                "--allow-empty",
                "-m",
                "Real head",
            ],
            Some(&worktree),
            &auth,
        )
        .expect("commit real head");
        run_git(
            &["branch", "-M", "windows-integration"],
            Some(&worktree),
            &auth,
        )
        .expect("rename branch");
        run_git(
            &["remote", "add", "origin", remote_path],
            Some(&worktree),
            &auth,
        )
        .expect("add remote");
        run_git(
            &["push", "origin", "windows-integration"],
            Some(&worktree),
            &auth,
        )
        .expect("push real head");
        let real_commit = run_git(&["rev-parse", "HEAD"], Some(&worktree), &auth)
            .expect("resolve real commit")
            .trim()
            .to_ascii_lowercase();

        run_git(
            &[
                "-c",
                "user.name=Buzz Test",
                "-c",
                "user.email=test@example.com",
                "commit",
                "--allow-empty",
                "-m",
                "Attacker-chosen shadow head",
            ],
            Some(&worktree),
            &auth,
        )
        .expect("commit shadow head");
        // Pushed under a ref whose tail is still `refs/heads/windows-integration`.
        run_git(
            &[
                "push",
                "origin",
                "HEAD:refs/heads/0/refs/heads/windows-integration",
            ],
            Some(&worktree),
            &auth,
        )
        .expect("push shadow ref");
        let shadow_commit = run_git(&["rev-parse", "HEAD"], Some(&worktree), &auth)
            .expect("resolve shadow commit")
            .trim()
            .to_ascii_lowercase();
        assert_ne!(real_commit, shadow_commit);

        let head = resolve_stream_head_blocking(remote_path, "windows-integration", &auth)
            .expect("resolve head");
        assert_eq!(head, real_commit);
    }

    #[test]
    fn an_implemented_candidate_without_matching_external_evidence_stays_pending() {
        let f = fixtures();
        let mut consumer = case(
            &f,
            "issue-state-positive",
            &[
                "repo",
                "policy",
                "root_a",
                "enroll_a",
                "update_a",
                "accept_a",
                "backlog_a",
                "assign_a",
                "ready_a",
                "dev_a",
            ],
        );
        // No `observe()` call: the consumer keeps the case's own fixture
        // evidence — this instead proves the opposite direction, that the
        // *unmodified* evidence is what makes `implemented_a` acceptable, by
        // first accepting it once...
        let ok_candidate = json!({
            "pubkey": f["events"]["implemented_a"]["event"]["pubkey"],
            "created_at": f["events"]["implemented_a"]["event"]["created_at"],
            "kind": f["events"]["implemented_a"]["event"]["kind"],
            "tags": f["events"]["implemented_a"]["event"]["tags"],
            "content": f["events"]["implemented_a"]["event"]["content"],
        });
        Publication::prepare(&consumer, ok_candidate.clone()).expect("evidenced claim accepted");
        // ...then proving an otherwise-identical claim is refused, never
        // silently trusted, once the external Git evidence is withdrawn —
        // exactly the caller-supplied-evidence path this command's own
        // `consumer.observe()` call replaces with a freshly resolved fact.
        consumer.observe(
            serde_json::from_value(json!({
                "git_readbacks": [], "git_ancestry": [], "provider_readbacks": [],
                "downloads": [], "host_authorization": {"allowed": false},
            }))
            .expect("empty evidence"),
            f["events"]["implemented_a"]["event"]["created_at"]
                .as_u64()
                .expect("now"),
        );
        let refusal = Publication::prepare(&consumer, ok_candidate).expect_err("unevidenced claim");
        assert_eq!(
            refusal.to_string(),
            "invalid input: bw:pending:external:relay-head"
        );
    }
}
