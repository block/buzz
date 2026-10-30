//! Read-only huddle inspection: `buzz huddles list` and `buzz huddles participants`.
//!
//! State is derived exactly as desktop derives it (see [`super::huddle_tracker`]).
//! A huddle with no valid end event is `active` until its start is older than
//! desktop's one-hour joinable window, after which it is `stale`: desktop shows
//! it as ended, but no end event proves it.

use std::collections::HashSet;

use buzz_core::kind::{
    KIND_HUDDLE_ENDED, KIND_HUDDLE_PARTICIPANT_JOINED, KIND_HUDDLE_PARTICIPANT_LEFT,
    KIND_HUDDLE_STARTED,
};
use serde_json::{json, Value};

use super::huddle_tracker::{HuddleSession, HuddleTracker, LifecycleEvent};
use crate::client::BuzzClient;
use crate::error::CliError;
use crate::validate::parse_uuid;

/// Lifecycle events scanned per command. Reaching it is reported in the output.
const MAX_LIFECYCLE_EVENTS: u32 = 5_000;

/// Matches desktop's `HUDDLE_JOINABLE_WINDOW_SECONDS`.
const JOINABLE_WINDOW_SECONDS: u64 = 60 * 60;

/// The newest lifecycle events of one channel, and whether older ones exist.
struct Window {
    events: Vec<LifecycleEvent>,
    truncated: bool,
    oldest_created_at: Option<u64>,
}

impl Window {
    fn from_events(raw: Vec<Value>, bound: u32) -> Result<Self, CliError> {
        let bound = bound as usize;
        let truncated = raw.len() > bound;
        let mut seen = HashSet::new();
        let mut events = Vec::new();
        for value in raw.iter().take(bound) {
            let event = LifecycleEvent::parse(value)?;
            if seen.insert(event.id.clone()) {
                events.push(event);
            }
        }
        let oldest_created_at = events.iter().map(|event| event.created_at).min();
        Ok(Self {
            events,
            truncated,
            oldest_created_at,
        })
    }

    /// Whether every event at or after `created_at` was scanned. The relay
    /// returns newest first, so events share the oldest scanned second only
    /// partially.
    fn covers(&self, created_at: u64) -> bool {
        !self.truncated
            || self
                .oldest_created_at
                .is_some_and(|oldest| created_at > oldest)
    }

    fn scan_json(&self, bound: u32) -> Value {
        json!({
            "truncated": self.truncated,
            "events_scanned": self.events.len(),
            "event_bound": bound,
            "liveness_checked": false,
        })
    }
}

async fn fetch_relay_self(client: &BuzzClient) -> Result<String, CliError> {
    let raw = client
        .get_public("/")
        .await
        .map_err(|e| CliError::Other(format!("failed to fetch relay info document: {e}")))?;
    let info: Value = serde_json::from_str(&raw)
        .map_err(|e| CliError::Other(format!("relay info document is not valid JSON: {e}")))?;
    let self_hex = info.get("self").and_then(Value::as_str).ok_or_else(|| {
        CliError::Other(
            "relay info document has no 'self' pubkey; huddle events cannot be authenticated"
                .into(),
        )
    })?;
    super::agents::normalize_relay_self_hex(self_hex)
}

async fn fetch_window(client: &BuzzClient, channel: &str, bound: u32) -> Result<Window, CliError> {
    let filter = json!({
        "kinds": [
            KIND_HUDDLE_STARTED,
            KIND_HUDDLE_PARTICIPANT_JOINED,
            KIND_HUDDLE_PARTICIPANT_LEFT,
            KIND_HUDDLE_ENDED,
        ],
        "#h": [channel],
    });
    let probe = bound
        .checked_add(1)
        .ok_or_else(|| CliError::Other("query bound is too large".into()))?;
    let raw = client.query_paginated(filter, probe).await?;
    Window::from_events(raw, bound)
}

fn session_state(session: &HuddleSession, now: u64) -> &'static str {
    if session.ended_at.is_some() {
        "ended"
    } else if now.saturating_sub(session.start_created_at) > JOINABLE_WINDOW_SECONDS {
        "stale"
    } else {
        "active"
    }
}

