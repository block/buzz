//! Adoption-path retention: pending 30175/30176 enqueue (Wes/Carl P1).
//!
//! A successful adoption must leave pending retention rows so a crash before
//! the next boot reconcile cannot lose the only adopted copy. `plan_add` marks
//! which rows the add wrote (`retain_personas`); `commit_and_enqueue` — the
//! sole route to a durable adoption commit — writes the stores and, only once
//! that commit succeeds, enqueues those personas plus the team. These tests
//! drive the whole sequence (commit → scope resolve → enqueue) through
//! `commit_and_enqueue` with a real temp-dir scope and a spy commit, so they go
//! RED if the enqueue is deleted from the seam and prove the enqueue is gated
//! on a successful commit — the connection the isolated helper could not show.

use super::super::apply::{commit_and_enqueue, plan_add};
use super::*;
use crate::managed_agents::persona_events::persona_d_tag;
use crate::managed_agents::retention::{
    get_pending_sync, open_retention_db, scoped_retention_db_path, RetainedEvent, RetentionScope,
};
use buzz_core_pkg::kind::{KIND_PERSONA, KIND_TEAM};
use std::cell::Cell;

const RELAY: &str = "wss://relay.example";

/// A retention scope rooted in a fresh temp dir, owned by fresh keys.
fn scope(dir: &std::path::Path) -> RetentionScope {
    let keys = nostr::Keys::generate();
    let db_path = scoped_retention_db_path(dir, RELAY, &keys.public_key().to_hex());
    std::fs::create_dir_all(db_path.parent().unwrap()).unwrap();
    RetentionScope {
        db_path,
        relay_url: RELAY.to_string(),
        owner_keys: keys,
    }
}

fn clone_scope(scope: &RetentionScope) -> RetentionScope {
    RetentionScope {
        db_path: scope.db_path.clone(),
        relay_url: scope.relay_url.clone(),
        owner_keys: scope.owner_keys.clone(),
    }
}

fn pending(scope: &RetentionScope) -> Vec<RetainedEvent> {
    let conn = open_retention_db(&scope.db_path).unwrap();
    get_pending_sync(&conn).unwrap()
}

/// Drive `commit_and_enqueue` with a spy commit that always succeeds and a
/// scope resolver that hands back `scope`. Returns the pending rows plus
/// whether the commit ran — the full command sequencing minus the AppHandle.
fn run_adoption(
    plan: super::super::apply::AddPlan,
    scope: &RetentionScope,
) -> (Vec<RetainedEvent>, bool) {
    let committed = Cell::new(false);
    let resolved = clone_scope(scope);
    commit_and_enqueue(
        plan,
        |_personas, _teams| {
            committed.set(true);
            Ok(())
        },
        || Ok(resolved),
    )
    .unwrap();
    (pending(scope), committed.get())
}

/// A successful adoption of a two-member team commits, then enqueues a pending
/// 30175 for each minted member copy and a pending 30176 for the team.
#[test]
fn adoption_commits_then_enqueues_persona_and_team_rows() {
    let dir = tempfile::tempdir().unwrap();
    let scope = scope(dir.path());

    let source = source(&"a".repeat(64));
    let body = content(vec![
        member("m1", "Do the work."),
        member("m2", "Review the work."),
    ]);
    let plan = plan_add(&[], &[], &source, &body, NOW).unwrap();
    let (personas, _teams) = plan.stores.as_ref().expect("a fresh add writes stores");
    assert_eq!(personas.len(), 2, "two members copied");
    assert_eq!(
        plan.retain_personas.len(),
        2,
        "both minted copies must be retained"
    );
    let expected_d_tags: Vec<String> = personas.iter().map(persona_d_tag).collect();
    let team_id = plan.team.id.clone();

    let (rows, committed) = run_adoption(plan, &scope);
    assert!(committed, "a fresh add commits the stores");

    let persona_rows: Vec<_> = rows.iter().filter(|r| r.kind == KIND_PERSONA).collect();
    let team_rows: Vec<_> = rows.iter().filter(|r| r.kind == KIND_TEAM).collect();
    assert_eq!(
        persona_rows.len(),
        2,
        "each minted member gets a pending 30175 row"
    );
    assert_eq!(
        team_rows.len(),
        1,
        "the adopted team gets a pending 30176 row"
    );
    assert_eq!(team_rows[0].d_tag, team_id, "team row keyed by team id");
    assert!(
        rows.iter().all(|r| r.pending_sync),
        "every enqueued row is flagged for the flush loop"
    );
    // Each persona row is keyed by its member's d-tag — proves the minted
    // copies (not some unrelated record) were retained.
    for d_tag in &expected_d_tags {
        assert!(
            persona_rows.iter().any(|r| &r.d_tag == d_tag),
            "member {d_tag} must have a pending row"
        );
    }
}

