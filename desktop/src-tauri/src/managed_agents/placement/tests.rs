use super::*;
use buzz_core_pkg::desktop_lifecycle::{Outcome, ResultMessage};
use nostr::Timestamp;
fn event(keys: &Keys, host: &str, start: bool, stamp: u64) -> Event {
    let target = StopTarget {
        v: 1,
        community: "wss://one.example".into(),
        desktop: host.repeat(32),
        agent: keys.public_key().to_hex(),
    };
    let event = if start {
        Request {
            target,
            action: Action::Start,
            observed: None,
            configuration: None,
            cursor: None,
        }
        .sign(keys)
        .unwrap()
    } else {
        target.sign(keys).unwrap()
    };
    nostr::EventBuilder::new(event.kind, event.content)
        .tags(event.tags)
        .custom_created_at(Timestamp::from(stamp))
        .sign_with_keys(keys)
        .unwrap()
}
#[test]
fn opposite_arrival_and_scoped_stops_converge_without_resurrection() {
    let keys = Keys::generate();
    let agent = keys.public_key().to_hex();
    let events = [
        event(&keys, "a", true, 1),
        event(&keys, "b", true, 2),
        event(&keys, "a", false, 3),
    ];
    for order in [vec![0, 1, 2], vec![2, 0, 1], vec![1, 2, 0]] {
        let conn = Connection::open_in_memory().unwrap();
        for i in order {
            observe(&conn, &events[i], &keys, "wss://one.example").unwrap();
        }
        assert_eq!(
            desired(&conn, &agent).unwrap(),
            Some(("b".repeat(32), events[1].id.to_hex()))
        );
        assert!(blocked(&conn, &agent, &"a".repeat(32)).unwrap());
        assert!(!blocked(&conn, &agent, &"b".repeat(32)).unwrap());
        observe(
            &conn,
            &event(&keys, "b", false, 4),
            &keys,
            "wss://one.example",
        )
        .unwrap();
        assert_eq!(desired(&conn, &agent).unwrap(), None);
        assert!(blocked(&conn, &agent, &"b".repeat(32)).unwrap());
        observe(&conn, &events[0], &keys, "wss://one.example").unwrap();
        assert_eq!(desired(&conn, &agent).unwrap(), None);
    }
}
#[test]
fn same_second_lower_id_and_future_timestamp_are_authority() {
    let keys = Keys::generate();
    let conn = Connection::open_in_memory().unwrap();
    let a = event(&keys, "a", true, 1000);
    let b = event(&keys, "b", true, 1000);
    for e in [&a, &b, &a] {
        observe(&conn, e, &keys, "wss://one.example").unwrap();
    }
    let winner = if a.id < b.id { &a } else { &b };
    assert_eq!(
        desired(&conn, &keys.public_key().to_hex())
            .unwrap()
            .unwrap()
            .1,
        winner.id.to_hex()
    );
    observe(
        &conn,
        &event(&keys, "c", true, 999),
        &keys,
        "wss://one.example",
    )
    .unwrap();
    assert_eq!(
        desired(&conn, &keys.public_key().to_hex())
            .unwrap()
            .unwrap()
            .1,
        winner.id.to_hex()
    );
}
#[test]
fn consumption_survives_restart_and_result_eviction() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("journal.db");
    let keys = Keys::generate();
    let event = event(&keys, "a", true, 1);
    let request = Request::read(&event, &keys, "wss://one.example").unwrap();
    let mut conn = Connection::open(&path).unwrap();
    assert!(admit(&conn, &event, &request).unwrap());
    for i in 0..258 {
        let result = ResultMessage {
            request: request.clone(),
            id: event.id.to_hex(),
            outcome: Outcome::Unknown,
            observation: None,
        }
        .sign(&keys)
        .unwrap();
        save(&mut conn, &i.to_string(), &result).unwrap();
    }
    drop(conn);
    let conn = Connection::open(&path).unwrap();
    assert!(!admit(&conn, &event, &request).unwrap());
    assert!(saved(&conn, "0").unwrap().is_none());
    assert!(admit(&conn, &super::tests::event(&keys, "a", true, 2), &request).unwrap());
}

#[test]
fn receiver_consumes_before_effect_and_never_repeats_restart() {
    let keys = Keys::generate();
    let mut conn = Connection::open_in_memory().unwrap();
    let start = event(&keys, "a", true, 10);
    let start_request = Request::read(&start, &keys, "wss://one.example").unwrap();
    observe(&conn, &start, &keys, "wss://one.example").unwrap();
    let restart = Request {
        action: Action::Restart,
        observed: Some("f".repeat(64)),
        configuration: None,
        cursor: None,
        ..start_request
    }
    .sign(&keys)
    .unwrap();
    let mut effects = 0;
    let result = receive(
        &mut conn,
        &restart,
        &keys,
        "wss://one.example",
        &"a".repeat(32),
        true,
        |conn, request| {
            assert!(
                !admit(conn, &restart, request).unwrap(),
                "effect must see durable consumption"
            );
            effects += 1;
            Ok((Outcome::Running, None))
        },
    )
    .unwrap()
    .unwrap();
    let retry = receive(
        &mut conn,
        &restart,
        &keys,
        "wss://one.example",
        &"a".repeat(32),
        true,
        |_, _| panic!("duplicate effect"),
    )
    .unwrap()
    .unwrap();
    assert_eq!(result, retry);
    assert_eq!(effects, 1);
    conn.execute("DELETE FROM desktop_lifecycle_results", [])
        .unwrap();
    let unknown = receive(
        &mut conn,
        &restart,
        &keys,
        "wss://one.example",
        &"a".repeat(32),
        true,
        |_, _| panic!("evicted effect"),
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        ResultMessage::read(&unknown, &keys, &restart, "wss://one.example")
            .unwrap()
            .outcome,
        Outcome::Unknown
    );
}

