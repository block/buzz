use super::*;
use base64::Engine;
use sha2::Digest;

#[tokio::test]
#[ignore = "requires Postgres"]
async fn sync_downloads_ten_thousand_tasks_with_sixty_thousand_memberships() {
    let _guard = SERIAL.lock().await;
    let f = fixture().await;
    let homes = (0..60_001)
        .map(|n| if n == 0 { f.home } else { Uuid::new_v4() })
        .collect::<Vec<_>>();
    for chunk in homes[1..].chunks(500) {
        sqlx::query("INSERT INTO channels(community_id,id,name,visibility,channel_type,created_by,archived_at) SELECT $1,id,id::text,'private','stream',$2,'2000-01-01' FROM unnest($3::uuid[]) id")
            .bind(f.community.as_uuid()).bind(f.owner.public_key().as_bytes().as_slice()).bind(chunk).execute(&f.pool).await.unwrap();
        sqlx::query("INSERT INTO channel_members(community_id,channel_id,pubkey,role,sync_modified_at) SELECT $1,id,$2,'owner','2000-01-01' FROM unnest($3::uuid[]) id")
            .bind(f.community.as_uuid()).bind(f.owner.public_key().as_bytes().as_slice()).bind(chunk).execute(&f.pool).await.unwrap();
    }
    sqlx::query("UPDATE channel_members SET sync_modified_at='2000-01-01' WHERE community_id=$1 AND channel_id=ANY($2)")
        .bind(f.community.as_uuid()).bind(vec![f.home,f.private]).execute(&f.pool).await.unwrap();
    sqlx::query("UPDATE channels SET archived_at=NULL WHERE community_id=$1 AND id=ANY($2)")
        .bind(f.community.as_uuid())
        .bind(&homes[..10_000])
        .execute(&f.pool)
        .await
        .unwrap();
    let mut event_bytes = 0;
    for chunk in homes[..10_050].chunks(500) {
        let values=chunk.iter().map(|home| {
            let id=Uuid::new_v4();
            let base=f.revision_as(&f.owner,*home,id,"create",None);
            let event=EventBuilder::new(Kind::Custom(45010),json!({"notes":"x".repeat(4400)}).to_string())
                .tags(base.tags.clone()).sign_with_keys(&f.owner).unwrap();
            event_bytes=serde_json::to_vec(&event).unwrap().len();
            json!({"id":event.id.to_hex(),"artifact_id":id,"tags":event.tags,"content":event.content,"sig":event.sig.to_string(),"created_at":event.created_at.as_secs(),"channel_id":home})
        }).collect::<Vec<_>>();
        sqlx::query("INSERT INTO events(community_id,id,pubkey,created_at,kind,tags,content,sig,channel_id) SELECT $1,decode(x.id,'hex'),$2,to_timestamp(x.created_at),45010,x.tags,x.content,decode(x.sig,'hex'),x.channel_id FROM jsonb_to_recordset($3) x(id text,tags jsonb,content text,sig text,created_at bigint,channel_id uuid)")
            .bind(f.community.as_uuid()).bind(f.owner.public_key().as_bytes().as_slice()).bind(json!(values)).execute(&f.pool).await.unwrap();
        sqlx::query("INSERT INTO artifact_heads(community_id,artifact_id,event_id,channel_id,artifact_type,modified_at) SELECT $1,x.artifact_id,decode(x.id,'hex'),x.channel_id,'buzz.task','2000-01-01' FROM jsonb_to_recordset($2) x(id text,artifact_id uuid,channel_id uuid)")
            .bind(f.community.as_uuid()).bind(json!(values)).execute(&f.pool).await.unwrap();
    }
    sqlx::raw_sql("ANALYZE artifact_heads; ANALYZE channel_members; ANALYZE channels")
        .execute(&f.pool)
        .await
        .unwrap();
    let mut other = (*bridge_handler_test_state().await.unwrap()).clone();
    let config = Arc::make_mut(&mut other.config);
    config.require_auth_token = true;
    config.artifact_incremental_enabled = true;
    other.relay_keypair = f.state.relay_keypair.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/query", listener.local_addr().unwrap());
    let router = crate::router::build_router(Arc::new(other));
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .unwrap();
    let started = std::time::Instant::now();
    let mut page = sync(&f, None).await;
    let mut ids = std::collections::HashSet::new();
    let mut pages = 0;
    loop {
        pages += 1;
        for change in page["changes"].as_array().unwrap() {
            assert_eq!(change["type"], "upsert");
            ids.insert(change["artifactId"].as_str().unwrap().to_owned());
        }
        if page["nextCursor"].is_null() {
            break;
        }
        let (status, next) = signed_query(
            &f,
            &client,
            &url,
            json!([{"artifact":"incremental","query":query(),"cursor":page["nextCursor"]}]),
        )
        .await;
        assert!(status.is_success(), "{status}: {next}");
        page = next;
    }
    let cold = started.elapsed();
    assert_eq!(ids.len(), 10_050);
    assert_eq!(pages, 101);
    let warm = std::time::Instant::now();
    let (status, delta) = signed_query(
        &f,
        &client,
        &url,
        json!([{"artifact":"incremental","query":query(),"checkpoint":page["checkpoint"]}]),
    )
    .await;
    assert!(status.is_success(), "{status}: {delta}");
    assert_eq!(delta["changes"], json!([]));
    assert!(delta["checkpoint"].is_string());
    eprintln!("10,050 tasks; 60,002 memberships; {event_bytes}-byte events; {pages} cold pages: {cold:?}; empty warm sync: {:?}",warm.elapsed());
    server.abort();
}

async fn signed_query(
    f: &Fixture,
    client: &reqwest::Client,
    url: &str,
    body: Value,
) -> (StatusCode, Value) {
    let bytes = serde_json::to_vec(&body).unwrap();
    let auth = EventBuilder::new(Kind::Custom(27235), "")
        .tags([
            Tag::parse(["u", &format!("https://{}/query", f.host)]).unwrap(),
            Tag::parse(["method", "POST"]).unwrap(),
            Tag::parse(["payload", &hex::encode(sha2::Sha256::digest(&bytes))]).unwrap(),
            Tag::parse(["nonce", &Uuid::new_v4().to_string()]).unwrap(),
        ])
        .sign_with_keys(&f.owner)
        .unwrap();
    let authorization = format!(
        "Nostr {}",
        base64::engine::general_purpose::STANDARD.encode(serde_json::to_vec(&auth).unwrap())
    );
    let response = client
        .post(url)
        .header("host", &f.host)
        .header("content-type", "application/json")
        .header("authorization", authorization)
        .body(bytes)
        .send()
        .await
        .unwrap();
    let status = StatusCode::from_u16(response.status().as_u16()).unwrap();
    let body = serde_json::from_slice(&response.bytes().await.unwrap()).unwrap();
    (status, body)
}
