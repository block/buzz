//! Local-only reproduction of durable revalidation against a running relay.
//! Requires a disposable database, NIP-FI off, relay membership enabled, and
//! NIP-OA enabled. It writes durable bans directly, without Redis publication.
//! Run with DATABASE_URL and RELAY_URL pointing to that isolated stack.
use futures_util::{SinkExt, StreamExt};
use nostr::{EventBuilder, Keys, Kind, Tag};
use serde_json::{json, Value};
use sqlx::PgPool;
use std::time::{Duration, Instant};
use tokio::net::TcpStream;
use tokio_tungstenite::{
    connect_async,
    tungstenite::{client::IntoClientRequest, Message},
    MaybeTlsStream, WebSocketStream,
};
use uuid::Uuid;

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

async fn frame(socket: &mut Socket) -> Message {
    tokio::time::timeout(Duration::from_secs(30), socket.next())
        .await
        .expect("bounded relay frame")
        .expect("socket remains open")
        .expect("frame")
}

async fn open(
    base: &str,
    host: &str,
    channel: Option<Uuid>,
    key: &Keys,
    owner: Option<&Keys>,
) -> Socket {
    let url = channel.map_or_else(|| base.to_owned(), |id| format!("{base}/huddle/{id}/audio"));
    let mut request = url.into_client_request().unwrap();
    request.headers_mut().insert("host", host.parse().unwrap());
    let (mut socket, _) = connect_async(request).await.expect("real relay connection");
    let mut challenge = None;
    for _ in 0..32 {
        if let Message::Text(text) = frame(&mut socket).await {
            let value: Value = serde_json::from_str(&text).unwrap();
            if channel.is_some() && value["type"] == "challenge" {
                challenge = Some(value["challenge"].as_str().unwrap().to_owned());
                break;
            }
            if channel.is_none() && value[0] == "AUTH" {
                challenge = Some(value[1].as_str().unwrap().to_owned());
                break;
            }
        }
    }
    let challenge = challenge.expect("bounded authentication challenge");
    let mut event = EventBuilder::new(Kind::Authentication, "")
        .tag(Tag::parse(["relay", &format!("ws://{host}")]).unwrap())
        .tag(Tag::parse(["challenge", &challenge]).unwrap());
    if let Some(owner) = owner {
        let tag = buzz_sdk::nip_oa::compute_auth_tag(owner, &key.public_key(), "").unwrap();
        let tag: Vec<String> = serde_json::from_str(&tag).unwrap();
        event = event.tag(Tag::parse(tag).unwrap());
    }
    let event = event.sign_with_keys(key).unwrap();
    let auth = if channel.is_some() {
        json!({"type":"auth","event":event})
    } else {
        json!(["AUTH", event])
    };
    socket
        .send(Message::Text(auth.to_string().into()))
        .await
        .unwrap();
    for _ in 0..32 {
        if let Message::Text(text) = frame(&mut socket).await {
            let value: Value = serde_json::from_str(&text).unwrap();
            if channel.is_some() {
                assert_ne!(value["type"], "error", "{value}");
                if value["type"] == "joined" {
                    return socket;
                }
            } else if value[0] == "OK" {
                assert_eq!(value[2], true, "{value}");
                return socket;
            }
        }
    }
    panic!("no admission acknowledgement");
}

async fn closed(mut socket: Socket, since: Instant, label: &str) -> Duration {
    for _ in 0..64 {
        if let Message::Close(Some(close)) = frame(&mut socket).await {
            assert_eq!(u16::from(close.code), 1008, "{label}: {close:?}");
            let elapsed = since.elapsed();
            assert!(elapsed < Duration::from_secs(30), "{label}: {elapsed:?}");
            println!(
                "{label}: policy close in {} ms ({})",
                elapsed.as_millis(),
                close.reason
            );
            return elapsed;
        }
    }
    panic!("{label}: no policy close");
}

