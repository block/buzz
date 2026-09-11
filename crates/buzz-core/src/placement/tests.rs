use super::*;
use nostr::{EventBuilder, Keys, Kind, Timestamp};

fn host(value: u8) -> PublicKey {
    Keys::parse(&format!("{value:064x}"))
        .expect("fixture key")
        .public_key()
}

fn intent(action: PlacementAction, target: u8, seconds: u64) -> PlacementIntent {
    // Signed fixtures exercise the actual event ID/timestamp extraction, not
    // receiver time. Wire parsing and owner authorization are separate layers.
    let keys = Keys::parse(&format!("{:064x}", 10)).expect("owner");
    let event = EventBuilder::new(Kind::TextNote, format!("{action:?}:{target}"))
        .custom_created_at(Timestamp::from(seconds))
        .sign_with_keys(&keys)
        .expect("sign fixture");
    crate::verify_event(&event).expect("verify fixture");
    PlacementIntent {
        order: EventOrder::from_event(&event),
        host: host(target),
        action,
    }
}

fn start(target: u8, seconds: u64) -> PlacementIntent {
    intent(PlacementAction::Start, target, seconds)
}

fn stop(target: u8, seconds: u64) -> PlacementIntent {
    intent(PlacementAction::Stop, target, seconds)
}

#[test]
fn signed_order_uses_newer_seconds_then_lower_id() {
    let x = start(1, 100);
    let y = start(2, 100);
    assert_ne!(x.order.event_id(), y.order.event_id());
    assert_eq!(
        x.order.cmp(&y.order),
        y.order.event_id().cmp(&x.order.event_id())
    );
    assert!(start(1, 101).order > x.order);
    assert!(start(1, u64::MAX).order > start(1, 101).order);
    assert_eq!(x.order.cmp(&x.order), Ordering::Equal);
    assert_eq!(x.order.partial_cmp(&y.order), Some(x.order.cmp(&y.order)));
}

#[test]
fn empty_and_stop_only_history_never_authorize_launch() {
    let empty = PlacementProjection::new(&[]);
    assert_eq!(empty.desired(), None);
    assert_eq!(empty.target(host(1)), TargetIntent::Unknown);
    let x = stop(1, 20);
    let events = [x];
    let view = PlacementProjection::new(&events);
    assert_eq!(view.desired(), None);
    assert_eq!(view.target(host(1)), TargetIntent::Stopped(x.order));
    assert_eq!(view.target(host(2)), TargetIntent::Unknown);
    assert!(!view.retains_start(host(1), start(1, 10).order));
}

#[test]
fn delayed_stop_of_old_host_preserves_destination_and_its_continuation() {
    let x = start(1, 10);
    let y = start(2, 20);
    let stop_x = stop(1, 30);
    for events in [[x, y, stop_x], [stop_x, x, y], [y, stop_x, x]] {
        let view = PlacementProjection::new(&events);
        assert_eq!(view.desired(), Some((y.host, y.order)));
        assert!(view.retains_start(y.host, y.order));
        assert!(!view.retains_start(x.host, x.order));
        assert_eq!(view.target(x.host), TargetIntent::Stopped(stop_x.order));
    }
}

#[test]
fn stop_of_latest_destination_never_resurrects_previous_host() {
    let x = start(1, 10);
    let y = start(2, 20);
    let stop_y = stop(2, 30);
    for events in [[x, y, stop_y], [stop_y, x, y], [y, stop_y, x]] {
        let view = PlacementProjection::new(&events);
        assert_eq!(view.desired(), None);
        assert_eq!(view.target(x.host), TargetIntent::Stopped(y.order));
        assert_eq!(view.target(y.host), TargetIntent::Stopped(stop_y.order));
        assert!(!view.retains_start(x.host, x.order));
        assert!(!view.retains_start(y.host, y.order));
    }
}

#[test]
fn older_stop_cannot_cancel_newer_start_but_new_selection_invalidates_old_work() {
    let old = start(1, 10);
    let stopped = stop(1, 20);
    let new = start(1, 30);
    let events = [new, stopped, old];
    let view = PlacementProjection::new(&events);
    assert_eq!(view.desired(), Some((new.host, new.order)));
    assert!(view.retains_start(new.host, new.order));
    assert!(!view.retains_start(old.host, old.order));
}

