//! Production and preflight tests for opt-in addressed failure routing.
//!
//! The HTTP fixture answers the same `/query` calls used by production: kind:0
//! sibling profiles carry genuine NIP-OA attestations and kind:39002 carries
//! the membership snapshot. `/events` captures the signed notice.

use std::sync::{Arc, Mutex};

use nostr::{Event, EventBuilder, Keys, Kind, Tag};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::*;

#[derive(Default)]
struct Capture {
    events: Vec<Event>,
    queries: usize,
}

#[derive(Clone, Copy)]
enum MembershipMode {
    Valid,
    WrongSigner,
    SlowProfiles,
    Tampered,
}

fn event(
    keys: &Keys,
    channel_id: uuid::Uuid,
    content: &str,
    failure: bool,
    visited: &[String],
) -> Event {
    let mut builder = EventBuilder::new(Kind::Custom(9), content);
    builder = builder.tag(Tag::parse(["h".to_string(), channel_id.to_string()]).unwrap());
    if failure {
        builder = builder.tag(Tag::parse(["buzz:agent-failure", "1"]).unwrap());
    }
    for value in visited {
        builder = builder.tag(Tag::parse([failure_routing::VISITED_TAG, value.as_str()]).unwrap());
    }
    builder.sign_with_keys(keys).unwrap()
}

fn batch(channel_id: uuid::Uuid, events: Vec<Event>) -> queue::FlushBatch {
    queue::FlushBatch {
        channel_id,
        scope: scope::SessionScope::Conversation { channel_id },
        events: events
            .into_iter()
            .map(|event| queue::BatchEvent {
                event,
                prompt_tag: "@mention".into(),
                received_at: std::time::Instant::now(),
            })
            .collect(),
        cancelled_events: vec![],
        cancel_reason: None,
    }
}

fn handlers(ordinary: Option<&Keys>, recovery: Option<&Keys>) -> failure_routing::FailureHandlers {
    failure_routing::FailureHandlers {
        ordinary: ordinary.map(Keys::public_key),
        recovery: recovery.map(Keys::public_key),
    }
}

async fn fixture(
    channel_id: uuid::Uuid,
    owner: &Keys,
    valid_agents: &[&Keys],
    members: &[&Keys],
    invalid_attestation: bool,
) -> (
    relay::RestClient,
    Arc<Mutex<Capture>>,
    tokio::task::JoinHandle<()>,
) {
    fixture_impl(
        channel_id,
        owner,
        valid_agents,
        members,
        invalid_attestation,
        MembershipMode::Valid,
        None,
    )
    .await
}

async fn fixture_mode(
    channel_id: uuid::Uuid,
    owner: &Keys,
    valid_agents: &[&Keys],
    members: &[&Keys],
    invalid_attestation: bool,
    mode: MembershipMode,
) -> (
    relay::RestClient,
    Arc<Mutex<Capture>>,
    tokio::task::JoinHandle<()>,
) {
    fixture_impl(
        channel_id,
        owner,
        valid_agents,
        members,
        invalid_attestation,
        mode,
        None,
    )
    .await
}

