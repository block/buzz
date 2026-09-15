//! Actual command local-save bodies and the production async inbound dispatcher.
use super::*;
use crate::managed_agents::{load_teams, save_teams, team_events::build_team_event, TeamRecord};
use std::time::Duration;

fn team(f: &Fixture) -> TeamRecord {
    serde_json::from_value(serde_json::json!({
        "id": "ordering-team", "name": "Team", "persona_ids": [f.persona.id],
        "created_at": "2026-01-01T00:00:00Z", "updated_at": "2026-01-01T00:00:00Z"
    }))
    .unwrap()
}

fn event(f: &Fixture, kind: u16, value: &str, time: u64) -> Event {
    let builder = match kind {
        30175 => {
            let mut record = f.persona.clone();
            record.display_name = value.into();
            build_persona_event(&record).unwrap()
        }
        30176 => {
            let mut record = team(f);
            record.name = value.into();
            build_team_event(&record).unwrap()
        }
        30177 => {
            let mut record = f.agent.clone();
            record.parallelism = if value == "Inbound" { 7 } else { 1 };
            build_agent_event(&record).unwrap()
        }
        _ => unreachable!(),
    };
    builder
        .custom_created_at(Timestamp::from(time))
        .sign_with_keys(&f.signer.keys)
        .unwrap()
}

fn start_local(f: &Fixture, kind: u16) -> tokio::task::JoinHandle<Result<(), String>> {
    let app = f.app.handle().clone();
    let persona_input = edit_request(f);
    let team_input = serde_json::from_value(serde_json::json!({
        "id": team(f).id, "name": "Local edit", "personaIds": [f.persona.id]
    }))
    .unwrap();
    let agent_input = serde_json::from_value(serde_json::json!({
        "pubkey": f.agent.pubkey, "parallelism": 3
    }))
    .unwrap();
    tokio::spawn(async move {
        match kind {
            30175 => super::super::update::update_persona_with(persona_input, app, false)
                .await
                .map(|_| ()),
            30176 => crate::commands::teams::update_team_with(team_input, app)
                .await
                .map(|_| ()),
            30177 => crate::commands::agent_models::save_managed_agent_update(
                agent_input,
                &app,
                &app.state::<AppState>(),
            )
            .await
            .map(|_| ()),
            _ => unreachable!(),
        }
    })
}

fn start_arrival(f: &Fixture, event: Event) -> tokio::task::JoinHandle<Result<(), String>> {
    let app = f.app.handle().clone();
    let relay = f.scope.relay_url.clone();
    tokio::spawn(async move {
        super::super::inbound::reconcile_async_for_test(event.as_json(), relay, app).await
    })
}

async fn done(task: tokio::task::JoinHandle<Result<(), String>>) {
    tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}

async fn deferred(task: &mut tokio::task::JoinHandle<Result<(), String>>) {
    assert!(
        tokio::time::timeout(Duration::from_millis(50), task)
            .await
            .is_err(),
        "arrival must wait, not overwrite disk or be dropped"
    );
}

fn assert_disk(f: &Fixture, kind: u16, inbound: bool) {
    match kind {
        30175 => assert_eq!(
            load_personas(f.app.handle())
                .unwrap()
                .iter()
                .find(|p| p.id == f.persona.id)
                .unwrap()
                .display_name,
            if inbound { "Inbound" } else { "Local edit" }
        ),
        30176 => assert_eq!(
            load_teams(f.app.handle())
                .unwrap()
                .iter()
                .find(|t| t.id == team(f).id)
                .unwrap()
                .name,
            if inbound { "Inbound" } else { "Local edit" }
        ),
        30177 => assert_eq!(
            load_managed_agents(f.app.handle())
                .unwrap()
                .iter()
                .find(|a| a.pubkey == f.agent.pubkey)
                .unwrap()
                .parallelism,
            if inbound { 7 } else { 3 }
        ),
        _ => unreachable!(),
    }
}

#[tokio::test]
async fn actual_local_commands_order_older_and_truly_newer_arrivals() {
    for kind in [30175, 30176, 30177] {
        for newer in [false, true] {
            let f = Fixture::new(false).await;
            save_teams(f.app.handle(), &[team(&f)]).unwrap();
            let now = Timestamp::now().as_secs();
            done(start_arrival(&f, event(&f, kind, "Old", now - 100))).await;
            let local = start_local(&f, kind);
            f.signer.wait_entered().await;
            let inbound = event(
                &f,
                kind,
                "Inbound",
                if newer { now + 100 } else { now - 50 },
            );
            let mut arrival = start_arrival(&f, inbound.clone());
            deferred(&mut arrival).await;
            f.unlocked();
            assert_disk(&f, kind, false);
            f.signer.release.notify_one();
            done(local).await;
            done(arrival).await;
            assert_disk(&f, kind, newer);
            let row = get_retained_event(
                &open_retention_db(&f.scope.db_path).unwrap(),
                kind.into(),
                &inbound.pubkey.to_hex(),
                inbound.tags.identifier().unwrap(),
            )
            .unwrap()
            .unwrap();
            assert_eq!(row.pending_sync, !newer);
            if newer {
                assert_eq!(row.raw_event, inbound.as_json());
            }
        }
    }
}

