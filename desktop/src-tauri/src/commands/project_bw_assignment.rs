//! Signed NIP-BW assignment/unassignment production (P4E). This reuses the
//! *existing* kind:1 assignment wire grammar `issueAssignments.ts` already
//! produces for non-BW repositories (`e`/`a`/`p`/`t`/optional `prior`,
//! `crates/buzz-sdk/src/builders.rs::build_git_issue_assignment_with_prior`)
//! — there is no new record type, kind or tag here. What this file adds is
//! the same delegation discipline `project_bw_write.rs` established for the
//! 46100 record types: assemble the exact candidate, then let the offline
//! Core `Consumer` (through the already-tested SDK `Publication` boundary)
//! decide whether this signer may select or release the sole delegate at
//! this precise causal point. Per NIP-BW.md ("issue-state and the existing
//! assignment wire"): only Owner or the coordinator active at the
//! operation's `created_at` may sign; a first operation omits `prior`, every
//! later one must reference the unique prior operation; assignment adds the
//! sole delegate only when none currently exists, unassignment removes
//! exactly the current delegate. None of that is decided in this file —
//! `Publication::prepare` runs the identical role/causality checks
//! `crates/buzz-core/src/bw/semantics.rs` applies to every other BW record,
//! so a direct scripted call is refused exactly as a UI button would be.
//!
//! This command never selects the operation into a `ready`/`in-development`
//! issue-state itself — that is a separate, Owner/coordinator-signed
//! `issue-state` record submitted through `submit_project_bw_record`
//! (`project_bw_write.rs`), matching NIP-BW.md's "Operations themselves do
//! not change the active writer."

use super::project_bw::load as load_bw_input;
use crate::app_state::AppState;
use crate::bw_projection;
use crate::relay::{query_relay, submit_signed_event_with_keys};
use buzz_sdk_pkg::bw::{Draft, Publication};
use nostr::{EventBuilder, Kind, Tag, Timestamp};
use serde::Deserialize;
use serde_json::{json, Value};
use tauri::State;

/// `issue` is the exact 1621 root ID; `delegate` is the sole assignee this
/// operation adds (`assignment`) or removes (`unassignment`); `prior` is the
/// preceding operation's event ID, omitted only for the very first operation
/// on this issue's assignment chain.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubmitBwAssignmentInput {
    repo: String,
    issue_id: String,
    delegate: String,
    operation: String,
    #[serde(default)]
    prior: Option<String>,
}

/// The only two operations the existing kind:1 assignment wire defines.
/// Rejected here before any signing or network call, independently of
/// whatever Core would also refuse the candidate for.
fn validate_assignment_operation(operation: &str) -> Result<(), String> {
    match operation {
        "assignment" | "unassignment" => Ok(()),
        other => Err(format!("Unsupported BW assignment operation: {other}")),
    }
}

fn normalized_pubkey(value: &str) -> Result<String, String> {
    let normalized = value.trim().to_ascii_lowercase();
    if normalized.len() != 64 || !normalized.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("BW assignment delegate must be a 64-character hex pubkey.".into());
    }
    Ok(normalized)
}

/// Build the exact kind:1 tag set for one assignment/unassignment candidate.
/// Pure and independently testable: this is the exact shape the Core
/// `Consumer` will see, byte-for-byte the same grammar the non-BW assignment
/// path already writes.
fn assemble_assignment_tags(
    issue_id: &str,
    repo: &str,
    delegate: &str,
    operation: &str,
    prior: Option<&str>,
) -> Vec<Vec<String>> {
    let mut tags = vec![
        vec![
            "e".to_string(),
            issue_id.to_string(),
            "".to_string(),
            "root".to_string(),
        ],
        vec!["a".to_string(), repo.to_string()],
        vec!["p".to_string(), delegate.to_string()],
        vec!["t".to_string(), operation.to_string()],
    ];
    if let Some(prior) = prior {
        tags.push(vec!["prior".to_string(), prior.to_string()]);
    }
    tags
}