async fn fixture_impl(
    channel_id: uuid::Uuid,
    owner: &Keys,
    valid_agents: &[&Keys],
    members: &[&Keys],
    invalid_attestation: bool,
    mode: MembershipMode,
    invalid_profile: Option<String>,
) -> (
    relay::RestClient,
    Arc<Mutex<Capture>>,
    tokio::task::JoinHandle<()>,
) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let owner = owner.clone();
    let relay_keys = Keys::generate();
    let valid: Vec<String> = valid_agents
        .iter()
        .map(|k| k.public_key().to_hex())
        .collect();
    let members: Vec<String> = members.iter().map(|k| k.public_key().to_hex()).collect();
    let membership_tags: Vec<Tag> =
        std::iter::once(Tag::parse(["d".to_string(), channel_id.to_string()]).unwrap())
            .chain(
                members
                    .iter()
                    .map(|key| Tag::parse(["p".to_string(), key.clone()]).unwrap()),
            )
            .collect();
    let membership_signer = match mode {
        MembershipMode::WrongSigner => Keys::generate(),
        MembershipMode::Valid | MembershipMode::Tampered | MembershipMode::SlowProfiles => {
            relay_keys.clone()
        }
    };
    let mut membership = EventBuilder::new(Kind::Custom(39002), "")
        .tags(membership_tags)
        .sign_with_keys(&membership_signer)
        .unwrap();
    if matches!(mode, MembershipMode::Tampered) {
        membership.content.push_str(" tampered");
    }
    let relay_self = relay_keys.public_key().to_hex();
    let captured = Arc::new(Mutex::new(Capture::default()));
    let captured_server = Arc::clone(&captured);
    let server = tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                break;
            };
            let mut request = Vec::new();
            let mut chunk = [0_u8; 4096];
            let header_end = loop {
                let Ok(size) = socket.read(&mut chunk).await else {
                    return;
                };
                if size == 0 || request.len() > 64 * 1024 {
                    return;
                }
                request.extend_from_slice(&chunk[..size]);
                if let Some(pos) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                    break pos + 4;
                }
            };
            let headers = String::from_utf8_lossy(&request[..header_end]).into_owned();
            let length = headers
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .map(str::trim)
                        .and_then(|v| v.parse::<usize>().ok())
                })
                .unwrap_or(0);
            while request.len() - header_end < length {
                let Ok(size) = socket.read(&mut chunk).await else {
                    return;
                };
                if size == 0 || request.len() + size > 128 * 1024 {
                    return;
                }
                request.extend_from_slice(&chunk[..size]);
            }
            let body = &request[header_end..header_end + length];
            let response = if headers.starts_with("GET /") {
                serde_json::json!({"self": relay_self})
            } else if headers.starts_with("POST /events") {
                if let Ok(submitted) = serde_json::from_slice::<Event>(body) {
                    captured_server.lock().unwrap().events.push(submitted);
                }
                serde_json::json!({})
            } else if headers.starts_with("POST /query") {
                captured_server.lock().unwrap().queries += 1;
                let text = String::from_utf8_lossy(body);
                if text.contains("39002") {
                    serde_json::json!([membership.clone()])
                } else {
                    if matches!(mode, MembershipMode::SlowProfiles) {
                        tokio::time::sleep(Duration::from_millis(1500)).await;
                    }
                    let author = serde_json::from_slice::<serde_json::Value>(body)
                        .ok()
                        .and_then(|v| {
                            v.get(0)
                                .and_then(|f| f.get("authors"))
                                .and_then(|a| a.get(0))
                                .and_then(|v| v.as_str())
                                .map(str::to_owned)
                        });
                    author
                        .filter(|key| valid.contains(key))
                        .map(|key| {
                            let agent = nostr::PublicKey::from_hex(&key).unwrap();
                            // Corrupt one profile when isolating source authorization
                            // from the independently valid handler's attestation.
                            let auth = if invalid_attestation
                                || invalid_profile.as_ref() == Some(&key)
                            {
                                serde_json::json!([
                                    "auth",
                                    owner.public_key().to_hex(),
                                    "kind=9",
                                    "0".repeat(128)
                                ])
                            } else {
                                serde_json::from_str::<serde_json::Value>(
                                    &buzz_sdk::nip_oa::compute_auth_tag(&owner, &agent, "kind=9")
                                        .unwrap(),
                                )
                                .unwrap()
                            };
                            serde_json::json!([{"pubkey": key, "tags": [auth]}])
                        })
                        .unwrap_or_else(|| serde_json::json!([]))
                }
            } else {
                serde_json::json!({})
            };
            let encoded = serde_json::to_vec(&response).unwrap();
            let reply = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                encoded.len()
            );
            socket.write_all(reply.as_bytes()).await.unwrap();
            socket.write_all(&encoded).await.unwrap();
            if headers.starts_with("POST /events") {
                return;
            }
        }
    });
    (
        relay::RestClient {
            http: reqwest::Client::new(),
            base_url,
            keys: Keys::generate(),
            auth_tag_json: None,
        },
        captured,
        server,
    )
}

