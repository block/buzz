//! Postgres-backed security regression tests for WF-08 approval decisions.
//!
//! Every test drives the production seams: gates are created by running the
//! real workflow executor and `finalize_run`, decisions go through the real
//! router (NIP-98 signed `POST .../decide`) or the real Nostr ingest path
//! (kinds 46030/46031), and assertions read persisted rows back.
//!
//! Mutations these tests were run against (each turns at least one RED):
//! - unsupported specs authorize in `approver::authorize_decision` (the
//!   original P1, `return true` for non-pubkey specs);
//! - the HTTP decide path skips `authorize_approval_decision`;
//! - the Nostr grant/deny path skips it;
//! - the decision compare-and-set result is ignored;
//! - the decide endpoint accepts `X-Pubkey` or a body its NIP-98 payload tag
//!   does not cover;
//! - the central check stops requiring channel read access.

use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use buzz_core::channel::MemberRole;
use buzz_core::TenantContext;
use buzz_db::workflow::{ApprovalStatus, RunStatus};
use nostr::{EventBuilder, Keys, Kind, Tag};
use serde_json::{json, Value};
use tower::ServiceExt;
use uuid::Uuid;

use crate::handlers::group_state_postgres_tests::{create_channel, http_auth};
use crate::handlers::ingest::ingest_event;
use crate::state::AppState;

struct Fixture {
    state: Arc<AppState>,
    tenant: TenantContext,
    host: String,
    channel: Uuid,
    owner: Keys,
    admin: Keys,
    approver: Keys,
    member: Keys,
    initiator: Keys,
    outsider: Keys,
}

async fn state() -> Arc<AppState> {
    crate::api::bridge::postgres_tests::bridge_handler_test_state()
        .await
        .expect("local Postgres and Redis")
}

/// A private channel whose owner is `owner`, with an admin, the designated
/// approver, an ordinary member, and the member who triggers runs.
async fn fixture() -> Fixture {
    let state = state().await;
    let host = format!("wf-approvals-{}.local", Uuid::new_v4().simple());
    let community = state
        .db
        .ensure_configured_community(&host)
        .await
        .expect("ensure community")
        .id;
    let tenant = TenantContext::resolved(community, &host);
    let (owner, admin, approver, member, initiator, outsider) = (
        Keys::generate(),
        Keys::generate(),
        Keys::generate(),
        Keys::generate(),
        Keys::generate(),
        Keys::generate(),
    );
    for keys in [&owner, &admin, &approver, &member, &initiator, &outsider] {
        state
            .db
            .ensure_user(community, &keys.public_key().to_bytes())
            .await
            .expect("ensure user");
    }
    let channel = create_channel(&state, &tenant, &owner, &[&["visibility", "private"]]).await;
    for (keys, role) in [
        (&admin, MemberRole::Admin),
        (&approver, MemberRole::Member),
        (&member, MemberRole::Member),
        (&initiator, MemberRole::Member),
    ] {
        state
            .db
            .add_member(
                community,
                channel,
                &keys.public_key().to_bytes(),
                role,
                Some(&owner.public_key().to_bytes()),
            )
            .await
            .expect("add member");
    }
    Fixture {
        state,
        tenant,
        host,
        channel,
        owner,
        admin,
        approver,
        member,
        initiator,
        outsider,
    }
}

/// A two-step workflow: an approval gate, then a short delay so a resumed run
/// visibly reaches `Completed`.
fn gate_yaml(from: &str) -> String {
    format!(
        "name: Gate\ntrigger:\n  on: webhook\nsteps:\n  - id: gate\n    action: request_approval\n    from: \"{from}\"\n    message: Approve?\n    timeout: 1h\n  - id: after\n    action: delay\n    duration: 1s\n"
    )
}

struct Gate {
    workflow: Uuid,
    run: Uuid,
    approval_ref: String,
}

/// Store `definition_json` as a workflow in the fixture channel and start a
/// run triggered by `f.initiator`; returns the run id.
async fn start_run(f: &Fixture, definition_json: &str) -> (Uuid, Uuid) {
    let community = f.tenant.community();
    let workflow = f
        .state
        .db
        .create_workflow(
            community,
            Some(f.channel),
            &f.owner.public_key().to_bytes(),
            &format!("wf-{}", Uuid::new_v4().simple()),
            definition_json,
            &[0; 32],
        )
        .await
        .expect("create workflow");
    let trigger = buzz_workflow::executor::TriggerContext {
        author: f.initiator.public_key().to_hex(),
        ..Default::default()
    };
    let trigger_json = serde_json::to_value(&trigger).unwrap();
    let run = f
        .state
        .db
        .create_workflow_run(community, workflow, None, Some(&trigger_json))
        .await
        .expect("create run");
    let def: buzz_workflow::WorkflowDef = serde_json::from_str(definition_json).unwrap();
    let engine = &f.state.workflow_engine;
    let result =
        buzz_workflow::executor::execute_from_step(engine, community, run, &def, &trigger, 0, None)
            .await;
    engine.finalize_run(community, run, result, None).await;
    (workflow, run)
}