#[test]
fn receiver_rejects_wrong_owner_route_and_superseded_start() {
    let keys = Keys::generate();
    let mut conn = Connection::open_in_memory().unwrap();
    let first = event(&keys, "a", true, 1);
    let newer = event(&keys, "a", true, 2);
    observe(&conn, &newer, &keys, "wss://one.example").unwrap();
    assert!(receive(
        &mut conn,
        &first,
        &Keys::generate(),
        "wss://one.example",
        &"a".repeat(32),
        true,
        |_, _| panic!("owner")
    )
    .is_err());
    assert!(receive(
        &mut conn,
        &first,
        &keys,
        "wss://one.example",
        &"b".repeat(32),
        true,
        |_, _| panic!("route")
    )
    .unwrap()
    .is_none());
    let result = receive(
        &mut conn,
        &first,
        &keys,
        "wss://one.example",
        &"a".repeat(32),
        true,
        |_, _| panic!("stale Start"),
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        ResultMessage::read(&result, &keys, &first, "wss://one.example")
            .unwrap()
            .outcome,
        Outcome::Unknown
    );
    let denied = receive(
        &mut conn,
        &newer,
        &keys,
        "wss://one.example",
        &"a".repeat(32),
        false,
        |_, _| panic!("unowned"),
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        ResultMessage::read(&denied, &keys, &newer, "wss://one.example")
            .unwrap()
            .outcome,
        Outcome::Failed
    );
}

#[test]
fn receiver_failure_is_saved_without_reinvoking_launch() {
    let keys = Keys::generate();
    let mut conn = Connection::open_in_memory().unwrap();
    let start = event(&keys, "a", true, Timestamp::now().as_secs());
    let result = receive(
        &mut conn,
        &start,
        &keys,
        "wss://one.example",
        &"a".repeat(32),
        true,
        |_, _| Err("native error with private path".into()),
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        ResultMessage::read(&result, &keys, &start, "wss://one.example")
            .unwrap()
            .outcome,
        Outcome::Failed
    );
    let retry = receive(
        &mut conn,
        &start,
        &keys,
        "wss://one.example",
        &"a".repeat(32),
        true,
        |_, _| panic!("retry"),
    )
    .unwrap()
    .unwrap();
    assert_eq!(retry, result);
}

#[test]
fn probes_never_write_placement_or_admission_and_retries_are_exact() {
    let keys = Keys::generate();
    for action in [Action::Catalog, Action::Preflight] {
        let mut conn = Connection::open_in_memory().unwrap();
        let reference = buzz_core_pkg::desktop_lifecycle::RuntimeConfigurationRef {
            id: uuid::Uuid::new_v4().to_string(),
            revision: uuid::Uuid::new_v4().to_string(),
        };
        let mut request =
            Request::read(&event(&keys, "a", true, 1), &keys, "wss://one.example").unwrap();
        request.action = action;
        request.configuration = (action == Action::Preflight).then_some(reference);
        let event = request.sign(&keys).unwrap();
        let result = receive(
            &mut conn,
            &event,
            &keys,
            "wss://one.example",
            &"a".repeat(32),
            true,
            |conn, _| {
                for table in ["desktop_placement", "desktop_lifecycle_admission"] {
                    assert_eq!(
                        conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r
                            .get::<_, i64>(0))
                            .unwrap(),
                        0
                    );
                }
                Ok((Outcome::Ineligible, None))
            },
        )
        .unwrap()
        .unwrap();
        let retry = receive(
            &mut conn,
            &event,
            &keys,
            "wss://one.example",
            &"a".repeat(32),
            true,
            |_, _| panic!("repeat probe"),
        )
        .unwrap()
        .unwrap();
        assert_eq!(result, retry);
        assert_eq!(latest_start(&conn, &request.target.agent).unwrap(), None);
    }
}

#[test]
fn expired_start_cannot_complete_a_delayed_preflight() {
    let keys = Keys::generate();
    let mut conn = Connection::open_in_memory().unwrap();
    let event = event(&keys, "a", true, Timestamp::now().as_secs() - 31);
    let result = receive(
        &mut conn,
        &event,
        &keys,
        "wss://one.example",
        &"a".repeat(32),
        true,
        |_, _| panic!("expired effect"),
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        ResultMessage::read(&result, &keys, &event, "wss://one.example")
            .unwrap()
            .outcome,
        Outcome::Unknown
    );
}