#[tokio::test]
#[ignore = "external infrastructure: isolated running relay and PostgreSQL"]
async fn external_infra_durable_ban_and_database_failure_close_real_root_and_audio() {
    let db = PgPool::connect(&std::env::var("DATABASE_URL").expect("disposable DATABASE_URL"))
        .await
        .unwrap();
    let base = std::env::var("RELAY_URL").expect("isolated RELAY_URL");
    assert!(base.starts_with("ws://127.0.0.1:"), "local relay only");
    let community = Uuid::new_v4();
    let host = format!(
        "revalidation-{}.test:{}",
        community.simple(),
        url::Url::parse(&base).unwrap().port().unwrap()
    );
    let channel = Uuid::new_v4();
    let (member, owner, agent, bystander) = (
        Keys::generate(),
        Keys::generate(),
        Keys::generate(),
        Keys::generate(),
    );
    sqlx::query("INSERT INTO communities(id,host) VALUES($1,$2)")
        .bind(community)
        .bind(&host)
        .execute(&db)
        .await
        .unwrap();
    sqlx::query("INSERT INTO channels(community_id,id,name,channel_type,visibility,created_by) VALUES($1,$2,'local-revalidation','stream','open',$3)")
        .bind(community).bind(channel).bind(bystander.public_key().to_bytes().to_vec()).execute(&db).await.unwrap();
    for key in [&member, &owner, &agent, &bystander] {
        sqlx::query("INSERT INTO users(community_id,pubkey) VALUES($1,$2)")
            .bind(community)
            .bind(key.public_key().to_bytes().to_vec())
            .execute(&db)
            .await
            .unwrap();
        sqlx::query("INSERT INTO relay_members(community_id,pubkey,role) VALUES($1,$2,'member')")
            .bind(community)
            .bind(key.public_key().to_hex())
            .execute(&db)
            .await
            .unwrap();
        sqlx::query("INSERT INTO channel_members(community_id,channel_id,pubkey,role) VALUES($1,$2,$3,'member')").bind(community).bind(channel).bind(key.public_key().to_bytes().to_vec()).execute(&db).await.unwrap();
    }
    let root_member = open(&base, &host, None, &member, None).await;
    let root_agent = open(&base, &host, None, &agent, Some(&owner)).await;
    let audio_member = open(&base, &host, Some(channel), &member, None).await;
    let audio_agent = open(&base, &host, Some(channel), &agent, Some(&owner)).await;
    let mut control = open(&base, &host, None, &bystander, None).await;
    // No moderation event or pub/sub operation: only authoritative DB commits.
    let since = Instant::now();
    for key in [&member, &owner] {
        sqlx::query("INSERT INTO community_bans(community_id,pubkey,banned,actor_pubkey) VALUES($1,$2,true,$3)")
            .bind(community).bind(key.public_key().to_bytes().to_vec()).bind(bystander.public_key().to_bytes().to_vec()).execute(&db).await.unwrap();
    }
    tokio::join!(
        closed(root_member, since, "root member"),
        closed(root_agent, since, "root owner-linked agent"),
        closed(audio_member, since, "audio member"),
        closed(audio_agent, since, "audio owner-linked agent")
    );
    control
        .send(Message::Ping(vec![1, 2, 3].into()))
        .await
        .unwrap();
    let mut pong = false;
    for _ in 0..32 {
        if let Message::Pong(data) = frame(&mut control).await {
            assert_eq!(data.as_ref(), [1, 2, 3]);
            pong = true;
            break;
        }
    }
    assert!(pong, "clear bystander responds within the frame bound");
    println!("clear bystander remains responsive after durable bans");
    let audio_control = open(&base, &host, Some(channel), &bystander, None).await;
    // This test owns a disposable database. Restore the schema even on failure.
    sqlx::query("ALTER TABLE community_bans RENAME TO live_unavailable_bans")
        .execute(&db)
        .await
        .unwrap();
    let since = Instant::now();
    let closes = tokio::spawn(async move {
        tokio::join!(
            closed(control, since, "root database failure"),
            closed(audio_control, since, "audio database failure")
        )
    });
    let outcome = closes.await;
    sqlx::query("ALTER TABLE live_unavailable_bans RENAME TO community_bans")
        .execute(&db)
        .await
        .unwrap();
    outcome.expect("database failures close within the bound");
}