fn track(window: &Window, relay_self: &str) -> HuddleTracker {
    let mut tracker = HuddleTracker::new(relay_self);
    tracker.apply_history(&window.events);
    tracker
}

fn build_list(
    window: &Window,
    relay_self: &str,
    channel: &str,
    now: u64,
    active_only: bool,
    limit: u32,
    bound: u32,
) -> Value {
    let tracker = track(window, relay_self);
    let mut sessions: Vec<(&String, &HuddleSession)> = tracker.sessions().iter().collect();
    sessions.sort_by(|a, b| {
        b.1.start_created_at
            .cmp(&a.1.start_created_at)
            .then_with(|| a.1.start_event_id.cmp(&b.1.start_event_id))
    });
    let selected: Vec<(&String, &HuddleSession)> = sessions
        .into_iter()
        .filter(|(_, session)| !active_only || session_state(session, now) == "active")
        .take(limit as usize)
        .collect();

    // Truncation hides only older huddles; the newest `limit` are exact when
    // all of them lie wholly inside the scanned window.
    let complete = !window.truncated
        || (selected.len() == limit as usize
            && selected
                .iter()
                .all(|(_, session)| window.covers(session.start_created_at)));

    let huddles: Vec<Value> = selected
        .iter()
        .map(|(id, session)| {
            json!({
                "huddle": id,
                "channel": channel,
                "state": session_state(session, now),
                "started_at": session.start_created_at,
                "ended_at": session.ended_at,
                "started_by": session.start_creator,
                "start_event": session.start_event_id,
                "participant_count": session.participants().len(),
            })
        })
        .collect();

    let mut out = window.scan_json(bound);
    out["complete"] = json!(complete);
    out["huddles"] = json!(huddles);
    out
}

fn build_participants(
    window: &Window,
    relay_self: &str,
    channel: &str,
    huddle: &str,
    now: u64,
    bound: u32,
) -> Result<Value, CliError> {
    let tracker = track(window, relay_self);
    let mut out = window.scan_json(bound);
    out["channel"] = json!(channel);
    out["huddle"] = json!(huddle);

    let Some(session) = tracker.sessions().get(huddle) else {
        if window.truncated {
            out["state"] = json!("unknown");
            out["complete"] = json!(false);
            out["participants"] = json!([]);
            out["note"] = json!(
                "the huddle's start is older than the scanned events; its roster is not known"
            );
            return Ok(out);
        }
        return Err(CliError::NotFound(format!(
            "no huddle {huddle} found in channel {channel}"
        )));
    };

    let participants: Vec<Value> = session
        .participants()
        .into_iter()
        .map(|(pubkey, joined_at)| json!({ "pubkey": pubkey, "joined_at": joined_at }))
        .collect();
    out["state"] = json!(session_state(session, now));
    out["started_at"] = json!(session.start_created_at);
    out["ended_at"] = json!(session.ended_at);
    out["complete"] = json!(window.covers(session.start_created_at));
    out["participants"] = json!(participants);
    Ok(out)
}

fn now_seconds() -> Result<u64, CliError> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .map_err(|e| CliError::Other(format!("system clock is before the Unix epoch: {e}")))
}

async fn run_list(
    client: &BuzzClient,
    channel: &str,
    active_only: bool,
    limit: u32,
    bound: u32,
    now: u64,
) -> Result<Value, CliError> {
    let channel = parse_uuid(channel)?.to_string();
    let relay_self = fetch_relay_self(client).await?;
    let window = fetch_window(client, &channel, bound).await?;
    Ok(build_list(
        &window,
        &relay_self,
        &channel,
        now,
        active_only,
        limit,
        bound,
    ))
}

async fn run_participants(
    client: &BuzzClient,
    channel: &str,
    huddle: &str,
    bound: u32,
    now: u64,
) -> Result<Value, CliError> {
    let channel = parse_uuid(channel)?.to_string();
    let huddle = parse_uuid(huddle)?.to_string();
    let relay_self = fetch_relay_self(client).await?;
    let window = fetch_window(client, &channel, bound).await?;
    build_participants(&window, &relay_self, &channel, &huddle, now, bound)
}