#[tokio::test]
async fn actual_local_failure_and_cancellation_release_waiting_inbound() {
    for kind in [30175, 30176, 30177] {
        for fail in [false, true] {
            let f = Fixture::new(fail).await;
            save_teams(f.app.handle(), &[team(&f)]).unwrap();
            let now = Timestamp::now().as_secs();
            done(start_arrival(&f, event(&f, kind, "Old", now - 100))).await;
            let local = start_local(&f, kind);
            f.signer.wait_entered().await;
            let mut arrival = start_arrival(&f, event(&f, kind, "Inbound", now + 100));
            deferred(&mut arrival).await;
            assert_disk(&f, kind, false); // BASE's saved retry state while signing
            f.unlocked();
            if fail {
                f.signer.release.notify_one();
                done(local).await;
            } else {
                local.abort();
                assert!(local.await.unwrap_err().is_cancelled());
            }
            done(arrival).await;
            assert_disk(&f, kind, true);
        }
    }
}

#[tokio::test]
async fn unrelated_coordinates_and_scopes_do_not_wait_for_a_local_signer() {
    let f = Fixture::new(false).await;
    let local = start_local(&f, 30175);
    f.signer.wait_entered().await;
    let mut other = f.persona.clone();
    other.id = "independent".into();
    other.display_name = "Independent".into();
    let arrival = build_persona_event(&other)
        .unwrap()
        .sign_with_keys(&f.signer.keys)
        .unwrap();
    done(start_arrival(&f, arrival)).await;
    assert!(load_personas(f.app.handle())
        .unwrap()
        .iter()
        .any(|p| p.id == other.id));
    // Same d-tag on a different active community is a different retained head.
    *f.app.state::<AppState>().relay_url_override.lock().unwrap() = Some("ws://127.0.0.1:2".into());
    let app = f.app.handle().clone();
    let arrival = event(&f, 30175, "Inbound", Timestamp::now().as_secs() + 100);
    done(tokio::spawn(async move {
        super::super::inbound::reconcile_async_for_test(
            arrival.as_json(),
            "ws://127.0.0.1:2".into(),
            app,
        )
        .await
    }))
    .await;
    f.unlocked();
    local.abort();
    assert!(local.await.unwrap_err().is_cancelled());
}

#[tokio::test]
async fn inbound_tombstone_waits_then_compares_to_the_completed_local_head() {
    for newer in [false, true] {
        let f = Fixture::new(false).await;
        let now = Timestamp::now().as_secs();
        done(start_arrival(&f, event(&f, 30175, "Old", now - 100))).await;
        let local = start_local(&f, 30175);
        f.signer.wait_entered().await;
        let tombstone = EventBuilder::new(Kind::EventDeletion, "")
            .tag(
                Tag::parse([
                    "a",
                    &format!(
                        "30175:{}:{}",
                        f.signer.keys.public_key().to_hex(),
                        crate::managed_agents::persona_events::persona_d_tag(&f.persona)
                    ),
                ])
                .unwrap(),
            )
            .custom_created_at(Timestamp::from(if newer { now + 100 } else { now - 50 }))
            .sign_with_keys(&f.signer.keys)
            .unwrap();
        let mut arrival = start_arrival(&f, tombstone);
        deferred(&mut arrival).await;
        f.signer.release.notify_one();
        done(local).await;
        done(arrival).await;
        assert_eq!(
            load_personas(f.app.handle())
                .unwrap()
                .iter()
                .any(|p| p.id == f.persona.id),
            !newer
        );
    }
}

#[tokio::test]
async fn multi_record_persona_command_reserves_not_yet_signed_linked_agents() {
    let f = Fixture::new(false).await;
    let mut first = f.agent.clone();
    first.name = f.persona.display_name.clone();
    let mut second = first.clone();
    second.pubkey = Keys::generate().public_key().to_hex();
    save_managed_agents(f.app.handle(), &[first.clone(), second.clone()]).unwrap();
    let now = Timestamp::now().as_secs();
    let local = start_local(&f, 30175);
    f.signer.wait_entered().await;
    f.signer.release.notify_one(); // persona commits before the linked saves
    f.signer.wait_entered().await; // first linked agent signs, second is queued
    let mut arrivals = Vec::new();
    for mut record in [first, second] {
        record.name = "Older inbound".into();
        let event = build_agent_event(&record)
            .unwrap()
            .custom_created_at(Timestamp::from(now - 50))
            .sign_with_keys(&f.signer.keys)
            .unwrap();
        let mut arrival = start_arrival(&f, event);
        deferred(&mut arrival).await;
        arrivals.push(arrival);
    }
    f.unlocked();
    f.signer.release.notify_one();
    f.signer.wait_entered().await;
    f.signer.release.notify_one();
    done(local).await;
    for arrival in arrivals {
        done(arrival).await;
    }
    assert!(load_managed_agents(f.app.handle())
        .unwrap()
        .iter()
        .all(|r| r.name == "Local edit"));
}

#[tokio::test]
async fn overlapping_local_commands_keep_arrival_deferred_until_all_plans_end() {
    let f = Fixture::new(false).await;
    let first = start_local(&f, 30175);
    f.signer.wait_entered().await;
    let second = start_local(&f, 30175);
    f.signer.wait_entered().await;
    let mut arrival = start_arrival(
        &f,
        event(&f, 30175, "Inbound", Timestamp::now().as_secs() - 50),
    );
    deferred(&mut arrival).await;
    first.abort();
    assert!(first.await.unwrap_err().is_cancelled());
    deferred(&mut arrival).await; // releasing one plan must not release the other
    f.signer.release.notify_one();
    done(second).await;
    done(arrival).await;
    assert_disk(&f, 30175, false);
}
