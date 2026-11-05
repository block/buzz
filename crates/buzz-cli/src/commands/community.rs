//! `buzz community` — community member management and invite links.
//!
//! `members list` reads the relay-signed NIP-43 membership list (kind 13534).
//! `members add|remove|set-role` publish the relay-admin commands 9030–9032;
//! the relay authorizes each one (add/remove: owner or admin, set-role: owner)
//! and never stores them. `invite create` mints an invite through the
//! NIP-98-signed `POST /api/invites` endpoint.
//!
//! The community (tenant) is selected by the relay host in `--relay` /
//! `BUZZ_RELAY_URL`.

use std::collections::HashSet;

use buzz_core::invite::{MAX_INVITE_TTL_SECS, MAX_INVITE_USES, MIN_INVITE_TTL_SECS};
use buzz_core::kind::KIND_NIP43_MEMBERSHIP_LIST;
use serde_json::{json, Value};

use crate::client::{normalize_write_response, BuzzClient};
use crate::error::CliError;
use crate::validate::{sdk_err, validate_hex64};
use crate::{CommunityCmd, CommunityInviteCmd, CommunityMembersCmd, OutputFormat};

const ROLE_ORDER: [&str; 3] = ["owner", "admin", "member"];

/// Resolve a `--pubkey` value (64-char hex or `npub1…`) to lowercase hex.
fn parse_pubkey(value: &str) -> Result<String, CliError> {
    let value = value.trim();
    if value.starts_with("npub1") {
        return nostr::PublicKey::parse(value)
            .map(|pk| pk.to_hex())
            .map_err(|_| CliError::Usage(format!("invalid npub: {value}")));
    }
    validate_hex64(value)?;
    Ok(value.to_ascii_lowercase())
}

/// Resolve and de-duplicate `--pubkey` values, keeping first-seen order.
fn parse_pubkeys(values: &[String]) -> Result<Vec<String>, CliError> {
    let mut seen = HashSet::new();
    let mut pubkeys = Vec::new();
    for value in values {
        let pubkey = parse_pubkey(value)?;
        if seen.insert(pubkey.clone()) {
            pubkeys.push(pubkey);
        }
    }
    Ok(pubkeys)
}

/// Parse a `--ttl` value: whole seconds, or a number with an `s`, `m`, `h`, or
/// `d` suffix (`90`, `30m`, `72h`, `7d`). The relay's bounds apply.
fn parse_ttl_secs(value: &str) -> Result<u64, CliError> {
    let value = value.trim();
    let (digits, unit) = match value.char_indices().last() {
        Some((i, c)) if c.is_ascii_alphabetic() => (&value[..i], c.to_ascii_lowercase()),
        _ => (value, 's'),
    };
    let multiplier: u64 = match unit {
        's' => 1,
        'm' => 60,
        'h' => 3_600,
        'd' => 86_400,
        _ => {
            return Err(CliError::Usage(format!(
                "invalid --ttl '{value}': use seconds or a number with s, m, h, or d (e.g. 72h)"
            )))
        }
    };
    let secs = digits
        .parse::<u64>()
        .ok()
        .and_then(|n| n.checked_mul(multiplier))
        .ok_or_else(|| {
            CliError::Usage(format!(
                "invalid --ttl '{value}': use seconds or a number with s, m, h, or d (e.g. 72h)"
            ))
        })?;
    if !(MIN_INVITE_TTL_SECS..=MAX_INVITE_TTL_SECS).contains(&secs) {
        return Err(CliError::Usage(format!(
            "--ttl must be between {MIN_INVITE_TTL_SECS} seconds and 30 days (got {secs} seconds)"
        )));
    }
    Ok(secs)
}

/// Re-classify the relay's 400 refusals of an admin command. Authorization
/// refusals exit 3, an unknown member exits 1, and other refusals are input
/// errors; database failures keep the relay-error classification.
fn classify_admin_error(error: CliError) -> CliError {
    let CliError::Relay { status: 400, body } = error else {
        return error;
    };
    if body.contains("not authorized") {
        CliError::Auth(body)
    } else if body.contains("member not found") {
        CliError::NotFound(body)
    } else if body.contains("database error") || !body.starts_with("invalid:") {
        CliError::Relay { status: 400, body }
    } else {
        CliError::Usage(body)
    }
}