/// Sign, submit and readback-confirm one assignment/unassignment candidate
/// against the live repository history, folding it into the returned
/// projection only once Core has accepted the exact confirmed readback.
#[tauri::command]
pub async fn submit_project_bw_assignment(
    input: SubmitBwAssignmentInput,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    validate_assignment_operation(&input.operation)?;
    let operation = input.operation.clone();
    let delegate = normalized_pubkey(&input.delegate)?;

    let bw_input = load_bw_input(&state, &input.repo).await?;
    let (mut consumer, _) = bw_projection::replay(&bw_input);
    // See the matching comment in `project_bw_write.rs`: `bw_input.now` predates
    // `load_bw_input`'s own network round trips, so it can trail real time by
    // more than Core's zero-tolerance `references:future` window by the time a
    // candidate is actually signed below. Refresh before evaluating anything.
    let created_at = Timestamp::now();
    consumer.observe(bw_input.external.clone(), created_at.as_secs());
    let keys = state.signing_keys()?;

    let tags = assemble_assignment_tags(
        &input.issue_id,
        &input.repo,
        &delegate,
        &operation,
        input.prior.as_deref(),
    );
    let content = if operation == "assignment" {
        "Assigned this issue".to_string()
    } else {
        "Unassigned this issue".to_string()
    };
    let draft = Draft::new(1, json!(tags), content.clone()).map_err(|error| error.to_string())?;
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

    let event = EventBuilder::new(Kind::TextNote, content)
        .tags(parsed_tags)
        .custom_created_at(created_at)
        .sign_with_keys(&keys)
        .map_err(|error| format!("sign bw assignment: {error}"))?;

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

    // These tests exercise the exact path `submit_project_bw_assignment`
    // runs — `assemble_assignment_tags`, `Draft::new`, `Publication::prepare`
    // — against the real pinned NIP-BW corpus, with no network. A scripted
    // caller that reached this exact code path directly is refused here
    // exactly as it would be refused through the UI: there is no separate,
    // weaker gate.
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
    fn tag_value<'a>(event: &'a Value, name: &str) -> Option<&'a str> {
        event["tags"].as_array()?.iter().find(|t| t[0] == name)?[1].as_str()
    }
    /// Rebuild the exact candidate `submit_project_bw_assignment` would sign
    /// for one fixture kind:1 assignment event, through this file's own tag
    /// assembly rather than lifted verbatim from the fixture.
    fn candidate_for(f: &Value, label: &str) -> Value {
        let e = &f["events"][label]["event"];
        let issue = tag_value(e, "e").expect("issue tag").to_owned();
        let repo = tag_value(e, "a").expect("repo tag").to_owned();
        let delegate = tag_value(e, "p").expect("delegate tag").to_owned();
        let operation = tag_value(e, "t").expect("operation tag").to_owned();
        let prior = tag_value(e, "prior").map(str::to_owned);
        let tags = assemble_assignment_tags(&issue, &repo, &delegate, &operation, prior.as_deref());
        json!({
            "pubkey": e["pubkey"],
            "created_at": e["created_at"],
            "kind": 1,
            "tags": tags,
            "content": e["content"],
        })
    }

    #[test]
    fn first_assignment_on_a_backlog_issue_is_accepted() {
        let f = fixtures();
        let consumer = case(
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
            ],
        );
        let candidate = candidate_for(&f, "assign_a");
        Publication::prepare(&consumer, candidate).expect("assignment accepted");
    }

    #[test]
    fn a_second_assignment_chained_over_an_already_active_one_is_refused_as_a_wrong_operation() {
        let f = fixtures();
        // `assign_a` is already the active assignment. A second `assignment`
        // op that correctly chains from it (rather than an `unassignment`
        // first) is a double-assignment: refused by causality, not signer
        // role, and distinct from the unrelated-fork case above because this
        // candidate is the sole, correctly-chained child of the real head.
        let consumer = case(
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
            ],
        );
        let assign_a = &f["events"]["assign_a"]["event"];
        let issue = assign_a["tags"][0][1].as_str().expect("issue");
        let repo = assign_a["tags"][1][1].as_str().expect("repo");
        let assign_a_id = assign_a["id"].as_str().expect("assign_a id");
        let tags = assemble_assignment_tags(
            issue,
            repo,
            // A different delegate: still rejected — the wrong-operation
            // check fires on the prior op's own `t`, independent of `p`.
            &"3".repeat(64),
            "assignment",
            Some(assign_a_id),
        );
        let double = json!({
            "pubkey": assign_a["pubkey"],
            "created_at": assign_a["created_at"].as_u64().unwrap() + 10,
            "kind": 1,
            "tags": tags,
            "content": "Double assignment attempt",
        });
        let refusal = Publication::prepare(&consumer, double).expect_err("double assignment");
        assert_eq!(
            refusal.to_string(),
            "invalid input: bw:reject:causality:assignment-operation"
        );
    }

    #[test]
    fn an_unauthorized_signer_is_refused_before_any_signing() {
        let f = fixtures();
        let consumer = case(
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
            ],
        );
        let mut candidate = candidate_for(&f, "assign_a");
        // Neither Owner nor a coordinator: `role-policy-positive`'s owner
        // pubkey lives in `f.trust.owner`; any other fixture pubkey (the
        // reporter of root_a) is not authorized to assign under this policy.
        candidate["pubkey"] = f["events"]["root_a"]["event"]["pubkey"].clone();
        let refusal = Publication::prepare(&consumer, candidate).expect_err("wrong signer");
        assert_eq!(
            refusal.to_string(),
            "invalid input: bw:reject:role:unauthorized"
        );
    }

    #[test]
    fn a_stale_prior_pointing_past_the_current_head_forks_instead_of_silently_winning() {
        let f = fixtures();
        // `unassign` already chains from `assign_a`. A second, independently
        // signed candidate that *also* claims `prior: assign_a` (instead of
        // the real current head) is a second child of the same operation —
        // a technical fork, exactly as NIP-BW.md requires ("a successor of
        // an ancestor is not silently discarded as stale... it creates a
        // fork"), never a silently accepted stale write.
        let consumer = case(
            &f,
            "assignment-switch",
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
                "unassign",
            ],
        );
        let assign_a = f["events"]["assign_a"]["event"]["id"]
            .as_str()
            .expect("assign_a id");
        let unassign = &f["events"]["unassign"]["event"];
        let tags = assemble_assignment_tags(
            unassign["tags"][0][1].as_str().expect("issue"),
            unassign["tags"][1][1].as_str().expect("repo"),
            unassign["tags"][2][1].as_str().expect("delegate"),
            "unassignment",
            Some(assign_a),
        );
        let stale = json!({
            "pubkey": unassign["pubkey"],
            // A different timestamp from the real `unassign` so this is a
            // genuinely distinct, independently-arriving candidate rather
            // than a byte-identical replay of the same event.
            "created_at": unassign["created_at"].as_u64().unwrap() + 5,
            "kind": 1,
            "tags": tags,
            "content": "Remove delegate (stale)",
        });
        let refusal = Publication::prepare(&consumer, stale).expect_err("stale prior forks");
        assert_eq!(
            refusal.to_string(),
            "invalid input: bw:conflict:causality:fork"
        );
    }

    #[test]
    fn unsupported_operations_are_refused_before_any_network_call() {
        assert!(validate_assignment_operation("assignment").is_ok());
        assert!(validate_assignment_operation("unassignment").is_ok());
        assert!(validate_assignment_operation("reassignment").is_err());
        assert!(validate_assignment_operation("").is_err());
    }

    #[test]
    fn delegate_must_be_a_normalized_hex_pubkey() {
        assert!(normalized_pubkey(&"A".repeat(64)).is_ok());
        assert_eq!(normalized_pubkey(&"A".repeat(64)).unwrap(), "a".repeat(64));
        assert!(normalized_pubkey("not-a-pubkey").is_err());
        assert!(normalized_pubkey(&"a".repeat(63)).is_err());
    }
}