/// A commit failure propagates and enqueues nothing: retention is gated on a
/// durable commit, so a failed adoption leaves no pending rows to publish under
/// the adopter's identity. Only reachable through the seam — the isolated
/// helper test could not express this ordering.
#[test]
fn commit_failure_enqueues_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let scope = scope(dir.path());

    let source = source(&"a".repeat(64));
    let body = content(vec![member("m1", "Do the work.")]);
    let plan = plan_add(&[], &[], &source, &body, NOW).unwrap();

    let resolver_ran = Cell::new(false);
    let resolved = clone_scope(&scope);
    let result = commit_and_enqueue(
        plan,
        |_personas, _teams| Err("disk full".to_string()),
        || {
            resolver_ran.set(true);
            Ok(resolved)
        },
    );

    assert_eq!(result.unwrap_err(), "disk full", "commit error propagates");
    assert!(
        !resolver_ran.get(),
        "a failed commit never resolves the scope or enqueues"
    );
    assert!(
        pending(&scope).is_empty(),
        "no retention rows for an add that did not commit"
    );
}

/// Idempotent replay: a plan with no stores skips the commit entirely and
/// enqueues nothing, so no duplicate or bumped rows appear on a second add.
#[test]
fn replay_skips_commit_and_enqueue() {
    let dir = tempfile::tempdir().unwrap();
    let scope = scope(dir.path());

    let source = source(&"a".repeat(64));
    let body = content(vec![member("m1", "Do the work.")]);

    // First add: mint + commit + enqueue.
    let first = plan_add(&[], &[], &source, &body, NOW).unwrap();
    let (personas, teams) = first.stores.clone().expect("first add writes stores");
    let (after_first, first_committed) = run_adoption(first, &scope);
    assert!(first_committed, "the first add commits");
    assert_eq!(after_first.len(), 2, "one persona + one team pending");

    // Replay: same publication, now present in the stores.
    let replay = plan_add(&personas, &teams, &source, &body, NOW).unwrap();
    assert!(replay.stores.is_none(), "a replay writes no stores");
    assert!(
        replay.retain_personas.is_empty(),
        "a replay retains nothing — nothing was written"
    );
    let (after_replay, replay_committed) = run_adoption(replay, &scope);
    assert!(
        !replay_committed,
        "a replay must not commit — nothing changed on disk"
    );
    assert_eq!(
        after_replay.len(),
        after_first.len(),
        "replay must not add pending rows"
    );
    let first_ids: Vec<_> = after_first.iter().map(|r| r.raw_event.clone()).collect();
    let replay_ids: Vec<_> = after_replay.iter().map(|r| r.raw_event.clone()).collect();
    assert_eq!(
        first_ids, replay_ids,
        "replay must not re-sign or bump existing rows"
    );
}

/// A reused local built-in is an untouched local record, so the add must NOT
/// enqueue a persona head for it — only the team is retained.
#[test]
fn reused_builtin_is_not_retained() {
    let dir = tempfile::tempdir().unwrap();
    let scope = scope(dir.path());

    let local = crate::managed_agents::built_in_persona_definition("builtin:fizz", NOW)
        .expect("builtin:fizz must exist");
    let t = team_fixture(vec![local.id.clone()]);
    let keys = nostr::Keys::generate();
    let event = build_team_catalog_event(&t, std::slice::from_ref(&local), true)
        .expect("real built-in projects within the size contract")
        .sign_with_keys(&keys)
        .unwrap();
    let src = source(&keys.public_key().to_hex());
    let body = crate::managed_agents::team_catalog::team_catalog_content_from_event(&event)
        .expect("projected event must parse");

    let plan = plan_add(std::slice::from_ref(&local), &[], &src, &body, NOW).unwrap();
    assert!(
        plan.retain_personas.is_empty(),
        "a reused built-in is untouched and must not be re-published under the adopter"
    );

    let (rows, committed) = run_adoption(plan, &scope);
    assert!(committed, "the add still commits the new team record");
    assert!(
        !rows.iter().any(|r| r.kind == KIND_PERSONA),
        "no persona head is enqueued for a reused built-in"
    );
    assert_eq!(
        rows.iter().filter(|r| r.kind == KIND_TEAM).count(),
        1,
        "the adopted team is still retained"
    );
}