fn parse_membership_list(events: &[Value]) -> Result<Vec<Value>, CliError> {
    let newest = events
        .iter()
        .filter(|event| {
            event.get("kind").and_then(Value::as_u64) == Some(KIND_NIP43_MEMBERSHIP_LIST as u64)
        })
        .map(|event| {
            let event: nostr::Event = serde_json::from_value(event.clone()).map_err(|e| {
                CliError::Other(format!("relay returned a malformed membership list: {e}"))
            })?;
            event.verify().map_err(|e| {
                CliError::Other(format!(
                    "relay returned a membership list that fails verification: {e}"
                ))
            })?;
            Ok(event)
        })
        .collect::<Result<Vec<_>, CliError>>()?
        .into_iter()
        .max_by_key(|event| event.created_at);
    let Some(event) = newest else {
        return Err(CliError::NotFound(
            "the relay published no membership list (open relays do not publish one)".into(),
        ));
    };

    let mut seen = HashSet::new();
    let mut members = Vec::new();
    for tag in event.tags.iter() {
        let parts = tag.as_slice();
        let (raw_pubkey, raw_role) = match parts.first().map(String::as_str) {
            Some("member") => (parts.get(1), parts.get(2)),
            Some("p") => (parts.get(1), parts.get(3)),
            _ => continue,
        };
        let Some(pubkey) = raw_pubkey.map(|p| p.trim().to_ascii_lowercase()) else {
            continue;
        };
        if pubkey.len() != 64
            || !pubkey
                .bytes()
                .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
            || !seen.insert(pubkey.clone())
        {
            continue;
        }
        let role = raw_role
            .map(String::as_str)
            .filter(|role| ROLE_ORDER.contains(role))
            .unwrap_or("member");
        members.push(json!({"pubkey": pubkey, "role": role}));
    }
    members.sort_by_key(|member| {
        ROLE_ORDER
            .iter()
            .position(|role| Some(*role) == member["role"].as_str())
            .unwrap_or(ROLE_ORDER.len())
    });
    Ok(members)
}

async fn members_list(client: &BuzzClient) -> Result<Value, CliError> {
    let filter = json!({"kinds": [KIND_NIP43_MEMBERSHIP_LIST], "limit": 1});
    let raw = client.query(&filter).await?;
    let events: Vec<Value> = serde_json::from_str(&raw)
        .map_err(|e| CliError::Other(format!("failed to parse membership list query: {e}")))?;
    Ok(Value::Array(parse_membership_list(&events)?))
}

/// Publish one admin command per pubkey, in order, stopping at the first
/// failure. Returns the results of the commands the relay accepted and the
/// failure, if any, so a partial run is never reported as a clean one.
async fn publish_member_commands(
    client: &BuzzClient,
    pubkeys: &[String],
    build: impl Fn(&str) -> Result<nostr::EventBuilder, buzz_sdk::SdkError>,
) -> (Vec<Value>, Option<CliError>) {
    let mut applied = Vec::new();
    for pubkey in pubkeys {
        let outcome = async {
            let event = client.sign_event(build(pubkey).map_err(sdk_err)?)?;
            client
                .submit_event(event)
                .await
                .map_err(classify_admin_error)
        }
        .await;
        match outcome {
            Ok(resp) => {
                let mut result: Value =
                    serde_json::from_str(&normalize_write_response(&resp)).unwrap_or(Value::Null);
                if let Some(result) = result.as_object_mut() {
                    result.insert("pubkey".into(), json!(pubkey));
                }
                applied.push(result);
            }
            Err(error) => return (applied, Some(error)),
        }
    }
    (applied, None)
}

async fn members_add(
    client: &BuzzClient,
    pubkeys: &[String],
    role: Option<&str>,
) -> (Vec<Value>, Option<CliError>) {
    let pubkeys = match parse_pubkeys(pubkeys) {
        Ok(pubkeys) => pubkeys,
        Err(e) => return (Vec::new(), Some(e)),
    };
    publish_member_commands(client, &pubkeys, |pk| {
        buzz_sdk::build_relay_member_add(pk, role)
    })
    .await
}

async fn members_remove(client: &BuzzClient, pubkeys: &[String]) -> (Vec<Value>, Option<CliError>) {
    let pubkeys = match parse_pubkeys(pubkeys) {
        Ok(pubkeys) => pubkeys,
        Err(e) => return (Vec::new(), Some(e)),
    };
    publish_member_commands(client, &pubkeys, buzz_sdk::build_relay_member_remove).await
}

