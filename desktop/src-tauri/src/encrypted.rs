//! NIP-17 encrypted messaging for serverless private channels and DMs.
//!
//! On a generic public relay there is no server to enforce channel access, so
//! "private" channels and DMs are made private by **encryption** rather than
//! access control. We use the NIP-17 / NIP-59 gift-wrap scheme:
//!
//! ```text
//! plaintext message  (kind 9 rumor, with the channel `h` tag)
//!   → seal           (kind 13, nip44-encrypted to one recipient)
//!     → gift wrap    (kind 1059, nip44-encrypted with an ephemeral key,
//!                      tagged `#p` = recipient, randomized timestamp)
//! ```
//!
//! One gift wrap is produced **per recipient** (every member, including the
//! sender, so the sender's own client can read its sent messages). The relay
//! only ever stores opaque `kind 1059` blobs addressed by `#p`; it never sees
//! the channel id, the content, or the real author. The inner rumor is a
//! normal `kind 9` event carrying the `h` tag, so once unwrapped it flows
//! through the exact same message-rendering path as a plaintext channel
//! message.
//!
//! This is the "small group" model: O(N) writes per message, no shared group
//! key, no forward secrecy. Suitable for small private groups; it naturally
//! gets heavy for large ones (the cost is the cap).

use nostr::nips::nip59::UnwrappedGift;
use nostr::{Event, EventBuilder, Keys, PublicKey, UnsignedEvent};

/// Build the gift-wrapped events for `rumor`, one per recipient pubkey.
///
/// `rumor` is the unsigned inner event (a normal kind-9 message with the `h`
/// tag). `sender_keys` signs the seal. `recipients` should include every
/// member of the channel **plus the sender** so the sender can read back their
/// own messages. Returns one signed `kind 1059` event per recipient, ready to
/// publish.
pub async fn build_gift_wraps(
    sender_keys: &Keys,
    rumor: UnsignedEvent,
    recipients: &[PublicKey],
) -> Result<Vec<Event>, String> {
    let mut wraps = Vec::with_capacity(recipients.len());
    for recipient in recipients {
        let wrap = EventBuilder::gift_wrap(sender_keys, recipient, rumor.clone(), [])
            .await
            .map_err(|e| format!("gift wrap failed: {e}"))?;
        wraps.push(wrap);
    }
    Ok(wraps)
}

/// Unwrap a `kind 1059` gift wrap addressed to us, returning the inner rumor
/// as a signed-shaped event we can hand to the normal message pipeline.
///
/// The recovered rumor is unsigned (NIP-17 rumors carry no signature), so we
/// surface the verified sender from the seal and rebuild a concrete event with
/// the rumor's id/pubkey/kind/tags/content. The `recipient_keys` must be the
/// identity the gift wrap was `#p`-addressed to.
pub async fn unwrap_gift(
    recipient_keys: &Keys,
    gift_wrap: &Event,
) -> Result<UnwrappedRumor, String> {
    let unwrapped = UnwrappedGift::from_gift_wrap(recipient_keys, gift_wrap)
        .await
        .map_err(|e| format!("gift unwrap failed: {e}"))?;

    let rumor = unwrapped.rumor;
    Ok(UnwrappedRumor {
        sender: unwrapped.sender,
        rumor,
    })
}

/// A decrypted gift wrap: the verified sender plus the inner rumor.
pub struct UnwrappedRumor {
    /// The real author of the message (recovered + verified from the seal).
    pub sender: PublicKey,
    /// The inner message event (kind 9, carries the channel `h` tag).
    pub rumor: UnsignedEvent,
}

