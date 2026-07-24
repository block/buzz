//! Integration tests for the seam contract against the in-memory hub —
//! several participants exchanging signed events through `Box<dyn Transport>`
//! with no I/O at all.

use buzz_transport::memory::InMemoryHub;
use buzz_transport::{SignedEvent, Subscription, Transport, TransportError};
use nostr::{EventBuilder, Keys, Kind, Tag};
use tokio::time::{timeout, Duration};
use uuid::Uuid;

/// Build a signed event in `channel`, optionally `p`-tagging a mention.
fn signed_event(
    keys: &Keys,
    kind: u16,
    channel: Uuid,
    content: &str,
    mention: Option<&str>,
) -> SignedEvent {
    let mut tags = vec![Tag::parse(["h", &channel.to_string()]).unwrap()];
    if let Some(pubkey) = mention {
        tags.push(Tag::parse(["p", pubkey]).unwrap());
    }
    let event = EventBuilder::new(Kind::Custom(kind), content)
        .tags(tags)
        .sign_with_keys(keys)
        .unwrap();
    SignedEvent::from_nostr(&event).unwrap()
}

async fn recv(transport: &mut Box<dyn Transport>) -> buzz_transport::TransportEvent {
    timeout(Duration::from_secs(5), transport.next_event())
        .await
        .expect("timed out waiting for an event")
        .expect("stream ended unexpectedly")
}

#[tokio::test]
async fn events_fan_out_between_participants() {
    let hub = InMemoryHub::new();
    let alice_keys = Keys::generate();
    let bob_keys = Keys::generate();
    let channel = Uuid::new_v4();

    let mut alice: Box<dyn Transport> = Box::new(hub.connect(alice_keys.public_key().to_hex()));
    let mut bob: Box<dyn Transport> = Box::new(hub.connect(bob_keys.public_key().to_hex()));
    let mut carol: Box<dyn Transport> = Box::new(hub.connect("carol".to_string()));

    for t in [&mut alice, &mut bob, &mut carol] {
        t.subscribe(Subscription::all(channel)).await.unwrap();
    }

    alice
        .publish(signed_event(&alice_keys, 9, channel, "hi from alice", None))
        .await
        .unwrap();

    // Both other participants receive it; the payload survives intact and
    // still verifies on the receiving side.
    for receiver in [&mut bob, &mut carol] {
        let received = recv(receiver).await;
        assert_eq!(received.channel_id, channel);
        assert_eq!(received.event.content, "hi from alice");
        assert_eq!(received.event.pubkey, alice_keys.public_key().to_hex());
        received.event.verify().unwrap();
    }

    // Alice never sees her own echo: the next thing she receives is Bob's
    // reply, not her own event.
    bob.publish(signed_event(&bob_keys, 9, channel, "hi back", None))
        .await
        .unwrap();
    let alice_got = recv(&mut alice).await;
    assert_eq!(alice_got.event.content, "hi back");
}

#[tokio::test]
async fn subscription_filters_gate_delivery() {
    let hub = InMemoryHub::new();
    let sender_keys = Keys::generate();
    let receiver_pubkey = Keys::generate().public_key().to_hex();
    let channel = Uuid::new_v4();

    let sender = hub.connect(sender_keys.public_key().to_hex());
    let mut receiver: Box<dyn Transport> = Box::new(hub.connect(receiver_pubkey.clone()));

    receiver
        .subscribe(Subscription {
            channel_id: channel,
            kinds: Some(vec![9]),
            require_mention: true,
            replay_since: None,
        })
        .await
        .unwrap();

    // Filtered out: wrong kind, then right kind without a mention, then an
    // unsubscribed channel entirely.
    sender
        .publish(signed_event(
            &sender_keys,
            7,
            channel,
            "reaction",
            Some(&receiver_pubkey),
        ))
        .await
        .unwrap();
    sender
        .publish(signed_event(&sender_keys, 9, channel, "no mention", None))
        .await
        .unwrap();
    sender
        .publish(signed_event(
            &sender_keys,
            9,
            Uuid::new_v4(),
            "elsewhere",
            Some(&receiver_pubkey),
        ))
        .await
        .unwrap();
    // Delivered: right channel, right kind, mentions the receiver.
    sender
        .publish(signed_event(
            &sender_keys,
            9,
            channel,
            "for you",
            Some(&receiver_pubkey),
        ))
        .await
        .unwrap();

    let received = recv(&mut receiver).await;
    assert_eq!(received.event.content, "for you");
}