#[test]
fn duplicate_and_lower_rank_backfill_do_not_change_the_selected_start() {
    let selected = start(2, 100);
    let events = [selected, stop(1, 200), selected, start(1, 1), stop(2, 99)];
    let view = PlacementProjection::new(&events);
    assert_eq!(view.desired(), Some((selected.host, selected.order)));
    assert!(view.retains_start(selected.host, selected.order));
}

#[test]
fn future_clock_wins_even_if_later_real_action_arrives_last() {
    let fast = start(1, 1_000_000);
    let events = [fast, start(2, 100), stop(1, 101)];
    let view = PlacementProjection::new(&events);
    assert_eq!(view.desired(), Some((fast.host, fast.order)));
    assert!(view.retains_start(fast.host, fast.order));
}

#[test]
fn same_second_start_stop_and_host_races_follow_lower_id() {
    let x = start(1, 100);
    for competitor in [stop(1, 100), start(2, 100), stop(2, 100)] {
        for events in [[x, competitor], [competitor, x]] {
            let view = PlacementProjection::new(&events);
            let expected = if competitor.order > x.order {
                match competitor.action {
                    PlacementAction::Start => Some((competitor.host, competitor.order)),
                    PlacementAction::Stop if competitor.host == x.host => None,
                    PlacementAction::Stop => Some((x.host, x.order)),
                }
            } else {
                Some((x.host, x.order))
            };
            assert_eq!(view.desired(), expected);
        }
    }
}

// Independent chronological reference model: conditional Stop is valid only
// over SIGNED ORDER, never over delivery order. No effects are executed here.
fn assert_matches_ordered_model(events: &[PlacementIntent]) {
    let mut ordered = events.to_vec();
    ordered.sort_by(|a, b| {
        a.order
            .created_at
            .cmp(&b.order.created_at)
            .then_with(|| b.order.id.cmp(&a.order.id))
    });
    ordered.dedup();
    let hosts = [host(1), host(2), host(3)];
    let mut desired = None;
    let mut targets = [TargetIntent::Unknown; 3];
    for event in ordered {
        match event.action {
            PlacementAction::Start => {
                desired = Some((event.host, event.order));
                for (target, state) in hosts.iter().zip(&mut targets) {
                    *state = if *target == event.host {
                        TargetIntent::Running(event.order)
                    } else {
                        TargetIntent::Stopped(event.order)
                    };
                }
            }
            PlacementAction::Stop => {
                if desired.is_some_and(|(target, _)| target == event.host) {
                    desired = None;
                }
                for (target, state) in hosts.iter().zip(&mut targets) {
                    if *target == event.host {
                        *state = TargetIntent::Stopped(event.order);
                    }
                }
            }
        }
    }
    let view = PlacementProjection::new(events);
    assert_eq!(view.desired(), desired, "events: {events:?}");
    for (target, state) in hosts.iter().zip(targets) {
        assert_eq!(view.target(*target), state, "events: {events:?}");
        for event in events {
            assert_eq!(
                view.retains_start(*target, event.order),
                state == TargetIntent::Running(event.order)
            );
        }
    }
}

fn permutations(events: &mut [PlacementIntent], offset: usize, count: &mut usize) {
    if offset == events.len() {
        *count += 1;
        // Every partial delivery/backfill set is projected independently.
        for end in 0..=events.len() {
            assert_matches_ordered_model(&events[..end]);
        }
        let mut duplicated = events.to_vec();
        duplicated.extend_from_slice(events);
        assert_matches_ordered_model(&duplicated);
        return;
    }
    for next in offset..events.len() {
        events.swap(offset, next);
        permutations(events, offset + 1, count);
        events.swap(offset, next);
    }
}

#[test]
fn all_delivery_permutations_and_prefixes_match_signed_order_projection() {
    for mut events in [
        [
            start(1, 10),
            start(2, 20),
            stop(1, 30),
            stop(2, 40),
            start(3, 50),
            stop(3, 60),
        ],
        [
            start(1, 10),
            start(2, 10),
            stop(1, 10),
            stop(2, 10),
            start(3, 10),
            stop(3, 10),
        ],
    ] {
        let mut count = 0;
        permutations(&mut events, 0, &mut count);
        assert_eq!(count, 720);
    }
}