/// Run a gate workflow through the real executor until it parks at the gate.
async fn park_gate(f: &Fixture, from: &str) -> Gate {
    let (_, json) = buzz_workflow::WorkflowEngine::parse_yaml(&gate_yaml(from)).expect("valid");
    let (workflow, run) = start_run(f, &json).await;
    let approvals = f
        .state
        .db
        .get_run_approvals(f.tenant.community(), workflow, run)
        .await
        .expect("approvals");
    assert_eq!(approvals.len(), 1, "run must park at exactly one gate");
    assert_eq!(run_status(f, run).await, RunStatus::WaitingApproval);
    Gate {
        workflow,
        run,
        approval_ref: hex::encode(&approvals[0].token),
    }
}

fn decide_path(gate: &Gate) -> String {
    format!(
        "/workflows/{}/runs/{}/approvals/{}/decide",
        gate.workflow, gate.run, gate.approval_ref
    )
}

/// POST `body` to `path` signed by `keys` (NIP-98 with payload hash).
async fn post_signed(
    state: &Arc<AppState>,
    f: &Fixture,
    keys: &Keys,
    path: &str,
    body: &[u8],
) -> (StatusCode, Value) {
    let url = crate::api::bridge::nip98_expected_url(&state.config.relay_url, &f.tenant, path);
    let auth = crate::nip98::nip98_header(keys, &url, body).expect("nip98");
    send(
        state,
        Request::post(path)
            .header(header::HOST, &f.host)
            .header(header::AUTHORIZATION, auth)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body.to_vec()))
            .expect("request"),
    )
    .await
}

