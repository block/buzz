use std::time::Instant;

use nostr::{EventBuilder, Keys, Kind, Tag};
use uuid::Uuid;

use crate::config::DedupMode;
use crate::queue::{EventQueue, QueuedEvent};

fn thread_event(channel_id: Uuid, keys: &Keys, root: &str, content: &str) -> QueuedEvent {
    let reply = Tag::parse(["e", root, "", "reply"]).expect("valid thread tag");
    let event = EventBuilder::new(Kind::Custom(9), content)
        .tags([reply])
        .sign_with_keys(keys)
        .expect("sign test event");
    QueuedEvent {
        channel_id,
        event,
        received_at: Instant::now(),
        prompt_tag: "test".into(),
    }
}

fn top_level_event(channel_id: Uuid, keys: &Keys, content: &str) -> QueuedEvent {
    let event = EventBuilder::new(Kind::Custom(9), content)
        .sign_with_keys(keys)
        .expect("sign test event");
    QueuedEvent {
        channel_id,
        event,
        received_at: Instant::now(),
        prompt_tag: "test".into(),
    }
}

#[test]
fn batch_splits_when_actor_changes() {
    let mut queue = EventQueue::new(DedupMode::Queue);
    let channel_id = Uuid::new_v4();
    let first_actor = Keys::generate();
    let second_actor = Keys::generate();
    let root = "c".repeat(64);

    queue.push(thread_event(
        channel_id,
        &first_actor,
        &root,
        "from first actor",
    ));
    queue.push(thread_event(
        channel_id,
        &second_actor,
        &root,
        "from second actor",
    ));

    let first = queue.flush_next().expect("first authority group");
    assert_eq!(first.events.len(), 1);
    assert_eq!(first.events[0].event.content, "from first actor");

    queue.mark_complete(channel_id);
    let second = queue.flush_next().expect("second authority group");
    assert_eq!(second.events.len(), 1);
    assert_eq!(second.events[0].event.content, "from second actor");
}

#[test]
fn batch_splits_when_thread_destination_changes() {
    let mut queue = EventQueue::new(DedupMode::Queue);
    let channel_id = Uuid::new_v4();
    let actor = Keys::generate();
    let first_root = "d".repeat(64);
    let second_root = "e".repeat(64);

    queue.push(thread_event(
        channel_id,
        &actor,
        &first_root,
        "first thread",
    ));
    queue.push(thread_event(
        channel_id,
        &actor,
        &second_root,
        "second thread",
    ));

    let first = queue.flush_next().expect("first destination group");
    assert_eq!(first.events.len(), 1);
    assert_eq!(first.events[0].event.content, "first thread");

    queue.mark_complete(channel_id);
    let second = queue.flush_next().expect("second destination group");
    assert_eq!(second.events.len(), 1);
    assert_eq!(second.events[0].event.content, "second thread");
}

#[test]
fn unrelated_top_level_instructions_are_distinct_destinations() {
    let mut queue = EventQueue::new(DedupMode::Queue);
    let channel_id = Uuid::new_v4();
    let actor = Keys::generate();

    queue.push(top_level_event(channel_id, &actor, "first request"));
    queue.push(top_level_event(channel_id, &actor, "second request"));

    let first = queue.flush_next().expect("first top-level request");
    assert_eq!(first.events.len(), 1);
    assert_eq!(first.events[0].event.content, "first request");

    queue.mark_complete(channel_id);
    let second = queue.flush_next().expect("second top-level request");
    assert_eq!(second.events.len(), 1);
    assert_eq!(second.events[0].event.content, "second request");
}