#[tokio::test]
async fn unsubscribe_and_resubscribe_take_effect() {
    let hub = InMemoryHub::new();
    let sender_keys = Keys::generate();
    let muted = Uuid::new_v4();
    let open = Uuid::new_v4();

    let sender = hub.connect(sender_keys.public_key().to_hex());
    let mut receiver: Box<dyn Transport> = Box::new(hub.connect("receiver".to_string()));

    receiver.subscribe(Subscription::all(muted)).await.unwrap();
    receiver.subscribe(Subscription::all(open)).await.unwrap();
    receiver.unsubscribe(muted).await.unwrap();

    sender
        .publish(signed_event(&sender_keys, 9, muted, "unwanted", None))
        .await
        .unwrap();
    sender
        .publish(signed_event(&sender_keys, 9, open, "wanted", None))
        .await
        .unwrap();
    assert_eq!(recv(&mut receiver).await.event.content, "wanted");

    // Re-subscribing replaces the previous subscription: narrow `open` to a
    // kind the sender doesn't use and delivery stops.
    receiver
        .subscribe(Subscription {
            channel_id: open,
            kinds: Some(vec![7]),
            require_mention: false,
            replay_since: None,
        })
        .await
        .unwrap();
    sender
        .publish(signed_event(&sender_keys, 9, open, "now filtered", None))
        .await
        .unwrap();
    sender
        .publish(signed_event(&sender_keys, 7, open, "reaction", None))
        .await
        .unwrap();
    assert_eq!(recv(&mut receiver).await.event.content, "reaction");
}

#[tokio::test]
async fn invalid_events_are_rejected_at_publish() {
    let hub = InMemoryHub::new();
    let keys = Keys::generate();
    let channel = Uuid::new_v4();
    let transport = hub.connect(keys.public_key().to_hex());

    // Tampered content: signature no longer covers the payload.
    let mut tampered = signed_event(&keys, 9, channel, "original", None);
    tampered.content = "tampered".into();
    assert!(matches!(
        transport.try_publish(tampered),
        Err(TransportError::InvalidEvent(_))
    ));

    // Validly signed but not channel-scoped: nothing to route by.
    let event = EventBuilder::new(Kind::Custom(9), "no channel")
        .sign_with_keys(&keys)
        .unwrap();
    let unrouted = SignedEvent::from_nostr(&event).unwrap();
    assert!(matches!(
        transport.try_publish(unrouted),
        Err(TransportError::InvalidEvent(_))
    ));
}

#[tokio::test]
async fn reconnect_keeps_the_stream_usable() {
    let hub = InMemoryHub::new();
    let sender_keys = Keys::generate();
    let channel = Uuid::new_v4();

    let sender = hub.connect(sender_keys.public_key().to_hex());
    let mut receiver: Box<dyn Transport> = Box::new(hub.connect("receiver".to_string()));
    receiver
        .subscribe(Subscription::all(channel))
        .await
        .unwrap();

    receiver.reconnect().await.unwrap();
    sender
        .publish(signed_event(
            &sender_keys,
            9,
            channel,
            "after reconnect",
            None,
        ))
        .await
        .unwrap();
    assert_eq!(recv(&mut receiver).await.event.content, "after reconnect");

    receiver.shutdown().await;
}