async fn send(state: &Arc<AppState>, request: Request<Body>) -> (StatusCode, Value) {
    let response = crate::router::build_router(state.clone())
        .oneshot(request)
        .await
        .expect("router");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .expect("body");
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

async fn decide_on(
    state: &Arc<AppState>,
    f: &Fixture,
    keys: &Keys,
    gate: &Gate,
    decision: &str,
) -> (StatusCode, Value) {
    let body = serde_json::to_vec(&json!({ "decision": decision, "note": "n" })).unwrap();
    post_signed(state, f, keys, &decide_path(gate), &body).await
}

async fn decide(f: &Fixture, keys: &Keys, gate: &Gate, decision: &str) -> StatusCode {
    decide_on(&f.state, f, keys, gate, decision).await.0
}

async fn approval(f: &Fixture, gate: &Gate) -> buzz_db::workflow::ApprovalRecord {
    f.state
        .db
        .get_approval_by_stored_hash(
            f.tenant.community(),
            &hex::decode(&gate.approval_ref).unwrap(),
        )
        .await
        .expect("approval")
}

async fn run_status(f: &Fixture, run: Uuid) -> RunStatus {
    f.state
        .db
        .get_workflow_run(f.tenant.community(), run)
        .await
        .expect("run")
        .status
}

/// Wait for a resumed run to leave `Running`/`WaitingApproval`.
async fn settled_run(f: &Fixture, run: Uuid) -> buzz_db::workflow::WorkflowRunRecord {
    for _ in 0..100 {
        let record = f
            .state
            .db
            .get_workflow_run(f.tenant.community(), run)
            .await
            .expect("run");
        if !matches!(
            record.status,
            RunStatus::Running | RunStatus::WaitingApproval
        ) {
            return record;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("run {run} did not settle");
}

/// The gate is still pending, undecided, and the run is still parked.
async fn assert_untouched(f: &Fixture, gate: &Gate, case: &str) {
    let record = approval(f, gate).await;
    assert_eq!(record.status, ApprovalStatus::Pending, "{case}: status");
    assert_eq!(record.approver_pubkey, None, "{case}: approver");
    assert_eq!(record.note, None, "{case}: note");
    assert_eq!(
        run_status(f, gate.run).await,
        RunStatus::WaitingApproval,
        "{case}: run"
    );
}

/// The review's P1, end to end: nobody but the named pubkey may grant OR deny
/// — not channel members, not the run's initiator, not the workflow owner,
/// not a channel admin — and a refused decision writes nothing and resumes
/// nothing. The designated approver's grant persists and resumes the run;
/// repeats and flips are refused afterwards.
#[tokio::test]
#[ignore = "requires Postgres and Redis"]
async fn only_the_designated_pubkey_decides() {
    let f = fixture().await;
    let gate = park_gate(&f, &f.approver.public_key().to_hex()).await;

    for (keys, who) in [
        (&f.member, "member"),
        (&f.initiator, "initiator"),
        (&f.owner, "workflow/channel owner"),
        (&f.admin, "channel admin"),
    ] {
        for decision in ["granted", "denied"] {
            let (status, body) = decide_on(&f.state, &f, keys, &gate, decision).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{who} {decision}");
            assert_eq!(body["error"], "approver is not authorized for this gate");
        }
    }
    for decision in ["granted", "denied"] {
        assert_eq!(
            decide(&f, &f.outsider, &gate, decision).await,
            StatusCode::NOT_FOUND,
            "outsider {decision} sees no workflow"
        );
    }
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_untouched(&f, &gate, "after refused decisions").await;

    let (status, body) = decide_on(&f.state, &f, &f.approver, &gate, "granted").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["status"], "resumed");
    let record = approval(&f, &gate).await;
    assert_eq!(record.status, ApprovalStatus::Granted);
    assert_eq!(
        record.approver_pubkey.as_deref(),
        Some(f.approver.public_key().to_bytes().as_slice())
    );
    let run = settled_run(&f, gate.run).await;
    assert_eq!(run.status, RunStatus::Completed, "{:?}", run.error_message);

    // Repeating or flipping a decided gate is refused and changes nothing.
    for (keys, decision) in [
        (&f.approver, "granted"),
        (&f.approver, "denied"),
        (&f.member, "denied"),
    ] {
        let status = decide(&f, keys, &gate, decision).await;
        assert!(
            status == StatusCode::CONFLICT || status == StatusCode::FORBIDDEN,
            "{decision}: {status}"
        );
    }
    assert_eq!(
        decide(&f, &f.approver, &gate, "denied").await,
        StatusCode::CONFLICT
    );
    assert_eq!(approval(&f, &gate).await.status, ApprovalStatus::Granted);
    assert_eq!(run_status(&f, gate.run).await, RunStatus::Completed);
}

/// An authorized denial persists, fails the run (never resumes it as
/// success), and is final.
#[tokio::test]
#[ignore = "requires Postgres and Redis"]
async fn authorized_denial_fails_the_run_and_is_final() {
    let f = fixture().await;
    let gate = park_gate(&f, &f.approver.public_key().to_hex()).await;

    let (status, body) = decide_on(&f.state, &f, &f.approver, &gate, "denied").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["status"], "denied");
    assert_eq!(approval(&f, &gate).await.status, ApprovalStatus::Denied);
    let run = f
        .state
        .db
        .get_workflow_run(f.tenant.community(), gate.run)
        .await
        .unwrap();
    assert_eq!(run.status, RunStatus::Failed);
    assert_eq!(run.error_code.as_deref(), Some("approval_denied"));

    assert_eq!(
        decide(&f, &f.approver, &gate, "granted").await,
        StatusCode::CONFLICT
    );
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(approval(&f, &gate).await.status, ApprovalStatus::Denied);
    assert_eq!(run_status(&f, gate.run).await, RunStatus::Failed);
}

/// Gates persisted with a role or empty spec (definitions stored before
/// definition-time validation, or rows written by older code) fail closed for
/// everyone, on both decisions. Supplying the role's name grants nothing.
#[tokio::test]
#[ignore = "requires Postgres and Redis"]
async fn legacy_role_and_empty_gates_fail_closed() {
    let f = fixture().await;
    for spec in ["@manager", "@admin", "@owner", "", "manager"] {
        // Park a real gate, then rewrite its spec as a legacy row would hold it.
        let gate = park_gate(&f, "any").await;
        sqlx::query("UPDATE workflow_approvals SET approver_spec = $1 WHERE token = $2")
            .bind(spec)
            .bind(hex::decode(&gate.approval_ref).unwrap())
            .execute(f.state.db.pool())
            .await
            .expect("rewrite spec");
        for keys in [&f.owner, &f.admin, &f.approver, &f.member, &f.initiator] {
            for decision in ["granted", "denied"] {
                let (status, body) = decide_on(&f.state, &f, keys, &gate, decision).await;
                assert_eq!(status, StatusCode::FORBIDDEN, "{spec:?} {decision}");
                assert_eq!(
                    body["error"],
                    "approver spec for this gate is not supported; nobody may decide it"
                );
            }
        }
        assert_untouched(&f, &gate, spec).await;
    }
}

/// A stored definition that still names a role never parks a gate: the run
/// fails at gate creation with a clear error instead of waiting on an
/// approval nobody (or everybody) could give.
#[tokio::test]
#[ignore = "requires Postgres and Redis"]
async fn legacy_role_definition_fails_the_run_instead_of_parking() {
    let f = fixture().await;
    let (_, json) = buzz_workflow::WorkflowEngine::parse_yaml(&gate_yaml("any")).unwrap();
    let legacy = json.replace("\"from\":\"any\"", "\"from\":\"@manager\"");
    assert_ne!(legacy, json);
    let (workflow, run) = start_run(&f, &legacy).await;
    let record = f
        .state
        .db
        .get_workflow_run(f.tenant.community(), run)
        .await
        .unwrap();
    assert_eq!(record.status, RunStatus::Failed);
    assert_eq!(record.error_code.as_deref(), Some("invalid_definition"));
    assert!(
        record
            .error_message
            .as_deref()
            .unwrap_or("")
            .contains("not supported"),
        "{:?}",
        record.error_message
    );
    let approvals = f
        .state
        .db
        .get_run_approvals(f.tenant.community(), workflow, run)
        .await
        .unwrap();
    assert!(approvals.is_empty());
}

/// `any` is the one explicit opt-in: channel readers may decide, outsiders
/// still may not.
#[tokio::test]
#[ignore = "requires Postgres and Redis"]
async fn any_gate_admits_channel_readers_only() {
    let f = fixture().await;
    let gate = park_gate(&f, "any").await;
    assert_eq!(
        decide(&f, &f.outsider, &gate, "granted").await,
        StatusCode::NOT_FOUND
    );
    assert_untouched(&f, &gate, "outsider").await;
    assert_eq!(
        decide(&f, &f.member, &gate, "granted").await,
        StatusCode::OK
    );
    assert_eq!(settled_run(&f, gate.run).await.status, RunStatus::Completed);
}

/// The decide endpoint changes state, so it follows the member-POST rule:
/// the X-Pubkey dev header cannot claim an identity, and a NIP-98 signature
/// covers the exact body (a signed grant cannot be replayed as a deny).
#[tokio::test]
#[ignore = "requires Postgres and Redis"]
async fn decide_requires_nip98_bound_to_the_body() {
    let f = fixture().await;
    let gate = park_gate(&f, &f.approver.public_key().to_hex()).await;
    let path = decide_path(&gate);
    let deny = serde_json::to_vec(&json!({ "decision": "denied" })).unwrap();

    let (status, _) = send(
        &f.state,
        Request::post(&path)
            .header(header::HOST, &f.host)
            .header("x-pubkey", f.approver.public_key().to_hex())
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(deny.clone()))
            .expect("request"),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "X-Pubkey must not authenticate"
    );

    let grant = serde_json::to_vec(&json!({ "decision": "granted" })).unwrap();
    let url = crate::api::bridge::nip98_expected_url(&f.state.config.relay_url, &f.tenant, &path);
    let signed_for_grant = crate::nip98::nip98_header(&f.approver, &url, &grant).unwrap();
    let (status, _) = send(
        &f.state,
        Request::post(&path)
            .header(header::HOST, &f.host)
            .header(header::AUTHORIZATION, signed_for_grant)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(deny))
            .expect("request"),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "payload hash must bind the body"
    );
    assert_untouched(&f, &gate, "unauthenticated attempts").await;

    // Malformed bodies are refused after auth and change nothing.
    for body in [
        &b"{\"decision\":\"maybe\"}"[..],
        b"not json",
        b"{\"decision\":\"granted\",\"approver\":\"x\"}",
    ] {
        let (status, _) = post_signed(&f.state, &f, &f.approver, &path, body).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }
    assert_untouched(&f, &gate, "malformed bodies").await;
}