impl UnwrappedRumor {
    /// Extract the channel id from the rumor's `h` tag, if present.
    pub fn channel_id(&self) -> Option<String> {
        self.rumor.tags.iter().find_map(|t| {
            let s = t.as_slice();
            if s.len() >= 2 && s[0] == "h" {
                Some(s[1].clone())
            } else {
                None
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Kind 1059 — NIP-59 gift wrap (test-only constant).
    const KIND_GIFT_WRAP: u16 = 1059;
    use nostr::Kind;

    fn rumor(sender: &Keys, channel_id: &str, content: &str) -> UnsignedEvent {
        EventBuilder::new(Kind::Custom(9), content)
            .tags([nostr::Tag::parse(vec!["h", channel_id]).unwrap()])
            .build(sender.public_key())
    }

    #[tokio::test]
    async fn group_member_can_decrypt_others_messages() {
        // A sends to a 3-person group {A, B, C}. B must be able to read it.
        let a = Keys::generate();
        let b = Keys::generate();
        let c = Keys::generate();
        let channel = uuid::Uuid::new_v4().to_string();

        let recipients = [a.public_key(), b.public_key(), c.public_key()];
        let wraps = build_gift_wraps(&a, rumor(&a, &channel, "secret hello"), &recipients)
            .await
            .unwrap();
        assert_eq!(wraps.len(), 3, "one wrap per recipient");

        // Each wrap is a kind 1059 addressed to exactly one recipient via #p.
        for w in &wraps {
            assert_eq!(w.kind, Kind::Custom(KIND_GIFT_WRAP));
        }

        // B finds the wrap addressed to B and decrypts it.
        let b_pk = b.public_key().to_hex();
        let wrap_for_b = wraps
            .iter()
            .find(|w| {
                w.tags.iter().any(|t| {
                    let s = t.as_slice();
                    s.len() >= 2 && s[0] == "p" && s[1] == b_pk
                })
            })
            .expect("a wrap addressed to B");

        let got = unwrap_gift(&b, wrap_for_b).await.unwrap();
        assert_eq!(got.sender, a.public_key(), "sender recovered + verified");
        assert_eq!(got.rumor.content, "secret hello");
        assert_eq!(got.channel_id().as_deref(), Some(channel.as_str()));
    }

    #[tokio::test]
    async fn non_member_cannot_decrypt() {
        // A sends to {A, B}. An outsider D (not p-tagged) cannot decrypt B's wrap.
        let a = Keys::generate();
        let b = Keys::generate();
        let d = Keys::generate();
        let channel = uuid::Uuid::new_v4().to_string();

        let wraps = build_gift_wraps(
            &a,
            rumor(&a, &channel, "members only"),
            &[a.public_key(), b.public_key()],
        )
        .await
        .unwrap();

        let b_pk = b.public_key().to_hex();
        let wrap_for_b = wraps
            .iter()
            .find(|w| {
                w.tags.iter().any(|t| {
                    t.as_slice().len() >= 2 && t.as_slice()[0] == "p" && t.as_slice()[1] == b_pk
                })
            })
            .unwrap();

        // D tries to unwrap B's gift wrap — must fail (wrong recipient key).
        assert!(unwrap_gift(&d, wrap_for_b).await.is_err());
    }

    #[tokio::test]
    #[ignore = "network: hits wss://relay.damus.io"]
    async fn encrypted_group_roundtrip_over_relay() {
        // A sends an encrypted message to {A,B,C} on a real public relay.
        // B fetches its gift-wrap inbox and must decrypt A's message.
        use futures_util::{SinkExt, StreamExt};
        use tokio_tungstenite::{connect_async, tungstenite::Message};

        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

        let a = Keys::generate();
        let b = Keys::generate();
        let c = Keys::generate();
        let channel = uuid::Uuid::new_v4().to_string();
        let secret = format!("encrypted-{}", &channel[..8]);
        let relay =
            std::env::var("RELAY_URL").unwrap_or_else(|_| "wss://relay.damus.io".to_string());
        let relay = relay.as_str();

        // A builds + publishes one gift wrap per member.
        let wraps = build_gift_wraps(
            &a,
            rumor(&a, &channel, &secret),
            &[a.public_key(), b.public_key(), c.public_key()],
        )
        .await
        .unwrap();

        let (ws, _) = connect_async(relay).await.expect("connect");
        let (mut write, mut read) = ws.split();
        for w in &wraps {
            let ev = serde_json::json!(["EVENT", w]).to_string();
            write
                .send(Message::Text(ev.into()))
                .await
                .expect("send wrap");
        }
        // Drain a few OKs.
        for _ in 0..wraps.len() {
            let _ = tokio::time::timeout(std::time::Duration::from_secs(3), read.next()).await;
        }
        let _ = write.close().await;
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;

        // B connects fresh and queries its gift-wrap inbox (#p = B).
        let (ws2, _) = connect_async(relay).await.expect("connect b");
        let (mut w2, mut r2) = ws2.split();
        let b_pk = b.public_key().to_hex();
        let req = serde_json::json!([
            "REQ", "inbox",
            {"kinds":[KIND_GIFT_WRAP], "#p":[b_pk], "limit": 50}
        ])
        .to_string();
        w2.send(Message::Text(req.into())).await.expect("req");

        let mut found = false;
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
        while tokio::time::Instant::now() < deadline {
            let Ok(Some(Ok(msg))) =
                tokio::time::timeout(std::time::Duration::from_secs(5), r2.next()).await
            else {
                break;
            };
            let Message::Text(text) = msg else { continue };
            let Ok(arr) = serde_json::from_str::<serde_json::Value>(&text) else {
                continue;
            };
            match arr.get(0).and_then(|v| v.as_str()) {
                Some("EVENT") => {
                    if let Some(ev) = arr
                        .get(2)
                        .and_then(|v| serde_json::from_value::<Event>(v.clone()).ok())
                    {
                        if let Ok(got) = unwrap_gift(&b, &ev).await {
                            if got.rumor.content == secret {
                                assert_eq!(got.sender, a.public_key());
                                assert_eq!(got.channel_id().as_deref(), Some(channel.as_str()));
                                found = true;
                                break;
                            }
                        }
                    }
                }
                Some("EOSE") => break,
                _ => {}
            }
        }
        let _ = w2.close().await;
        assert!(
            found,
            "B did not receive/decrypt A's encrypted group message"
        );
        eprintln!("✅ encrypted group roundtrip OK over {relay}");
    }

    #[tokio::test]
    async fn sender_can_read_own_message() {
        // The sender includes itself as a recipient so it can read its sent msg.
        let a = Keys::generate();
        let b = Keys::generate();
        let channel = uuid::Uuid::new_v4().to_string();

        let wraps = build_gift_wraps(
            &a,
            rumor(&a, &channel, "echo"),
            &[a.public_key(), b.public_key()],
        )
        .await
        .unwrap();

        let a_pk = a.public_key().to_hex();
        let wrap_for_a = wraps
            .iter()
            .find(|w| {
                w.tags.iter().any(|t| {
                    t.as_slice().len() >= 2 && t.as_slice()[0] == "p" && t.as_slice()[1] == a_pk
                })
            })
            .unwrap();
        let got = unwrap_gift(&a, wrap_for_a).await.unwrap();
        assert_eq!(got.rumor.content, "echo");
    }

    /// Proves the encrypted-reply privacy fix: a THREADED reply in an encrypted
    /// channel is gift-wrapped (kind 1059) — NOT leaked as a plaintext kind-9 —
    /// and the NIP-10 thread tags travel INSIDE the rumor, so threading is
    /// preserved after decryption. This mirrors what `send_channel_message`
    /// does for an encrypted reply: build via `events::build_message` with a
    /// `ThreadRef`, then gift-wrap.
    #[tokio::test]
    async fn encrypted_threaded_reply_is_wrapped_and_preserves_thread() {
        use crate::events::{self, ThreadRef};
        use nostr::EventId;

        let a = Keys::generate(); // replier
        let b = Keys::generate(); // other member
        let channel = uuid::Uuid::new_v4();

        // A reply two levels deep: root != parent.
        let root =
            EventId::from_hex("1111111111111111111111111111111111111111111111111111111111111111")
                .unwrap();
        let parent =
            EventId::from_hex("2222222222222222222222222222222222222222222222222222222222222222")
                .unwrap();
        let thread_ref = ThreadRef {
            root_event_id: root,
            parent_event_id: parent,
        };

        // Build the rumor exactly as the command does for an encrypted reply.
        let builder = events::build_message(
            channel,
            "threaded reply",
            Some(&thread_ref),
            &[],
            &[],
            &[],
            &[],
        )
        .unwrap();
        let rumor = builder.build(a.public_key());

        let recipients = [a.public_key(), b.public_key()];
        let wraps = build_gift_wraps(&a, rumor, &recipients).await.unwrap();

        // 1. Privacy: every wire event is a kind-1059 gift wrap, NOT a
        //    plaintext kind-9. The reply content never appears unencrypted.
        for w in &wraps {
            assert_eq!(
                w.kind,
                Kind::Custom(KIND_GIFT_WRAP),
                "reply must be gift-wrapped, not plaintext"
            );
            assert!(
                !w.content.contains("threaded reply"),
                "plaintext reply content leaked into the wrapper!"
            );
        }

        // 2. Threading preserved: B decrypts and the rumor carries the NIP-10
        //    root + reply `e` tags.
        let b_pk = b.public_key().to_hex();
        let wrap_for_b = wraps
            .iter()
            .find(|w| {
                w.tags.iter().any(|t| {
                    t.as_slice().len() >= 2 && t.as_slice()[0] == "p" && t.as_slice()[1] == b_pk
                })
            })
            .expect("a wrap addressed to B");
        let got = unwrap_gift(&b, wrap_for_b).await.unwrap();
        assert_eq!(got.rumor.content, "threaded reply");
        assert_eq!(
            got.channel_id().as_deref(),
            Some(channel.to_string().as_str())
        );

        let mut found_root = false;
        let mut found_reply = false;
        for t in got.rumor.tags.iter() {
            let s = t.as_slice();
            if s.len() >= 4 && s[0] == "e" {
                match s[3].as_str() {
                    "root" => {
                        assert_eq!(s[1], root.to_hex());
                        found_root = true;
                    }
                    "reply" => {
                        assert_eq!(s[1], parent.to_hex());
                        found_reply = true;
                    }
                    _ => {}
                }
            }
        }
        assert!(found_root, "rumor missing NIP-10 root e-tag");
        assert!(found_reply, "rumor missing NIP-10 reply e-tag");
    }
}
