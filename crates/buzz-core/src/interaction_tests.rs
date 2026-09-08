use super::*;
use nostr::Keys;

fn event(kind: u32, content: &str, extra: &[&[&str]], keys: &Keys, time: u64) -> Event {
    let mut ts = vec![Tag::parse(["h", "00000000-0000-0000-0000-000000000001"]).unwrap()];
    ts.extend(extra.iter().map(|t| Tag::parse(t.iter().copied()).unwrap()));
    EventBuilder::new(Kind::Custom(kind as u16), content)
        .tags(ts)
        .custom_created_at(Timestamp::from(time))
        .sign_with_keys(keys)
        .unwrap()
}
fn prompt(extra: &[&[&str]]) -> Event {
    let mut ts = vec![
        &["itype", "buttons"][..],
        &["opt", "approve", "Approve", "primary"],
        &["opt", "deny", "Deny", "danger"],
        &["closes", "first"],
        &["deadline", "2000"],
    ];
    ts.extend_from_slice(extra);
    event(
        KIND_INTERACTION_PROMPT,
        "Render?",
        &ts,
        &Keys::generate(),
        1000,
    )
}
fn response(pk: &Keys, choice: &str, time: u64) -> Event {
    event(
        KIND_INTERACTION_RESPONSE,
        "",
        &[&["e", &"a".repeat(64), "", "prompt"], &["choice", choice]],
        pk,
        time,
    )
}

#[test]
fn schema_rejects_ambiguous_or_unsafe_prompts() {
    assert!(Prompt::parse(&prompt(&[])).is_ok());
    for extra in [
        vec![&["itype", "poll"][..]],
        vec![&["visibility", "asker-only"][..]],
        vec![&["visibility", "tallies-only"][..]],
        vec![&["expiration", "2000"][..]],
        vec![&["field", "password", "Password", "secret", "required"][..]],
        vec![&["opt", "APPROVE", "Other"][..]],
        vec![&["opt", "other", "Approve"][..]],
        vec![&["opt", "other", "deny"][..]],
        vec![&["responders", "listed"][..]],
        vec![&["max", "2"][..]],
        vec![&["via", "forged"][..]],
    ] {
        assert!(
            Prompt::parse(&prompt(&extra)).is_err(),
            "accepted {extra:?}"
        );
    }
    let p = Prompt::parse(&prompt(&[])).unwrap();
    assert!(p.validate_lifetime(1000).is_ok());
    assert!(p.validate_lifetime(2000).is_err());
    assert!(p.text_answer(" \nAPPROVE\t").is_some());
    for text in ["approve!", "approve but revise", "approved", ""] {
        assert!(p.text_answer(text).is_none());
    }
}

#[test]
fn validation_and_replacement_have_one_actor_one_vote() {
    let mut p = Prompt::parse(&prompt(&[])).unwrap();
    p.closes = "manual".into();
    let k = Keys::generate();
    let mut state = InteractionState::default();
    let a = response(&k, "approve", 1000);
    assert!(state
        .respond(&p, a.clone(), p.answer(&a).unwrap(), 1000)
        .unwrap());
    assert!(!state
        .respond(&p, a.clone(), p.answer(&a).unwrap(), 1000)
        .unwrap());
    let b = response(&k, "deny", 1001);
    state
        .respond(&p, b.clone(), p.answer(&b).unwrap(), 1001)
        .unwrap();
    assert_eq!(state.summary(&p)["tally"]["approve"], 0);
    assert_eq!(state.summary(&p)["tally"]["deny"], 1);
    assert_eq!(state.votes.len(), 1);
    assert!(state
        .respond(&p, a.clone(), p.answer(&a).unwrap(), 1002)
        .is_err());
    assert!(p.answer(&response(&k, "maybe", 1001)).is_err());
    state.close("manual");
    assert!(state
        .respond(&p, response(&k, "approve", 1003), Answer::default(), 1003)
        .is_err());
    assert!(!state.close("expiry"));
}

#[test]
fn close_rules_ties_and_deadline_are_deterministic() {
    let p = Prompt::parse(&prompt(&[])).unwrap();
    let a = response(&Keys::generate(), "approve", 1000);
    let b = response(&Keys::generate(), "deny", 1001);
    let mut state = InteractionState::default();
    state
        .respond(&p, a.clone(), p.answer(&a).unwrap(), 1000)
        .unwrap();
    assert_eq!(state.summary(&p)["winner"], "approve");
    assert!(state
        .respond(&p, b.clone(), p.answer(&b).unwrap(), 1001)
        .is_err());
    let mut p = p;
    p.closes = "quorum:2".into();
    let mut state = InteractionState::default();
    state
        .respond(&p, a.clone(), p.answer(&a).unwrap(), 1000)
        .unwrap();
    assert!(state.close_reason.is_none());
    state
        .respond(&p, b.clone(), p.answer(&b).unwrap(), 1001)
        .unwrap();
    assert_eq!(state.close_reason.as_deref(), Some("quorum"));
    assert!(state.summary(&p)["winner"].is_null());
    assert!(InteractionState::default()
        .respond(&p, a.clone(), p.answer(&a).unwrap(), 2000)
        .is_err());
}