/// A gate's ref only works under its own workflow and run.
#[tokio::test]
#[ignore = "requires Postgres and Redis"]
async fn decision_is_bound_to_its_workflow_and_run() {
    let f = fixture().await;
    let approver = f.approver.public_key().to_hex();
    let first = park_gate(&f, &approver).await;
    let second = park_gate(&f, &approver).await;
    for misrouted in [
        Gate {
            workflow: first.workflow,
            run: second.run,
            approval_ref: first.approval_ref.clone(),
        },
        Gate {
            workflow: second.workflow,
            run: second.run,
            approval_ref: first.approval_ref.clone(),
        },
        Gate {
            workflow: second.workflow,
            run: first.run,
            approval_ref: first.approval_ref.clone(),
        },
    ] {
        assert_eq!(
            decide(&f, &f.approver, &misrouted, "granted").await,
            StatusCode::NOT_FOUND
        );
    }
    assert_untouched(&f, &first, "first").await;
    assert_untouched(&f, &second, "second").await;
}

/// Concurrent grants and denials from the approver, raced against refused
/// attempts from a member: exactly one decision wins, everyone else is
/// refused, and the run ends in the state the winner chose.
#[tokio::test]
#[ignore = "requires Postgres and Redis"]
async fn concurrent_decisions_have_one_winner() {
    let f = fixture().await;
    let gate = park_gate(&f, &f.approver.public_key().to_hex()).await;
    let attempts = (0..12).map(|i| {
        let (keys, decision) = match i % 3 {
            0 => (&f.approver, "granted"),
            1 => (&f.approver, "denied"),
            _ => (&f.member, "granted"),
        };
        let f = &f;
        let gate = &gate;
        async move {
            (
                keys.public_key(),
                decision,
                decide(f, keys, gate, decision).await,
            )
        }
    });
    let results = futures::future::join_all(attempts).await;
    let winners: Vec<_> = results
        .iter()
        .filter(|(_, _, s)| *s == StatusCode::OK)
        .collect();
    assert_eq!(winners.len(), 1, "{results:?}");
    let (winner_key, winner_decision, _) = winners[0];
    assert_eq!(*winner_key, f.approver.public_key());
    for (key, _, status) in &results {
        if *key == f.member.public_key() {
            assert_eq!(*status, StatusCode::FORBIDDEN);
        } else if *status != StatusCode::OK {
            assert_eq!(*status, StatusCode::CONFLICT);
        }
    }
    let record = approval(&f, &gate).await;
    let run = settled_run(&f, gate.run).await;
    if *winner_decision == "granted" {
        assert_eq!(record.status, ApprovalStatus::Granted);
        assert_eq!(run.status, RunStatus::Completed);
    } else {
        assert_eq!(record.status, ApprovalStatus::Denied);
        assert_eq!(run.status, RunStatus::Failed);
    }
}

