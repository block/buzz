//! Local-only signed HTTP/WS NIP-AR integration probe. Run against an ephemeral
//! relay configured with the public test key 1 as its owner. Never use test keys
//! for a real community. See docs/community-artifact-rollout.md.
use base64::Engine;
use futures_util::{SinkExt, StreamExt};
use nostr::{Event, EventBuilder, Keys, Kind, Tag};
use serde_json::{json, Value};
use sha2::Digest;
use std::time::Duration;
use uuid::Uuid;
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn sign(keys: &Keys, kind: u16, tags: Vec<Vec<String>>, content: &str) -> Result<Event> {
    Ok(EventBuilder::new(Kind::Custom(kind), content)
        .tags(
            tags.into_iter()
                .map(Tag::parse)
                .collect::<std::result::Result<Vec<_>, _>>()?,
        )
        .sign_with_keys(keys)?)
}
async fn post(
    client: &reqwest::Client,
    base: &str,
    keys: &Keys,
    path: &str,
    body: Value,
) -> Result<(bool, Value)> {
    let bytes = serde_json::to_vec(&body)?;
    let auth = sign(
        keys,
        27235,
        vec![
            vec!["u".into(), format!("{base}{path}")],
            vec!["method".into(), "POST".into()],
            vec!["payload".into(), hex::encode(sha2::Sha256::digest(&bytes))],
            vec!["nonce".into(), Uuid::new_v4().to_string()],
        ],
        "",
    )?;
    let response = client
        .post(format!("{base}{path}"))
        .header("content-type", "application/json")
        .header(
            "authorization",
            format!(
                "Nostr {}",
                base64::engine::general_purpose::STANDARD.encode(serde_json::to_vec(&auth)?)
            ),
        )
        .body(bytes)
        .send()
        .await?;
    let success = response.status().is_success();
    Ok((success, response.json().await?))
}
async fn publish(client: &reqwest::Client, base: &str, keys: &Keys, event: &Event) -> Result<()> {
    let (ok, body) = post(client, base, keys, "/events", json!(event)).await?;
    if !ok || body["accepted"] != true {
        return Err(format!("publish failed: {body}").into());
    }
    Ok(())
}
fn revision(keys: &Keys, scope: &str, id: &str, d: Uuid, prev: Option<&Event>) -> Result<Event> {
    let mut tags = vec![
        vec!["ar".into(), "1".into()],
        vec![scope.into(), id.into()],
        vec!["d".into(), d.to_string()],
        vec!["type".into(), "example.settings".into()],
        vec!["title".into(), "Probe".into()],
        vec![
            "op".into(),
            if prev.is_some() {
                "update".into()
            } else {
                "create".into()
            },
        ],
    ];
    if let Some(prev) = prev {
        tags.push(vec!["prev".into(), prev.id.to_hex()]);
    }
    sign(keys, 45010, tags, "private probe payload")
}
/// Exercise the unchanged channel wire protocol on expand and contract, also
/// against the previous relay binary. No community capability is required.
async fn channel_compat(client: &reqwest::Client, base: &str) -> Result<()> {
    let owner = Keys::parse(&format!("{:064x}", 1))?;
    let homes = [Uuid::new_v4(), Uuid::new_v4()];
    for home in homes {
        let event = sign(
            &owner,
            9007,
            vec![
                vec!["h".into(), home.to_string()],
                vec!["name".into(), format!("compat-{home}")],
            ],
            "",
        )?;
        publish(client, base, &owner, &event).await?;
    }
    let d = Uuid::new_v4();
    let first = revision(&owner, "h", &homes[0].to_string(), d, None)?;
    publish(client, base, &owner, &first).await?;
    let update = revision(&owner, "h", &homes[0].to_string(), d, Some(&first))?;
    publish(client, base, &owner, &update).await?;
    publish(client, base, &owner, &first).await?;
    let (ok, heads) = post(
        client,
        base,
        &owner,
        "/query",
        json!([{"artifact":"current","#d":[d]}]),
    )
    .await?;
    assert!(ok);
    assert_eq!(heads, json!([update]));
    let moved = sign(
        &owner,
        45010,
        vec![
            vec!["ar".into(), "1".into()],
            vec!["h".into(), homes[1].to_string()],
            vec!["d".into(), d.to_string()],
            vec!["type".into(), "example.settings".into()],
            vec!["title".into(), "Moved probe".into()],
            vec!["op".into(), "move".into()],
            vec!["prev".into(), update.id.to_hex()],
        ],
        "moved probe payload",
    )?;
    publish(client, base, &owner, &moved).await?;
    let (ok, heads) = post(
        client,
        base,
        &owner,
        "/query",
        json!([{"artifact":"head","#d":[d]}]),
    )
    .await?;
    assert!(ok);
    assert_eq!(heads, json!([moved]));
    let (ok, history) = post(
        client,
        base,
        &owner,
        "/query",
        json!([{"artifact":"history","#d":[d]}]),
    )
    .await?;
    assert!(ok);
    let history = history.as_array().ok_or("history array")?;
    assert_eq!(history.len(), 3);
    for event in [&first, &update, &moved] {
        assert!(
            history.contains(&json!(event)),
            "exact signed channel event preserved"
        );
    }
    println!("PASS: exact signed channel create/update/replay/move/head/history wire output");
    Ok(())
}
async fn next(
    socket: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
) -> Result<Value> {
    loop {
        let message = tokio::time::timeout(Duration::from_secs(10), socket.next())
            .await?
            .ok_or("socket ended")??;
        if let tokio_tungstenite::tungstenite::Message::Text(text) = message {
            return Ok(serde_json::from_str(&text)?);
        }
    }
}
#[tokio::main]
async fn main() -> Result<()> {
    let base = std::env::args()
        .nth(1)
        .ok_or("usage: community_artifact_probe http://localhost:3030 [--expect-disabled | --channel-only]")?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()?;
    let info: Value = client
        .get(&base)
        .header("accept", "application/nostr+json")
        .send()
        .await?
        .json()
        .await?;
    if std::env::args().any(|s| s == "--expect-disabled") {
        assert!(info["artifacts"].get("community_scope").is_none());
        println!("PASS: community capability withheld on expand/disabled schema");
        return channel_compat(&client, &base).await;
    }
    if std::env::args().any(|s| s == "--channel-only") {
        return channel_compat(&client, &base).await;
    }
    let capability = &info["artifacts"]["community_scope"];
    assert_eq!(capability["version"], 1);
    assert_eq!(capability["max_policies"], 256);
    let community = capability["community_id"]
        .as_str()
        .ok_or("community capability missing")?;
    assert_eq!(Uuid::parse_str(community)?.to_string(), community);
    let owner = Keys::parse(&format!("{:064x}", 1))?;
    let peer = Keys::parse(&format!("{:064x}", 2))?;
    let (ok, policy) = post(
        &client,
        &base,
        &owner,
        "/query",
        json!([{"artifact":"policy","scope":"community","#type":["example.settings"]}]),
    )
    .await?;
    assert!(ok);
    let mut policy_prev = policy["revision"].as_str().unwrap_or("0").to_owned();
    if policy["policy"] != "everyone" {
        let reset = sign(
            &owner,
            9034,
            vec![
                vec!["community".into(), community.into()],
                vec!["type".into(), "example.settings".into()],
                vec!["policy".into(), "everyone".into()],
                vec!["prev".into(), policy_prev],
            ],
            "",
        )?;
        publish(&client, &base, &owner, &reset).await?;
        policy_prev = reset.id.to_hex();
    }
    let d = Uuid::new_v4();
    let first = revision(&peer, "community", community, d, None)?;
    publish(&client, &base, &peer, &first).await?;
    let current = json!([{"artifact":"current","scope":"community","#type":["example.settings"]}]);
    let (ok, heads) = post(&client, &base, &peer, "/query", current.clone()).await?;
    assert!(ok);
    assert!(heads
        .as_array()
        .ok_or("heads array")?
        .iter()
        .any(|e| e["id"] == first.id.to_hex()));
    let unsigned = client
        .post(format!("{base}/query"))
        .json(&current)
        .send()
        .await?;
    assert!(!unsigned.status().is_success());
    let ws_url = base.replacen("http", "ws", 1);
    let (mut ws, _) = tokio_tungstenite::connect_async(&ws_url).await?;
    let challenge = loop {
        let message = next(&mut ws).await?;
        if message[0] == "AUTH" {
            break message[1].as_str().ok_or("challenge")?.to_owned();
        }
    };
    let auth = sign(
        &peer,
        22242,
        vec![
            vec!["relay".into(), ws_url],
            vec!["challenge".into(), challenge],
        ],
        "",
    )?;
    ws.send(tokio_tungstenite::tungstenite::Message::Text(
        json!(["AUTH", auth]).to_string().into(),
    ))
    .await?;
    loop {
        let message = next(&mut ws).await?;
        if message[0] == "OK" {
            assert_eq!(message[2], true);
            break;
        }
    }
    ws.send(tokio_tungstenite::tungstenite::Message::Text(
        json!(["REQ","s1",{"artifact":"current","scope":"community","#type":["example.settings"]}])
            .to_string()
            .into(),
    ))
    .await?;
    let mut seen = false;
    loop {
        let message = next(&mut ws).await?;
        match message[0].as_str() {
            Some("EVENT") => seen |= message[2]["id"] == first.id.to_hex(),
            Some("EOSE") => break,
            _ => return Err(format!("unexpected snapshot frame {message}").into()),
        }
    }
    assert!(seen);
    let update = revision(&peer, "community", community, d, Some(&first))?;
    publish(&client, &base, &peer, &update).await?;
    let message = next(&mut ws).await?;
    assert_eq!(message[0], "EVENT");
    assert_eq!(message[2]["id"], update.id.to_hex());
    let command = sign(
        &owner,
        9034,
        vec![
            vec!["community".into(), community.into()],
            vec!["type".into(), "example.settings".into()],
            vec!["policy".into(), "admins".into()],
            vec!["prev".into(), policy_prev],
        ],
        "",
    )?;
    publish(&client, &base, &owner, &command).await?;
    publish(&client, &base, &owner, &command).await?;
    publish(&client, &base, &peer, &first).await?;
    let refused = revision(&peer, "community", community, d, Some(&update))?;
    assert!(
        !post(&client, &base, &peer, "/events", json!(refused))
            .await?
            .0
    );
    let wrong = revision(
        &peer,
        "community",
        &Uuid::new_v4().to_string(),
        Uuid::new_v4(),
        None,
    )?;
    assert!(
        !post(&client, &base, &peer, "/events", json!(wrong))
            .await?
            .0
    );
    for filter in [
        json!({"ids":[update.id.to_hex()]}),
        json!({"kinds":[45010],"search":"probe"}),
        json!({"kinds":[45010]}),
    ] {
        let (ok, body) = post(&client, &base, &peer, "/query", json!([filter])).await?;
        assert!(ok);
        let events = body.as_array().ok_or("generic events array")?;
        for event in events {
            assert_ne!(event["id"], update.id.to_hex());
            assert!(event["tags"]
                .as_array()
                .ok_or("event tags")?
                .iter()
                .all(|tag| tag[0] != "community"));
        }
    }
    assert!(
        !post(&client, &base, &peer, "/count", current.clone())
            .await?
            .0
    );
    let home = Uuid::new_v4();
    let create = sign(
        &owner,
        9007,
        vec![
            vec!["h".into(), home.to_string()],
            vec!["name".into(), format!("probe-{home}")],
        ],
        "",
    )?;
    publish(&client, &base, &owner, &create).await?;
    let channel = revision(&owner, "h", &home.to_string(), d, None)?;
    publish(&client, &base, &owner, &channel).await?;
    let (ok, body) = post(
        &client,
        &base,
        &peer,
        "/query",
        json!([{"artifact":"current","#d":[d]}]),
    )
    .await?;
    assert!(ok);
    assert_eq!(body[0]["id"], channel.id.to_hex());
    let redaction = sign(
        &owner,
        9005,
        vec![
            vec!["community".into(), community.into()],
            vec!["e".into(), update.id.to_hex()],
        ],
        "",
    )?;
    publish(&client, &base, &owner, &redaction).await?;
    let message = next(&mut ws).await?;
    assert_eq!(message[0], "CLOSED");
    assert!(message[2]
        .as_str()
        .ok_or("closed reason")?
        .starts_with("resync:"));
    publish(&client, &base, &peer, &update).await?;
    ws.close(None).await?;
    println!("PASS: signed HTTP and NIP-42 WS snapshot/live, policy CAS replay, wrong tenant, privacy, same-d channel collision, and redaction/resync");
    Ok(())
}