/// A reactivated existing copy (revived from an earlier team delete) flips a
/// persisted field, so it must be re-retained.
#[test]
fn reactivated_copy_is_retained() {
    let dir = tempfile::tempdir().unwrap();
    let scope = scope(dir.path());

    let source = source(&"a".repeat(64));
    let body = content(vec![member("m1", "Do the work.")]);
    // Seed an existing, deactivated copy (what delete_team_with_cascade
    // leaves behind).
    let (mut personas, _) = plan_add(&[], &[], &source, &body, NOW)
        .unwrap()
        .stores
        .unwrap();
    personas[0].is_active = false;

    let plan = plan_add(&personas, &[], &source, &body, NOW).unwrap();
    assert_eq!(
        plan.retain_personas.len(),
        1,
        "a reactivated copy must be retained"
    );
    assert!(
        plan.retain_personas[0].is_active,
        "the retained row reflects the reactivation"
    );

    let (rows, committed) = run_adoption(plan, &scope);
    assert!(committed, "reactivation writes the flipped field");
    assert_eq!(
        rows.iter().filter(|r| r.kind == KIND_PERSONA).count(),
        1,
        "the reactivated copy is enqueued"
    );
}

/// Partial-commit crash recovery (Carl/Wes P1): the first adoption wrote the
/// member persona but crashed before post-commit retention, so the copy is
/// active on disk with NO 30175 retention row and the team was never written.
/// The recovery retry must enqueue the missing member 30175 AND the team 30176
/// — otherwise the adopted member's head is lost forever.
///
/// Before the fix, `resolve_member` returned `retain: false` for an
/// already-active provenance match, so the retry enqueued only the team and the
/// member copy never got its 30175. This drives the seam end-to-end: seed only
/// the active persona (no team, no retention row), retry through
/// `commit_and_enqueue`, and assert both pending heads appear.
#[test]
fn partial_commit_retry_enqueues_the_orphaned_member_head() {
    let dir = tempfile::tempdir().unwrap();
    let scope = scope(dir.path());

    let source = source(&"a".repeat(64));
    let body = content(vec![member("m1", "Do the work.")]);

    // First attempt's on-disk residue: the member persona was written and is
    // active, but the team row and the retention rows never landed (the crash
    // was between the persona write and post-commit retention).
    let (personas_after_crash, _teams) = plan_add(&[], &[], &source, &body, NOW)
        .unwrap()
        .stores
        .unwrap();
    assert!(
        personas_after_crash[0].is_active,
        "the orphaned copy is active — the crash was after the persona write"
    );
    assert!(
        pending(&scope).is_empty(),
        "no retention rows exist yet — the crash preceded post-commit retention"
    );

    // The recovery retry: team row still absent, so this is a fresh add that
    // reuses the active orphaned copy by provenance.
    let plan = plan_add(&personas_after_crash, &[], &source, &body, NOW).unwrap();
    assert!(
        plan.stores.is_some(),
        "with no team row, the retry is a real add, not a replay"
    );
    assert_eq!(
        plan.retain_personas.len(),
        1,
        "the orphaned member copy must be retained so its missing 30175 is enqueued"
    );
    let member_d_tag = persona_d_tag(&plan.retain_personas[0]);
    let team_id = plan.team.id.clone();

    let (rows, committed) = run_adoption(plan, &scope);
    assert!(committed, "the recovery retry writes the missing team row");

    let persona_rows: Vec<_> = rows.iter().filter(|r| r.kind == KIND_PERSONA).collect();
    let team_rows: Vec<_> = rows.iter().filter(|r| r.kind == KIND_TEAM).collect();
    assert_eq!(
        persona_rows.len(),
        1,
        "the orphaned member's 30175 is enqueued on retry"
    );
    assert_eq!(
        persona_rows[0].d_tag, member_d_tag,
        "the enqueued 30175 is keyed by the recovered member, not some other record"
    );
    assert_eq!(team_rows.len(), 1, "the team's 30176 is enqueued");
    assert_eq!(team_rows[0].d_tag, team_id, "team row keyed by team id");
    assert!(
        rows.iter().all(|r| r.pending_sync),
        "both recovered heads are flagged for the flush loop"
    );
}