#[test]
fn forms_validate_real_values_and_required_fields() {
    let k = Keys::generate();
    let ev = event(
        KIND_INTERACTION_PROMPT,
        "Details?",
        &[
            &["itype", "form"],
            &["closes", "first"],
            &["deadline", "2000"],
            &["field", "amount", "Amount", "number", "required"],
            &["field", "date", "Date", "date", "optional"],
            &["field", "ready", "Ready", "boolean", "optional"],
            &["field", "size", "Size", "select", "optional"],
            &["optsel", "size", "small", "Small"],
        ],
        &k,
        1000,
    );
    let p = Prompt::parse(&ev).unwrap();
    let make = |values: &[&[&str]]| {
        let id = ev.id.to_hex();
        let mut ts = vec![vec!["e", id.as_str(), "", "prompt"]];
        ts.extend(values.iter().map(|t| t.to_vec()));
        event(
            KIND_INTERACTION_RESPONSE,
            "",
            &ts.iter().map(Vec::as_slice).collect::<Vec<_>>(),
            &k,
            1000,
        )
    };
    assert!(p
        .answer(&make(&[
            &["value", "amount", "12.5"],
            &["value", "date", "2026-09-08"],
            &["value", "ready", "false"],
            &["value", "size", "small"]
        ]))
        .is_ok());
    for values in [
        vec![],
        vec![&["value", "amount", "NaN"][..]],
        vec![&["value", "amount", "inf"][..]],
        vec![&["value", "unknown", "12"][..]],
        vec![&["value", "amount", "12"][..], &["value", "amount", "13"]],
        vec![
            &["value", "amount", "12"][..],
            &["value", "date", "2026-02-30"],
        ],
        vec![&["value", "amount", "12"][..], &["value", "ready", "yes"]],
        vec![&["value", "amount", "12"][..], &["value", "size", "large"]],
    ] {
        assert!(p.answer(&make(&values)).is_err(), "accepted {values:?}");
    }
}

#[test]
fn same_second_tie_break_is_independent_of_arrival_order() {
    let mut p = Prompt::parse(&prompt(&[])).unwrap();
    p.closes = "manual".into();
    let k = Keys::generate();
    let mut events = [response(&k, "approve", 1000), response(&k, "deny", 1000)];
    events.sort_by_key(|e| e.id);
    let mut state = InteractionState::default();
    for ev in events.iter().rev() {
        state
            .respond(&p, ev.clone(), p.answer(ev).unwrap(), 1000)
            .unwrap();
    }
    assert_eq!(
        state.votes[&k.public_key().to_hex()].source.id,
        events[0].id
    );
    assert!(state
        .respond(&p, events[1].clone(), p.answer(&events[1]).unwrap(), 1000)
        .is_err());
}

/// A hand-authored control trace: Discord-style multi-choice selection, then
/// GroupMe-style vote replacement before close. Expected counts are independent
/// of the implementation's tally calculation; see docs/interaction-controls.md.
#[test]
fn poll_control_trace_replaces_the_entire_selection_and_keeps_evidence() {
    let asker = Keys::generate();
    let prompt_event = event(
        KIND_INTERACTION_PROMPT,
        "Choose a thumbnail",
        &[
            &["itype", "poll"],
            &["opt", "a", "Door"],
            &["opt", "b", "Key"],
            &["opt", "c", "Hall"],
            &["min", "1"],
            &["max", "2"],
            &["closes", "manual"],
            &["deadline", "2000"],
        ],
        &asker,
        1000,
    );
    let p = Prompt::parse(&prompt_event).unwrap();
    let alice = Keys::generate();
    let bob = Keys::generate();
    let make = |key: &Keys, choices: &[&str], time| {
        let mut tags = vec![vec![
            "e".to_string(),
            prompt_event.id.to_hex(),
            "".into(),
            "prompt".into(),
        ]];
        tags.extend(choices.iter().map(|id| vec!["choice".into(), (*id).into()]));
        let tags: Vec<Vec<&str>> = tags
            .iter()
            .map(|t| t.iter().map(String::as_str).collect())
            .collect();
        event(
            KIND_INTERACTION_RESPONSE,
            "",
            &tags.iter().map(Vec::as_slice).collect::<Vec<_>>(),
            key,
            time,
        )
    };
    for choices in [vec![], vec!["a", "b", "c"], vec!["a", "a"], vec!["unknown"]] {
        assert!(p.answer(&make(&alice, &choices, 1001)).is_err());
    }
    let first = make(&alice, &["a", "b"], 1001);
    let other = make(&bob, &["b"], 1001);
    let revised = make(&alice, &["c"], 1002);
    let mut state = InteractionState::default();
    for (ev, tally) in [
        (&first, serde_json::json!({"a":1,"b":1,"c":0})),
        (&other, serde_json::json!({"a":1,"b":2,"c":0})),
        (&revised, serde_json::json!({"a":0,"b":1,"c":1})),
    ] {
        ev.verify().unwrap();
        state
            .respond(&p, ev.clone(), p.answer(ev).unwrap(), 1002)
            .unwrap();
        let summary = state.summary(&p);
        assert_eq!(summary["tally"], tally);
        assert_eq!(summary["status"], "open");
        assert!(summary["winner"].is_null());
    }
    assert_eq!(state.votes.len(), 2);
    assert_eq!(
        state.votes[&alice.public_key().to_hex()].source.id,
        revised.id
    );
    state.close("manual");
    let late = make(&bob, &["a"], 1003);
    assert!(state
        .respond(&p, late.clone(), p.answer(&late).unwrap(), 1003)
        .is_err());
    assert_eq!(
        state.summary(&p)["tally"],
        serde_json::json!({"a":0,"b":1,"c":1})
    );
}