pub async fn cmd_list(
    client: &BuzzClient,
    channel: &str,
    active_only: bool,
    limit: u32,
) -> Result<(), CliError> {
    let out = run_list(
        client,
        channel,
        active_only,
        limit,
        MAX_LIFECYCLE_EVENTS,
        now_seconds()?,
    )
    .await?;
    println!("{out}");
    Ok(())
}

pub async fn cmd_participants(
    client: &BuzzClient,
    channel: &str,
    huddle: &str,
) -> Result<(), CliError> {
    let out = run_participants(
        client,
        channel,
        huddle,
        MAX_LIFECYCLE_EVENTS,
        now_seconds()?,
    )
    .await?;
    println!("{out}");
    Ok(())
}

pub async fn dispatch(cmd: crate::HuddlesCmd, client: &BuzzClient) -> Result<(), CliError> {
    use crate::HuddlesCmd;
    match cmd {
        HuddlesCmd::List {
            channel,
            active,
            limit,
        } => cmd_list(client, &channel, active, limit).await,
        HuddlesCmd::Participants { channel, huddle } => {
            cmd_participants(client, &channel, &huddle).await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    use axum::{body::Bytes, extract::State, routing::post, Router};
    use clap::Parser;
    use nostr::Keys;

    const CHANNEL: &str = "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50";
    const HUDDLE: &str = "11111111-1111-4111-8111-111111111111";
    const OTHER_HUDDLE: &str = "22222222-2222-4222-8222-222222222222";
    const RELAY: &str = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
    const ALICE: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const BOB: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    const CAROL: &str = "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd";
    const EVE: &str = "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee";

    /// Recorded sequences shared with `tests/huddle_interop.mjs`, which replays
    /// the same events through desktop's own tracker.
    const SEQUENCES: &str = include_str!("../../tests/fixtures/huddle_sequences.json");

    fn event(
        id: u32,
        kind: u32,
        pubkey: &str,
        created_at: u64,
        tags: &[&[&str]],
        content: Value,
    ) -> Value {
        json!({
            "id": format!("{id:064x}"),
            "kind": kind,
            "pubkey": pubkey,
            "created_at": created_at,
            "tags": tags,
            "content": content.to_string(),
            "sig": "",
        })
    }

    fn lifecycle(raw: &[Value]) -> Window {
        Window::from_events(raw.to_vec(), MAX_LIFECYCLE_EVENTS).unwrap()
    }

    fn started(id: u32, at: u64, creator: &str, huddle: &str) -> Value {
        event(
            id,
            KIND_HUDDLE_STARTED,
            creator,
            at,
            &[&["h", CHANNEL]],
            json!({ "ephemeral_channel_id": huddle }),
        )
    }

    fn joined(id: u32, at: u64, who: &str, revision: u64, admission: &str) -> Value {
        event(
            id,
            KIND_HUDDLE_PARTICIPANT_JOINED,
            RELAY,
            at,
            &[&["h", CHANNEL], &["p", who]],
            json!({
                "ephemeral_channel_id": HUDDLE,
                "roster_revision": revision,
                "admission_id": admission,
                "generation": "1",
            }),
        )
    }

    fn left(id: u32, at: u64, who: &str, revision: u64, admission: &str) -> Value {
        event(
            id,
            KIND_HUDDLE_PARTICIPANT_LEFT,
            RELAY,
            at,
            &[&["h", CHANNEL], &["p", who]],
            json!({
                "ephemeral_channel_id": HUDDLE,
                "roster_revision": revision,
                "admission_id": admission,
                "generation": "1",
            }),
        )
    }

    fn ended(id: u32, at: u64, signer: &str) -> Value {
        event(
            id,
            KIND_HUDDLE_ENDED,
            signer,
            at,
            &[&["h", CHANNEL]],
            json!({ "ephemeral_channel_id": HUDDLE }),
        )
    }

    fn roster(out: &Value) -> Vec<String> {
        out["participants"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["pubkey"].as_str().unwrap().to_string())
            .collect()
    }

    fn participants_of(raw: &[Value], now: u64) -> Value {
        build_participants(
            &lifecycle(raw),
            RELAY,
            CHANNEL,
            HUDDLE,
            now,
            MAX_LIFECYCLE_EVENTS,
        )
        .unwrap()
    }

    #[test]
    fn rejoin_keeps_one_entry_and_a_stale_leave_does_not_remove_it() {
        let raw = [
            started(1, 100, ALICE, HUDDLE),
            joined(2, 101, BOB, 1, "adm-1"),
            left(3, 102, BOB, 2, "adm-1"),
            joined(4, 103, BOB, 3, "adm-2"),
            // A leave for the first admission delivered after the rejoin.
            left(5, 104, BOB, 2, "adm-1"),
        ];
        let out = participants_of(&raw, 200);
        assert_eq!(roster(&out), vec![BOB]);
        assert_eq!(out["participants"][0]["joined_at"], 103);
        assert_eq!(out["state"], "active");
        assert_eq!(out["complete"], true);
    }

    #[test]
    fn leave_without_join_adds_nobody_and_a_later_join_still_counts() {
        let raw = [
            started(1, 100, ALICE, HUDDLE),
            left(2, 101, BOB, 1, "adm-1"),
            joined(3, 102, CAROL, 2, "adm-2"),
        ];
        assert_eq!(roster(&participants_of(&raw, 200)), vec![CAROL]);

        let raw = [
            started(1, 100, ALICE, HUDDLE),
            left(2, 101, BOB, 2, "adm-1"),
            joined(3, 102, BOB, 1, "adm-1"),
        ];
        assert_eq!(
            roster(&participants_of(&raw, 200)),
            Vec::<String>::new(),
            "a join older than its recorded leave must not resurrect the participant"
        );
    }

    #[test]
    fn ended_huddle_has_an_empty_roster_and_ignores_later_joins() {
        let raw = [
            started(1, 100, ALICE, HUDDLE),
            joined(2, 101, BOB, 1, "adm-1"),
            ended(3, 110, RELAY),
            joined(4, 120, CAROL, 2, "adm-2"),
        ];
        let out = participants_of(&raw, 200);
        assert_eq!(out["state"], "ended");
        assert_eq!(out["ended_at"], 110);
        assert!(roster(&out).is_empty());
    }

    #[test]
    fn end_from_a_stranger_is_ignored() {
        let raw = [
            started(1, 100, ALICE, HUDDLE),
            joined(2, 101, BOB, 1, "adm-1"),
            ended(3, 110, EVE),
        ];
        let out = participants_of(&raw, 200);
        assert_eq!(out["state"], "active");
        assert_eq!(roster(&out), vec![BOB]);
        let by_creator = [raw[0].clone(), raw[1].clone(), ended(5, 110, ALICE)];
        assert_eq!(participants_of(&by_creator, 200)["state"], "ended");
    }

    #[test]
    fn missing_end_event_turns_stale_after_the_joinable_window() {
        let raw = [
            started(1, 1_000, ALICE, HUDDLE),
            joined(2, 1_001, BOB, 1, "adm-1"),
        ];
        let fresh = participants_of(&raw, 1_000 + JOINABLE_WINDOW_SECONDS);
        assert_eq!(fresh["state"], "active");
        let stale = participants_of(&raw, 1_000 + JOINABLE_WINDOW_SECONDS + 1);
        assert_eq!(stale["state"], "stale");
        assert_eq!(stale["ended_at"], Value::Null);
        assert_eq!(roster(&stale), vec![BOB], "stale is not an end event");
        assert_eq!(stale["liveness_checked"], false);
    }

    #[test]
    fn untrusted_lifecycle_events_never_reach_the_roster() {
        let forged_join = event(
            9,
            KIND_HUDDLE_PARTICIPANT_JOINED,
            EVE,
            102,
            &[&["h", CHANNEL], &["p", EVE]],
            json!({ "ephemeral_channel_id": HUDDLE, "admission_id": "x" }),
        );
        let raw = [started(1, 100, ALICE, HUDDLE), forged_join];
        assert!(roster(&participants_of(&raw, 200)).is_empty());
    }

    #[test]
    fn same_second_join_and_leave_follow_the_relay_roster_revision() {
        // Delivered newest-first with an identical timestamp; revision decides.
        let raw = [
            left(4, 101, BOB, 2, "adm-1"),
            joined(3, 101, BOB, 1, "adm-1"),
            started(1, 100, ALICE, HUDDLE),
        ];
        assert!(roster(&participants_of(&raw, 200)).is_empty());
    }

    #[test]
    fn recorded_sequences_match_the_expected_rosters() {
        let sequences: Vec<Value> = serde_json::from_str(SEQUENCES).unwrap();
        assert!(sequences.len() >= 4);
        for sequence in sequences {
            let name = sequence["name"].as_str().unwrap();
            let relay_self = sequence["relay_self"].as_str().unwrap();
            let huddle = sequence["huddle"].as_str().unwrap();
            let events = sequence["events"].as_array().unwrap().clone();
            let window = Window::from_events(events, MAX_LIFECYCLE_EVENTS).unwrap();
            let out = build_participants(&window, relay_self, CHANNEL, huddle, 0, 0).unwrap();
            let mut got = roster(&out);
            got.sort();
            let want: Vec<String> =
                serde_json::from_value(sequence["expected_participants"].clone()).unwrap();
            assert_eq!(got, want, "{name}: roster");
            assert_eq!(
                out["state"] == "ended",
                sequence["expected_ended"].as_bool().unwrap(),
                "{name}: ended"
            );
        }
    }

    #[test]
    fn list_orders_newest_first_and_filters_active() {
        let other = |id: u32, at: u64, kind: u32, signer: &str| {
            event(
                id,
                kind,
                signer,
                at,
                &[&["h", CHANNEL]],
                json!({ "ephemeral_channel_id": OTHER_HUDDLE }),
            )
        };
        let raw = [
            other(10, 50, KIND_HUDDLE_STARTED, CAROL),
            other(11, 60, KIND_HUDDLE_ENDED, RELAY),
            started(1, 100, ALICE, HUDDLE),
            joined(2, 101, BOB, 1, "adm-1"),
        ];
        let window = lifecycle(&raw);
        let all = build_list(
            &window,
            RELAY,
            CHANNEL,
            200,
            false,
            20,
            MAX_LIFECYCLE_EVENTS,
        );
        let huddles = all["huddles"].as_array().unwrap();
        assert_eq!(huddles.len(), 2);
        assert_eq!(huddles[0]["huddle"], HUDDLE);
        assert_eq!(huddles[0]["state"], "active");
        assert_eq!(huddles[0]["participant_count"], 1);
        assert_eq!(huddles[1]["huddle"], OTHER_HUDDLE);
        assert_eq!(huddles[1]["state"], "ended");
        assert_eq!(huddles[1]["ended_at"], 60);
        assert_eq!(all["complete"], true);

        let active = build_list(&window, RELAY, CHANNEL, 200, true, 20, MAX_LIFECYCLE_EVENTS);
        assert_eq!(active["huddles"].as_array().unwrap().len(), 1);

        let stale = build_list(
            &window,
            RELAY,
            CHANNEL,
            100 + JOINABLE_WINDOW_SECONDS + 1,
            true,
            20,
            MAX_LIFECYCLE_EVENTS,
        );
        assert!(
            stale["huddles"].as_array().unwrap().is_empty(),
            "stale huddles are not active"
        );

        let one = build_list(&window, RELAY, CHANNEL, 200, false, 1, MAX_LIFECYCLE_EVENTS);
        assert_eq!(one["huddles"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn truncated_scan_is_reported_not_returned_as_a_full_roster() {
        let raw = vec![
            joined(5, 104, CAROL, 3, "adm-3"),
            joined(4, 103, BOB, 2, "adm-2"),
            joined(3, 102, ALICE, 1, "adm-1"),
            started(1, 100, ALICE, HUDDLE),
        ];

        let window = Window::from_events(raw.clone(), 2).unwrap();
        assert!(window.truncated);
        let out = build_participants(&window, RELAY, CHANNEL, HUDDLE, 200, 2).unwrap();
        assert_eq!(out["state"], "unknown");
        assert_eq!(out["complete"], false);
        assert_eq!(out["truncated"], true);
        assert_eq!(out["event_bound"], 2);
        assert!(roster(&out).is_empty());

        let list = build_list(&window, RELAY, CHANNEL, 200, false, 20, 2);
        assert_eq!(list["complete"], false);

        // The start is inside the window, so the roster is exact even though
        // older history exists.
        let mut tail = raw.clone();
        tail.push(started(6, 20, CAROL, OTHER_HUDDLE));
        tail.push(started(
            7,
            10,
            CAROL,
            "33333333-3333-4333-8333-333333333333",
        ));
        let window = Window::from_events(tail, 5).unwrap();
        assert!(window.truncated);
        let out = build_participants(&window, RELAY, CHANNEL, HUDDLE, 200, 5).unwrap();
        assert_eq!(out["complete"], true);
        assert_eq!(roster(&out), vec![ALICE, BOB, CAROL]);
        let list = build_list(&window, RELAY, CHANNEL, 200, false, 1, 5);
        assert_eq!(list["complete"], true);
        assert_eq!(list["huddles"][0]["huddle"], HUDDLE);

        // An event sharing the oldest scanned second may have been cut off.
        let tie = vec![
            joined(3, 100, BOB, 1, "adm-1"),
            started(1, 100, ALICE, HUDDLE),
            joined(2, 100, CAROL, 2, "adm-2"),
        ];
        let window = Window::from_events(tie, 2).unwrap();
        let out = build_participants(&window, RELAY, CHANNEL, HUDDLE, 200, 2).unwrap();
        assert_eq!(out["complete"], false);
    }

    #[test]
    fn unknown_huddle_is_not_found_when_nothing_was_cut_off() {
        let window = lifecycle(&[started(1, 100, ALICE, HUDDLE)]);
        let err = build_participants(&window, RELAY, CHANNEL, OTHER_HUDDLE, 200, 10).unwrap_err();
        assert!(matches!(err, CliError::NotFound(_)));
    }

    #[test]
    fn malformed_relay_events_fail_instead_of_being_skipped() {
        assert!(Window::from_events(vec![json!({"id": "x"})], 10).is_err());
        let no_session = event(
            1,
            KIND_HUDDLE_STARTED,
            ALICE,
            1,
            &[],
            json!("not an object"),
        );
        let window = Window::from_events(vec![no_session], 10).unwrap();
        assert!(
            build_list(&window, RELAY, CHANNEL, 2, false, 20, 10)["huddles"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }

    #[derive(Clone)]
    struct FakeRelay {
        events: Arc<Vec<Value>>,
        filters: Arc<Mutex<Vec<Value>>>,
        self_pubkey: Option<&'static str>,
    }

    async fn fake_relay(
        events: Vec<Value>,
        self_pubkey: Option<&'static str>,
    ) -> (String, FakeRelay) {
        let relay = FakeRelay {
            events: Arc::new(events),
            filters: Arc::new(Mutex::new(Vec::new())),
            self_pubkey,
        };
        let app = Router::new()
            .route(
                "/",
                axum::routing::get(|State(relay): State<FakeRelay>| async move {
                    let body = match relay.self_pubkey {
                        Some(pk) => json!({ "self": pk }),
                        None => json!({}),
                    };
                    (
                        [("content-type", "application/nostr+json")],
                        body.to_string(),
                    )
                }),
            )
            .route(
                "/query",
                post(|State(relay): State<FakeRelay>, body: Bytes| async move {
                    let filters: Vec<Value> = serde_json::from_slice(&body).unwrap();
                    let filter = filters[0].clone();
                    relay.filters.lock().unwrap().push(filter.clone());
                    let mut matching: Vec<Value> = relay
                        .events
                        .iter()
                        .filter(|e| {
                            filter["kinds"].as_array().unwrap().contains(&e["kind"])
                                && e["tags"].as_array().unwrap().iter().any(|t| {
                                    t[0] == "h" && filter["#h"].as_array().unwrap().contains(&t[1])
                                })
                        })
                        .cloned()
                        .collect();
                    matching.sort_by(|a, b| {
                        b["created_at"]
                            .as_u64()
                            .cmp(&a["created_at"].as_u64())
                            .then_with(|| a["id"].as_str().cmp(&b["id"].as_str()))
                    });
                    if let (Some(until), Some(before)) =
                        (filter["until"].as_u64(), filter["before_id"].as_str())
                    {
                        matching.retain(|e| {
                            let at = e["created_at"].as_u64().unwrap();
                            at < until || (at == until && e["id"].as_str().unwrap() > before)
                        });
                    }
                    matching.truncate(filter["limit"].as_u64().unwrap() as usize);
                    axum::Json(matching)
                }),
            )
            .with_state(relay.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (format!("http://{addr}"), relay)
    }

    fn client(base: &str) -> BuzzClient {
        BuzzClient::new(base.to_string(), Keys::generate(), None, None).unwrap()
    }

    #[tokio::test]
    async fn commands_query_named_kinds_with_a_bound_and_report_truncation() {
        let mut events = vec![started(1, 100, ALICE, HUDDLE)];
        for i in 0..6u32 {
            let who = [ALICE, BOB, CAROL][i as usize % 3];
            events.push(joined(
                10 + i,
                101 + i as u64,
                who,
                i as u64,
                &format!("adm-{i}"),
            ));
        }
        let (base, relay) = fake_relay(events, Some(RELAY)).await;
        let client = client(&base);

        let full = run_participants(&client, CHANNEL, HUDDLE, 100, 200)
            .await
            .unwrap();
        assert_eq!(full["complete"], true);
        assert_eq!(roster(&full), vec![ALICE, BOB, CAROL]);

        let cut = run_participants(&client, CHANNEL, HUDDLE, 3, 200)
            .await
            .unwrap();
        assert_eq!(cut["complete"], false);
        assert_eq!(cut["truncated"], true);
        assert_eq!(cut["state"], "unknown");

        let list = run_list(&client, CHANNEL, false, 20, 100, 200)
            .await
            .unwrap();
        assert_eq!(list["huddles"][0]["participant_count"], 3);

        let filters = relay.filters.lock().unwrap();
        assert!(!filters.is_empty());
        for filter in filters.iter() {
            assert_eq!(
                filter["kinds"],
                json!([
                    KIND_HUDDLE_STARTED,
                    KIND_HUDDLE_PARTICIPANT_JOINED,
                    KIND_HUDDLE_PARTICIPANT_LEFT,
                    KIND_HUDDLE_ENDED
                ])
            );
            assert_eq!(filter["#h"], json!([CHANNEL]));
            assert!(filter["limit"].as_u64().unwrap() <= 500);
        }
    }

    #[tokio::test]
    async fn relay_without_a_self_key_is_an_error_not_an_empty_roster() {
        let (base, _relay) = fake_relay(vec![started(1, 100, ALICE, HUDDLE)], None).await;
        let err = run_list(&client(&base), CHANNEL, false, 20, 10, 200)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("self"));
    }

    #[tokio::test]
    async fn bad_ids_are_usage_errors() {
        let (base, relay) = fake_relay(vec![], Some(RELAY)).await;
        let client = client(&base);
        assert!(matches!(
            run_list(&client, "not-a-uuid", false, 20, 10, 0).await,
            Err(CliError::Usage(_))
        ));
        assert!(matches!(
            run_participants(&client, CHANNEL, "nope", 10, 0).await,
            Err(CliError::Usage(_))
        ));
        assert!(relay.filters.lock().unwrap().is_empty());
    }

    #[test]
    fn arguments_parse_to_the_documented_commands() {
        let cli = crate::Cli::try_parse_from([
            "buzz",
            "huddles",
            "list",
            "--channel",
            CHANNEL,
            "--active",
            "--limit",
            "5",
        ])
        .unwrap();
        assert!(matches!(
            cli.command,
            crate::Cmd::Huddles(crate::HuddlesCmd::List {
                active: true,
                limit: 5,
                ..
            })
        ));
        assert!(crate::Cli::try_parse_from([
            "buzz",
            "huddles",
            "list",
            "--channel",
            CHANNEL,
            "--limit",
            "0"
        ])
        .is_err());
        assert!(crate::Cli::try_parse_from([
            "buzz",
            "huddles",
            "participants",
            "--channel",
            CHANNEL
        ])
        .is_err());
    }
}