async fn members_set_role(
    client: &BuzzClient,
    pubkey: &str,
    role: &str,
) -> (Vec<Value>, Option<CliError>) {
    let pubkeys = match parse_pubkey(pubkey) {
        Ok(pubkey) => vec![pubkey],
        Err(e) => return (Vec::new(), Some(e)),
    };
    publish_member_commands(client, &pubkeys, |pk| {
        buzz_sdk::build_relay_member_set_role(pk, role)
    })
    .await
}

async fn invite_create(
    client: &BuzzClient,
    ttl: Option<&str>,
    max_uses: Option<i32>,
) -> Result<Value, CliError> {
    let mut body = serde_json::Map::new();
    if let Some(ttl) = ttl {
        body.insert("ttl_secs".into(), json!(parse_ttl_secs(ttl)?));
    }
    if let Some(max_uses) = max_uses {
        if !(1..=MAX_INVITE_USES).contains(&max_uses) {
            return Err(CliError::Usage(format!(
                "--max-uses must be between 1 and {MAX_INVITE_USES}"
            )));
        }
        body.insert("max_uses".into(), json!(max_uses));
    }
    let raw = client
        .post_json_once_authed("/api/invites", &Value::Object(body))
        .await?;
    let response: Value = serde_json::from_str(&raw)
        .map_err(|e| CliError::Other(format!("relay returned an invalid invite response: {e}")))?;
    let field = |name: &str| {
        response
            .get(name)
            .cloned()
            .ok_or_else(|| CliError::Other(format!("relay invite response is missing `{name}`")))
    };
    Ok(json!({
        "code": field("code")?,
        "url": field("url")?,
        "expires_at": field("expires_at")?,
        "max_uses": response.get("max_uses").cloned().unwrap_or(Value::Null),
        "uses_remaining": response.get("uses_remaining").cloned().unwrap_or(Value::Null),
    }))
}

fn print_member_commands(applied: Vec<Value>, failure: Option<CliError>) -> Result<(), CliError> {
    if !applied.is_empty() || failure.is_none() {
        println!("{}", Value::Array(applied));
    }
    failure.map_or(Ok(()), Err)
}

