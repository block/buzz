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

use super::project_bw::load as load_bw_input;
use crate::app_state::AppState;
use crate::bw_projection;
use crate::relay::{query_relay, submit_signed_event_with_keys};
use buzz_sdk_pkg::bw::{record as bw_record, Publication, RecordType};
use nostr::{EventBuilder, Kind, Tag, Timestamp};
use serde::Deserialize;
use serde_json::{json, Value};
use tauri::State;

/// The three P4D-scoped 46100 record types this command will sign. Every
/// other BW record type (release/build/test/verdict machinery) belongs to a
/// later phase and is deliberately not reachable through this command, even
/// though Core would refuse an unauthorized attempt at any of them anyway.
fn record_type(name: &str) -> Result<RecordType, String> {
    match name {
        "issue-state" => Ok(RecordType::IssueState),
        "issue-update" => Ok(RecordType::IssueUpdate),
        "triage-action" => Ok(RecordType::TriageAction),
        other => Err(format!(
            "Unsupported BW record type for this command: {other}"
        )),
    }
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
    state: State<'_, AppState>,
) -> Result<Value, String> {
    let kind = record_type(&input.record)?;
    let bw_input = load_bw_input(&state, &input.repo).await?;
    let (mut consumer, _) = bw_projection::replay(&bw_input);
    let activation = consumer
        .activation()
        .map_err(|refusal| refusal.to_string())?;
    let keys = state.signing_keys()?;

    let tags = assemble_bw_tags(&activation.policy, &input.repo, input.delegate, &input.tags)?;
    let draft = bw_record(kind, tags, input.content).map_err(|error| error.to_string())?;
    let dry = draft.dry_run().map_err(|error| error.to_string())?;

    let created_at = Timestamp::now();
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
    submit_signed_event_with_keys(&event, &state, &keys, None).await?;

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
}