#[tokio::test]
async fn production_post_routes_one_owner_request_to_attested_member() {
    let owner = Keys::generate();
    let processing = Keys::generate();
    let sibling = Keys::generate();
    let channel = uuid::Uuid::new_v4();
    let (mut rest, captured, server) =
        fixture(channel, &owner, &[&sibling], &[&sibling], false).await;
    rest.keys = processing.clone();
    let request = event(&owner, channel, "request", false, &[]);
    pool::post_failure_notice(
        &rest,
        &batch(channel, vec![request]),
        "limit",
        &handlers(Some(&sibling), None),
        Some(owner.public_key().to_hex()),
    )
    .await;
    server.await.unwrap();
    let guard = captured.lock().unwrap();
    let events = &guard.events;
    assert_eq!(events.len(), 1);
    assert_eq!(
        events[0]
            .tags
            .iter()
            .filter(|tag| tag.as_slice().first().map(String::as_str) == Some("p"))
            .count(),
        1
    );
    assert_eq!(
        events[0]
            .tags
            .iter()
            .find(|tag| tag.as_slice().first().map(String::as_str) == Some("p"))
            .unwrap()
            .as_slice()[1],
        sibling.public_key().to_hex()
    );
    assert!(events[0].verify().is_ok());
}

#[tokio::test]
async fn nonowner_invalid_attestation_and_missing_membership_are_blocked() {
    let owner = Keys::generate();
    let sibling = Keys::generate();
    let stranger = Keys::generate();
    let channel = uuid::Uuid::new_v4();
    let (rest, _, server) = fixture(channel, &owner, &[&sibling], &[&sibling], false).await;
    let denied = failure_routing::resolve(
        &rest,
        &batch(channel, vec![event(&stranger, channel, "x", false, &[])]),
        &handlers(Some(&sibling), None),
        Some(owner.public_key().to_hex()),
    )
    .await;
    assert!(denied.is_none());
    server.abort();

    let (rest, _, server) = fixture(channel, &owner, &[&owner, &sibling], &[&sibling], false).await;
    let mut forged = event(&owner, channel, "signed", false, &[]);
    forged.content.push_str(" mutated");
    assert!(failure_routing::resolve(
        &rest,
        &batch(channel, vec![forged]),
        &handlers(Some(&sibling), None),
        Some(owner.public_key().to_hex())
    )
    .await
    .is_none());
    server.abort();

    // The source is the owner, so only the handler attestation changes.
    for invalid_attestation in [false, true] {
        let (rest, _, server) = fixture(
            channel,
            &owner,
            &[&sibling],
            &[&sibling],
            invalid_attestation,
        )
        .await;
        let route = failure_routing::resolve(
            &rest,
            &batch(
                channel,
                vec![event(&owner, channel, "handler-control", false, &[])],
            ),
            &handlers(Some(&sibling), None),
            Some(owner.public_key().to_hex()),
        )
        .await;
        assert_eq!(
            route.is_some(),
            !invalid_attestation,
            "owner source isolates the handler attestation guard"
        );
        server.abort();
    }

    let (rest, _, server) = fixture(channel, &owner, &[&sibling], &[], false).await;
    let missing = failure_routing::resolve(
        &rest,
        &batch(channel, vec![event(&owner, channel, "x", false, &[])]),
        &handlers(Some(&sibling), None),
        Some(owner.public_key().to_hex()),
    )
    .await;
    assert!(missing.is_none());
    server.abort();

    let other_channel = uuid::Uuid::new_v4();
    let (rest, _, server) = fixture(channel, &owner, &[&owner, &sibling], &[&sibling], false).await;
    assert!(
        failure_routing::resolve(
            &rest,
            &batch(
                channel,
                vec![event(&owner, channel, "channel-control", false, &[])]
            ),
            &handlers(Some(&sibling), None),
            Some(owner.public_key().to_hex()),
        )
        .await
        .is_some(),
        "membership is valid for the batch channel"
    );
    let wrong_channel = failure_routing::resolve(
        &rest,
        &batch(
            channel,
            vec![event(&owner, other_channel, "wrong-channel", false, &[])],
        ),
        &handlers(Some(&sibling), None),
        Some(owner.public_key().to_hex()),
    )
    .await;
    assert!(wrong_channel.is_none());
    server.abort();
}