pub async fn dispatch(
    cmd: CommunityCmd,
    client: &BuzzClient,
    _format: &OutputFormat,
) -> Result<(), CliError> {
    match cmd {
        CommunityCmd::Members(CommunityMembersCmd::List) => {
            println!("{}", members_list(client).await?);
            Ok(())
        }
        CommunityCmd::Members(CommunityMembersCmd::Add { pubkey, role }) => {
            let (applied, failure) = members_add(client, &pubkey, role.as_deref()).await;
            print_member_commands(applied, failure)
        }
        CommunityCmd::Members(CommunityMembersCmd::Remove { pubkey }) => {
            let (applied, failure) = members_remove(client, &pubkey).await;
            print_member_commands(applied, failure)
        }
        CommunityCmd::Members(CommunityMembersCmd::SetRole { pubkey, role }) => {
            let (applied, failure) = members_set_role(client, &pubkey, &role).await;
            print_member_commands(applied, failure)
        }
        CommunityCmd::Invite(CommunityInviteCmd::Create { ttl, max_uses }) => {
            println!("{}", invite_create(client, ttl.as_deref(), max_uses).await?);
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::Bytes,
        extract::State,
        http::{HeaderMap, StatusCode},
        routing::post,
        Router,
    };
    use base64::Engine as _;
    use nostr::{nips::nip19::ToBech32, EventBuilder, Keys, Kind, Tag, Timestamp};
    use sha2::{Digest, Sha256};
    use std::sync::{Arc, Mutex};
    use tokio::net::TcpListener;

    #[derive(Default)]
    struct Relay {
        list_events: Vec<Value>,
        events: Vec<Value>,
        reject_pubkey: Option<(String, StatusCode, String)>,
        invites: Vec<Value>,
        invite_reply: Option<(StatusCode, String)>,
        nip98_failures: Vec<String>,
    }
    type Shared = Arc<Mutex<Relay>>;

    fn check_nip98(headers: &HeaderMap, url_path: &str, body: &[u8]) -> Vec<String> {
        let mut failures = Vec::new();
        let Some(header) = headers.get("authorization").and_then(|h| h.to_str().ok()) else {
            return vec!["missing Authorization".into()];
        };
        let Some(encoded) = header.strip_prefix("Nostr ") else {
            return vec!["not a Nostr scheme".into()];
        };
        let json = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .unwrap();
        let event: nostr::Event = serde_json::from_slice(&json).unwrap();
        if event.verify().is_err() {
            failures.push("bad signature".into());
        }
        let tag = |name: &str| {
            event
                .tags
                .iter()
                .find(|t| t.as_slice().first().map(String::as_str) == Some(name))
                .and_then(|t| t.as_slice().get(1).cloned())
        };
        if event.kind.as_u16() != 27235 {
            failures.push("wrong kind".into());
        }
        if tag("method").as_deref() != Some("POST") {
            failures.push("wrong method".into());
        }
        if !tag("u").is_some_and(|u| u.ends_with(url_path)) {
            failures.push("wrong url".into());
        }
        if tag("payload") != Some(hex::encode(Sha256::digest(body))) {
            failures.push("payload hash does not cover the body".into());
        }
        failures
    }

    async fn fake_relay(relay: Shared) -> String {
        let app = Router::new()
            .route(
                "/query",
                post(|State(relay): State<Shared>, body: Bytes| async move {
                    let filters: Vec<Value> = serde_json::from_slice(&body).unwrap();
                    let [filter] = filters.as_slice() else {
                        panic!("one filter expected: {filters:?}")
                    };
                    assert_eq!(filter["kinds"], json!([13534]));
                    axum::Json(json!(relay.lock().unwrap().list_events))
                }),
            )
            .route(
                "/events",
                post(|State(relay): State<Shared>, body: Bytes| async move {
                    let event: Value = serde_json::from_slice(&body).unwrap();
                    let mut relay = relay.lock().unwrap();
                    let target = event["tags"][0][1].as_str().unwrap().to_string();
                    if let Some((pubkey, status, message)) = &relay.reject_pubkey {
                        if *pubkey == target {
                            return (*status, json!({"error": message}).to_string());
                        }
                    }
                    relay.events.push(event.clone());
                    (
                        StatusCode::OK,
                        json!({"event_id": event["id"], "accepted": true, "message": ""})
                            .to_string(),
                    )
                }),
            )
            .route(
                "/api/invites",
                post(
                    |State(relay): State<Shared>, headers: HeaderMap, body: Bytes| async move {
                        let mut relay = relay.lock().unwrap();
                        let failures = check_nip98(&headers, "/api/invites", &body);
                        relay.nip98_failures.extend(failures);
                        let request: Value = if body.is_empty() {
                            json!({})
                        } else {
                            serde_json::from_slice(&body).unwrap()
                        };
                        relay.invites.push(request.clone());
                        if let Some((status, body)) = relay.invite_reply.clone() {
                            return (status, body);
                        }
                        (
                            StatusCode::OK,
                            json!({
                                "code": "v2.SECRET",
                                "url": "http://relay.test/invite/v2.SECRET",
                                "expires_at": 1_900_000_000u64,
                                "max_uses": request.get("max_uses").cloned(),
                                "uses_remaining": request.get("max_uses").cloned(),
                            })
                            .to_string(),
                        )
                    },
                ),
            )
            .with_state(relay);
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://{addr}")
    }

    async fn setup() -> (BuzzClient, Shared) {
        let relay = Shared::default();
        let url = fake_relay(relay.clone()).await;
        let client = BuzzClient::new(url, Keys::generate(), None, None).unwrap();
        (client, relay)
    }

    fn pk(c: char) -> String {
        c.to_string().repeat(64)
    }

    fn membership_event(tags: Vec<Vec<String>>, created_at: u64, signer: &Keys) -> Value {
        let tags: Vec<Tag> = tags.into_iter().map(|t| Tag::parse(t).unwrap()).collect();
        let event = EventBuilder::new(Kind::Custom(13534), "")
            .tags(tags)
            .custom_created_at(Timestamp::from(created_at))
            .sign_with_keys(signer)
            .unwrap();
        serde_json::to_value(event).unwrap()
    }

    fn strs(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|s| s.to_string()).collect()
    }

    fn published_tags(relay: &Shared) -> Vec<(u64, Vec<Vec<String>>)> {
        relay
            .lock()
            .unwrap()
            .events
            .iter()
            .map(|e| {
                (
                    e["kind"].as_u64().unwrap(),
                    serde_json::from_value(e["tags"].clone()).unwrap(),
                )
            })
            .collect()
    }

    #[tokio::test]
    async fn list_parses_the_relay_signed_list_like_desktop() {
        let (client, relay) = setup().await;
        let relay_keys = Keys::generate();
        let upper = "AB".repeat(32);
        relay.lock().unwrap().list_events = vec![
            membership_event(vec![strs(&["member", &pk('1'), "member"])], 10, &relay_keys),
            membership_event(
                vec![
                    strs(&["member", &pk('2'), "member"]),
                    strs(&["member", &pk('3'), "owner"]),
                    strs(&["p", &upper, "wss://relay", "admin"]),
                    strs(&["member", &pk('2'), "admin"]),
                    strs(&["member", "not-hex", "admin"]),
                    strs(&["member", &pk('4'), "superuser"]),
                    strs(&["member", &pk('5')]),
                    strs(&["t", "ignored"]),
                ],
                20,
                &relay_keys,
            ),
        ];
        let members = members_list(&client).await.unwrap();
        assert_eq!(
            members,
            json!([
                {"pubkey": pk('3'), "role": "owner"},
                {"pubkey": "ab".repeat(32), "role": "admin"},
                {"pubkey": pk('2'), "role": "member"},
                {"pubkey": pk('4'), "role": "member"},
                {"pubkey": pk('5'), "role": "member"},
            ])
        );

        relay.lock().unwrap().list_events = vec![];
        let err = members_list(&client).await.unwrap_err();
        assert!(matches!(err, CliError::NotFound(_)), "{err:?}");

        let mut forged = membership_event(vec![strs(&["member", &pk('1')])], 5, &relay_keys);
        forged["tags"] = json!([["member", pk('9'), "owner"]]);
        relay.lock().unwrap().list_events = vec![forged];
        let err = members_list(&client).await.unwrap_err();
        assert!(matches!(err, CliError::Other(_)), "{err:?}");
    }

    #[tokio::test]
    async fn add_remove_and_set_role_publish_the_desktop_wire_form() {
        let (client, relay) = setup().await;
        let npub = nostr::PublicKey::from_hex(&pk('b'))
            .unwrap()
            .to_bech32()
            .unwrap();

        let (applied, failure) = members_add(
            &client,
            &[pk('a'), npub, pk('a').to_uppercase()],
            Some("admin"),
        )
        .await;
        assert!(failure.is_none(), "{failure:?}");
        assert_eq!(applied.len(), 2, "duplicates publish once");
        assert!(applied.iter().all(|r| r["accepted"] == json!(true)));
        assert_eq!(applied[1]["pubkey"], json!(pk('b')));

        let (_, failure) = members_add(&client, &[pk('c')], None).await;
        assert!(failure.is_none());
        let (_, failure) = members_remove(&client, &[pk('a')]).await;
        assert!(failure.is_none());
        let (_, failure) = members_set_role(&client, &pk('b'), "member").await;
        assert!(failure.is_none());

        let role_tag = |role: &str| strs(&["role", role]);
        assert_eq!(
            published_tags(&relay),
            vec![
                (9030, vec![strs(&["p", &pk('a')]), role_tag("admin")]),
                (9030, vec![strs(&["p", &pk('b')]), role_tag("admin")]),
                (9030, vec![strs(&["p", &pk('c')])]),
                (9031, vec![strs(&["p", &pk('a')])]),
                (9032, vec![strs(&["p", &pk('b')]), role_tag("member")]),
            ]
        );

        let before = published_tags(&relay).len();
        for (result, label) in [
            (
                members_add(&client, &[pk('d')], Some("owner")).await,
                "owner",
            ),
            (members_add(&client, &["xyz".into()], None).await, "bad key"),
            (
                members_set_role(&client, &pk('d'), "root").await,
                "bad role",
            ),
            (
                members_remove(&client, &[pk('d'), "xyz".into()]).await,
                "late bad",
            ),
        ] {
            assert!(
                matches!(result.1, Some(CliError::Usage(_))),
                "{label}: {:?}",
                result.1
            );
            assert!(result.0.is_empty(), "{label}");
        }
        assert_eq!(
            published_tags(&relay).len(),
            before,
            "no publish on bad input"
        );
    }

    #[tokio::test]
    async fn a_failure_midway_reports_what_was_applied_and_stops() {
        let (client, relay) = setup().await;
        relay.lock().unwrap().reject_pubkey = Some((
            pk('2'),
            StatusCode::BAD_REQUEST,
            "invalid: actor not authorized: must be admin or owner".into(),
        ));
        let (applied, failure) = members_remove(&client, &[pk('1'), pk('2'), pk('3')]).await;
        assert_eq!(applied.len(), 1);
        assert_eq!(applied[0]["pubkey"], json!(pk('1')));
        assert!(matches!(failure, Some(CliError::Auth(_))), "{failure:?}");
        assert_eq!(published_tags(&relay).len(), 1, "the third is never sent");
    }

    #[test]
    fn relay_refusals_map_to_exit_codes() {
        use crate::error::exit_code;
        let refusal = |status, body: &str| {
            exit_code(&classify_admin_error(CliError::Relay {
                status,
                body: body.into(),
            }))
        };
        for (status, body, code) in [
            (400, "invalid: actor not authorized: must be owner", 3),
            (
                400,
                "invalid: actor not authorized: only owner can grant admin role",
                3,
            ),
            (403, "blocked: you are banned from this community", 3),
            (401, "auth-required", 3),
            (400, "invalid: member not found: abc", 1),
            (400, "invalid: cannot remove the relay owner", 1),
            (400, "invalid: invalid role: root", 1),
            (400, "invalid: cannot set role to owner", 1),
            (400, "invalid: database error: pool timed out", 2),
            (500, "error: boom", 2),
            (429, "rate-limited: slow down", 2),
        ] {
            assert_eq!(refusal(status, body), code, "{status} {body}");
        }
        assert!(matches!(
            classify_admin_error(CliError::Relay {
                status: 400,
                body: "invalid: member not found: abc".into()
            }),
            CliError::NotFound(_)
        ));
    }

    #[test]
    fn ttl_accepts_durations_within_relay_bounds() {
        for (input, secs) in [
            ("60", 60),
            ("90s", 90),
            ("30m", 1_800),
            ("72h", 259_200),
            ("30d", 2_592_000),
            (" 2H ", 7_200),
        ] {
            assert_eq!(parse_ttl_secs(input).unwrap(), secs, "{input}");
        }
        for bad in [
            "",
            "h",
            "59",
            "31d",
            "-5m",
            "1.5h",
            "5w",
            "99999999999999999999d",
            "1 h",
        ] {
            assert!(
                matches!(parse_ttl_secs(bad), Err(CliError::Usage(_))),
                "{bad:?}"
            );
        }
    }

    #[tokio::test]
    async fn invite_create_signs_the_body_and_prints_the_relay_fields() {
        let (client, relay) = setup().await;
        let created = invite_create(&client, None, None).await.unwrap();
        assert_eq!(created["code"], json!("v2.SECRET"));
        assert_eq!(created["url"], json!("http://relay.test/invite/v2.SECRET"));
        assert_eq!(created["expires_at"], json!(1_900_000_000u64));
        assert_eq!(created["max_uses"], Value::Null);

        let limited = invite_create(&client, Some("24h"), Some(5)).await.unwrap();
        assert_eq!(limited["max_uses"], json!(5));

        {
            let relay = relay.lock().unwrap();
            assert_eq!(
                relay.invites,
                vec![json!({}), json!({"ttl_secs": 86_400, "max_uses": 5})]
            );
            assert!(
                relay.nip98_failures.is_empty(),
                "{:?}",
                relay.nip98_failures
            );
        }

        for (ttl, uses) in [(Some("5s"), None), (None, Some(0)), (None, Some(10_001))] {
            let err = invite_create(&client, ttl, uses).await.unwrap_err();
            assert!(matches!(err, CliError::Usage(_)), "{err:?}");
        }
        assert_eq!(
            relay.lock().unwrap().invites.len(),
            2,
            "no request on bad input"
        );
    }

    #[tokio::test]
    async fn invite_create_failures_are_loud_and_never_retried() {
        let (client, relay) = setup().await;
        relay.lock().unwrap().invite_reply = Some((
            StatusCode::FORBIDDEN,
            json!({"error": "only relay owners and admins can create invites"}).to_string(),
        ));
        let err = invite_create(&client, None, None).await.unwrap_err();
        assert!(
            matches!(err, CliError::Relay { status: 403, .. }),
            "{err:?}"
        );
        assert_eq!(crate::error::exit_code(&err), 3);

        relay.lock().unwrap().invite_reply = Some((StatusCode::BAD_GATEWAY, "bad gateway".into()));
        let err = invite_create(&client, None, None).await.unwrap_err();
        assert!(matches!(err, CliError::DeliveryUnknown(_)), "{err:?}");

        relay.lock().unwrap().invite_reply =
            Some((StatusCode::OK, json!({"code": "x"}).to_string()));
        let err = invite_create(&client, None, None).await.unwrap_err();
        assert!(matches!(err, CliError::Other(_)), "{err:?}");
        assert_eq!(
            relay.lock().unwrap().invites.len(),
            3,
            "one request per attempt"
        );
    }
}