/// A gate parked by one relay process is enforced identically by a fresh one
/// (new engine, new caches): authorization reads only persisted state.
#[tokio::test]
#[ignore = "requires Postgres and Redis"]
async fn restart_does_not_weaken_authorization() {
    let f = fixture().await;
    let gate = park_gate(&f, &f.approver.public_key().to_hex()).await;
    let restarted = state().await;
    for keys in [&f.member, &f.initiator, &f.owner] {
        assert_eq!(
            decide_on(&restarted, &f, keys, &gate, "granted").await.0,
            StatusCode::FORBIDDEN
        );
    }
    assert_untouched(&f, &gate, "after restart").await;
    assert_eq!(
        decide_on(&restarted, &f, &f.approver, &gate, "granted")
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(settled_run(&f, gate.run).await.status, RunStatus::Completed);
}

/// Expiry is applied only on an authorized attempt: an unauthorized request
/// cannot write `expired`/`failed`; the approver's attempt expires the gate
/// and fails the run.
#[tokio::test]
#[ignore = "requires Postgres and Redis"]
async fn expired_gate_is_failed_only_by_an_authorized_attempt() {
    let f = fixture().await;
    let gate = park_gate(&f, &f.approver.public_key().to_hex()).await;
    sqlx::query(
        "UPDATE workflow_approvals SET expires_at = NOW() - INTERVAL '1 minute' WHERE token = $1",
    )
    .bind(hex::decode(&gate.approval_ref).unwrap())
    .execute(f.state.db.pool())
    .await
    .unwrap();
    assert_eq!(
        decide(&f, &f.member, &gate, "granted").await,
        StatusCode::FORBIDDEN
    );
    assert_untouched(&f, &gate, "unauthorized on expired gate").await;

    assert_eq!(
        decide(&f, &f.approver, &gate, "granted").await,
        StatusCode::GONE
    );
    assert_eq!(approval(&f, &gate).await.status, ApprovalStatus::Expired);
    let run = f
        .state
        .db
        .get_workflow_run(f.tenant.community(), gate.run)
        .await
        .unwrap();
    assert_eq!(run.status, RunStatus::Failed);
    assert_eq!(run.error_code.as_deref(), Some("approval_expired"));
    assert_eq!(
        decide(&f, &f.approver, &gate, "granted").await,
        StatusCode::CONFLICT
    );
}

/// Ingest a grant (46030) or deny (46031) command naming the gate.
async fn command(f: &Fixture, keys: &Keys, gate: &Gate, kind: u16) -> Result<(), String> {
    let event = EventBuilder::new(Kind::Custom(kind), "")
        .tags([Tag::parse(["d", &gate.approval_ref]).unwrap()])
        .sign_with_keys(keys)
        .unwrap();
    match ingest_event(&f.state, &f.tenant, event, http_auth(keys)).await {
        Ok(result) if result.accepted => Ok(()),
        Ok(result) => Err(result.message),
        Err(error) => Err(format!("{error:?}")),
    }
}

/// The Nostr grant/deny command kinds are an alternate way to change approval
/// state; they enforce the same policy, and a decision on one path is final
/// on the other.
#[tokio::test]
#[ignore = "requires Postgres and Redis"]
async fn nostr_command_path_enforces_the_same_policy() {
    let f = fixture().await;
    let gate = park_gate(&f, &f.approver.public_key().to_hex()).await;
    for kind in [46030, 46031] {
        for keys in [&f.member, &f.initiator, &f.owner, &f.admin] {
            let error = command(&f, keys, &gate, kind).await.unwrap_err();
            assert!(error.contains("forbidden"), "kind {kind}: {error}");
        }
        let error = command(&f, &f.outsider, &gate, kind).await.unwrap_err();
        assert!(error.contains("approval not found"), "kind {kind}: {error}");
    }
    assert_untouched(&f, &gate, "refused commands").await;

    let role_gate = park_gate(&f, "any").await;
    sqlx::query("UPDATE workflow_approvals SET approver_spec = '@manager' WHERE token = $1")
        .bind(hex::decode(&role_gate.approval_ref).unwrap())
        .execute(f.state.db.pool())
        .await
        .unwrap();
    for keys in [&f.member, &f.owner, &f.admin, &f.approver] {
        let error = command(&f, keys, &role_gate, 46030).await.unwrap_err();
        assert!(error.contains("not supported"), "{error}");
    }
    assert_untouched(&f, &role_gate, "role gate via command").await;

    command(&f, &f.approver, &gate, 46030)
        .await
        .expect("approver grant");
    assert_eq!(approval(&f, &gate).await.status, ApprovalStatus::Granted);
    assert_eq!(settled_run(&f, gate.run).await.status, RunStatus::Completed);
    assert_eq!(
        decide(&f, &f.approver, &gate, "denied").await,
        StatusCode::CONFLICT
    );
    // Repeats and flips over the command kinds change nothing either, and an
    // unauthorized caller is refused before learning the gate's state.
    for kind in [46030, 46031] {
        let error = command(&f, &f.approver, &gate, kind).await.unwrap_err();
        assert!(error.contains("already granted"), "kind {kind}: {error}");
        let error = command(&f, &f.member, &gate, kind).await.unwrap_err();
        assert!(error.contains("forbidden"), "kind {kind}: {error}");
    }
    let record = approval(&f, &gate).await;
    assert_eq!(record.status, ApprovalStatus::Granted);
    assert_eq!(
        record.approver_pubkey,
        Some(f.approver.public_key().to_bytes().to_vec())
    );

    // An authorized denial over the command kinds ends the run without
    // resuming it, and is final.
    let denied = park_gate(&f, &f.approver.public_key().to_hex()).await;
    command(&f, &f.approver, &denied, 46031)
        .await
        .expect("approver deny");
    assert_eq!(approval(&f, &denied).await.status, ApprovalStatus::Denied);
    let run = settled_run(&f, denied.run).await;
    assert_eq!(run.status, RunStatus::Cancelled);
    assert_eq!(run.error_code.as_deref(), Some("approval_denied"));
    let error = command(&f, &f.approver, &denied, 46030).await.unwrap_err();
    assert!(error.contains("already denied"), "{error}");
    assert_eq!(
        decide(&f, &f.approver, &denied, "granted").await,
        StatusCode::CONFLICT
    );
    assert_eq!(approval(&f, &denied).await.status, ApprovalStatus::Denied);
}