#[tokio::test]
async fn source_attestation_is_independent_of_valid_handler_attestation() {
    let owner = Keys::generate();
    let source = Keys::generate();
    let sibling = Keys::generate();
    let channel = uuid::Uuid::new_v4();
    for invalid_profile in [None, Some(source.public_key().to_hex())] {
        let should_accept = invalid_profile.is_none();
        let (rest, _, server) = fixture_impl(
            channel,
            &owner,
            &[&source, &sibling],
            &[&sibling],
            false,
            MembershipMode::Valid,
            invalid_profile,
        )
        .await;
        let route = failure_routing::resolve(
            &rest,
            &batch(
                channel,
                vec![event(&source, channel, "source-control", false, &[])],
            ),
            &handlers(Some(&sibling), None),
            Some(owner.public_key().to_hex()),
        )
        .await;
        assert_eq!(
            route.is_some(),
            should_accept,
            "only the source profile attestation differs between controls"
        );
        server.abort();
    }
}

#[tokio::test]
async fn membership_requires_active_relay_signature_and_intact_event() {
    let owner = Keys::generate();
    let sibling = Keys::generate();
    let channel = uuid::Uuid::new_v4();
    for mode in [MembershipMode::WrongSigner, MembershipMode::Tampered] {
        let (rest, _, server) =
            fixture_mode(channel, &owner, &[&sibling], &[&sibling], false, mode).await;
        assert!(failure_routing::resolve(
            &rest,
            &batch(channel, vec![event(&owner, channel, "request", false, &[])]),
            &handlers(Some(&sibling), None),
            Some(owner.public_key().to_hex()),
        )
        .await
        .is_none());
        server.abort();
    }
}

#[tokio::test]
async fn recovery_chain_a_to_b_to_c_stops_before_a_and_mixed_batch_keeps_visited() {
    let owner = Keys::generate();
    let a = Keys::generate();
    let b = Keys::generate();
    let c = Keys::generate();
    let channel = uuid::Uuid::new_v4();
    let mut notice = event(&owner, channel, "request", false, &[]);
    for (own, next) in [(&a, &b), (&b, &c)] {
        let (mut rest, _, server) =
            fixture(channel, &owner, &[&a, &b, &c], &[&a, &b, &c], false).await;
        rest.keys = (*own).clone();
        let route = failure_routing::resolve(
            &rest,
            &batch(channel, vec![notice.clone()]),
            &handlers(Some(next), Some(next)),
            Some(owner.public_key().to_hex()),
        )
        .await
        .expect("route");
        notice = failure_notice::build(own, &batch(channel, vec![notice]), "failed", Some(&route))
            .unwrap();
        server.abort();
    }
    let (mut rest, _, server) = fixture(channel, &owner, &[&a, &b, &c], &[&a, &b, &c], false).await;
    rest.keys = c.clone();
    assert!(failure_routing::resolve(
        &rest,
        &batch(channel, vec![notice.clone()]),
        &handlers(None, Some(&a)),
        Some(owner.public_key().to_hex())
    )
    .await
    .is_none());
    server.abort();
    let fresh = event(&owner, channel, "fresh", false, &[]);
    let (mut rest, _, server) = fixture(channel, &owner, &[&a, &b, &c], &[&a, &b, &c], false).await;
    rest.keys = c.clone();
    assert!(failure_routing::resolve(
        &rest,
        &batch(channel, vec![notice, fresh]),
        &handlers(Some(&a), None),
        Some(owner.public_key().to_hex())
    )
    .await
    .is_none());
    server.abort();
}

#[tokio::test]
async fn defaults_do_not_query_and_invalid_chains_fail_closed() {
    let owner = Keys::generate();
    let sibling = Keys::generate();
    let channel = uuid::Uuid::new_v4();
    let (rest, captured, server) = fixture(channel, &owner, &[&sibling], &[&sibling], false).await;
    let input = batch(channel, vec![event(&owner, channel, "request", false, &[])]);
    assert!(failure_routing::resolve(
        &rest,
        &input,
        &Default::default(),
        Some(owner.public_key().to_hex())
    )
    .await
    .is_none());
    assert_eq!(captured.lock().unwrap().queries, 0);
    // Positive control: this exact fixture must accept the configured route.
    assert!(failure_routing::resolve(
        &rest,
        &input,
        &handlers(Some(&sibling), None),
        Some(owner.public_key().to_hex())
    )
    .await
    .is_some());
    let queries = captured.lock().unwrap().queries;
    let malformed = EventBuilder::new(Kind::Custom(9), "bad")
        .tag(Tag::parse(["h".to_string(), channel.to_string()]).unwrap())
        .tag(Tag::parse(["buzz:agent-failure", "1"]).unwrap())
        .tag(Tag::parse([failure_routing::VISITED_TAG, "bad", "extra"]).unwrap())
        .sign_with_keys(&owner)
        .unwrap();
    assert!(failure_routing::resolve(
        &rest,
        &batch(channel, vec![malformed]),
        &handlers(None, Some(&sibling)),
        Some(owner.public_key().to_hex())
    )
    .await
    .is_none());
    let visited: Vec<String> = (0..8)
        .map(|_| Keys::generate().public_key().to_hex())
        .collect();
    assert!(failure_routing::resolve(
        &rest,
        &batch(
            channel,
            vec![event(&owner, channel, "long", true, &visited)]
        ),
        &handlers(None, Some(&sibling)),
        Some(owner.public_key().to_hex())
    )
    .await
    .is_none());
    let sources = (0..257)
        .map(|index| event(&owner, channel, &format!("source-{index}"), false, &[]))
        .collect();
    assert!(failure_routing::resolve(
        &rest,
        &batch(channel, sources),
        &handlers(Some(&sibling), None),
        Some(owner.public_key().to_hex())
    )
    .await
    .is_none());
    assert_eq!(
        captured.lock().unwrap().queries,
        queries,
        "invalid chains must stop before querying"
    );
    server.abort();
}

#[tokio::test]
async fn preflight_deadline_bounds_all_lookups_together() {
    let owner = Keys::generate();
    let agents: Vec<Keys> = (0..4).map(|_| Keys::generate()).collect();
    let channel = uuid::Uuid::new_v4();
    let refs: Vec<&Keys> = agents.iter().collect();
    let (rest, captured, server) = fixture_mode(
        channel,
        &owner,
        &refs,
        &refs,
        false,
        MembershipMode::SlowProfiles,
    )
    .await;
    let sources = agents[..3]
        .iter()
        .map(|key| event(key, channel, "request", false, &[]))
        .collect();
    let input = batch(channel, sources);
    let choices = handlers(Some(&agents[3]), None);
    let route = tokio::time::timeout(
        Duration::from_secs(8),
        failure_routing::resolve(&rest, &input, &choices, Some(owner.public_key().to_hex())),
    )
    .await
    .unwrap();
    assert!(
        route.is_none(),
        "four individually valid 1.5-second lookups exceed the total five-second budget"
    );
    assert_eq!(
        captured.lock().unwrap().queries,
        4,
        "must reach the fourth lookup rather than fail on an unrelated author check"
    );
    server.abort();
}
