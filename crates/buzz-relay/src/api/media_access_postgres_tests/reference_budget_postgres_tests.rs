use super::*;

pub(super) async fn run(
    state: &Arc<AppState>,
    tenant: &buzz_core::TenantContext,
    host: &str,
    owner: &Keys,
    member: &Keys,
    channel: Uuid,
) {
    let bytes = b"reference-budget-avatar-review21";
    let hash = hex::encode(Sha256::digest(bytes));
    let request = Request::builder()
        .method("PUT")
        .uri("/upload")
        .header(header::HOST, host)
        .header(header::AUTHORIZATION, proof(owner, host, &hash, "upload"))
        .header("X-SHA-256", &hash)
        .body(Body::from(bytes.as_slice()))
        .unwrap();
    let response = router(state.clone()).oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let descriptor: serde_json::Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 100_000).await.unwrap()).unwrap();
    let url = descriptor["url"].as_str().unwrap();
    let profile = EventBuilder::new(
        Kind::Metadata,
        serde_json::json!({"picture":url}).to_string(),
    )
    .sign_with_keys(owner)
    .unwrap();
    post(state, tenant, owner, profile.clone()).await.unwrap();

    // New artifacts must reject x-only imeta rather than accepting more decoys.
    let artifact = EventBuilder::new(Kind::Custom(45010), "budget fixture")
        .tags([
            Tag::parse(["ar", "1"]).unwrap(),
            Tag::parse(["d", &Uuid::new_v4().to_string()]).unwrap(),
            Tag::parse(["h", &channel.to_string()]).unwrap(),
            Tag::parse(["type", "buzz.task"]).unwrap(),
            Tag::parse(["op", "create"]).unwrap(),
            Tag::parse(["title", "Budget fixture"]).unwrap(),
            Tag::parse(["imeta", &format!("x {hash}")]).unwrap(),
        ])
        .sign_with_keys(owner)
        .unwrap();
    let Err(crate::handlers::ingest::IngestError::Rejected(message)) =
        post(state, tenant, owner, artifact).await
    else {
        panic!("expected invalid artifact imeta rejection");
    };
    assert!(message.contains("url"), "{message}");

    // Simulate historical artifact rows, including containment decoys where the
    // URL is on a separate tag. Neither is a same-tag media reference.
    sqlx::query("INSERT INTO events(community_id,id,pubkey,created_at,kind,tags,content,sig,channel_id) SELECT $1,decode(lpad(to_hex(n),64,'0'),'hex'),$2,'2025-01-01'::timestamptz,45010,$3,'decoy',decode(repeat('00',64),'hex'),$4 FROM generate_series(21000,22025) n")
        .bind(tenant.community().as_uuid()).bind(owner.public_key().as_bytes().as_slice())
        .bind(serde_json::json!([["imeta",format!("x {hash}")],["other",format!("url {url}")]]))
        .bind(channel).execute(state.db.pool()).await.unwrap();
    // Raw-content matches in malformed or large non-image fields must not
    // consume the byte budget before the real profile is examined.
    let padded = serde_json::json!({"about":format!("{hash}{}","a".repeat(256*1024)),"picture":null,"image":url}).to_string();
    let profiles: Vec<_> = (0..71)
        .map(|_| {
            let event = EventBuilder::new(Kind::Metadata, &padded)
                .custom_created_at(Timestamp::from(1_735_689_600))
                .sign_with_keys(&Keys::generate())
                .unwrap();
            serde_json::to_value(event).unwrap()
        })
        .collect();
    sqlx::query("INSERT INTO events(community_id,id,pubkey,created_at,kind,tags,content,sig) SELECT $1,decode(p->>'id','hex'),decode(p->>'pubkey','hex'),to_timestamp((p->>'created_at')::bigint),0,p->'tags',p->>'content',decode(p->>'sig','hex') FROM jsonb_array_elements($2) p")
        .bind(tenant.community().as_uuid()).bind(serde_json::Value::Array(profiles))
        .execute(state.db.pool()).await.unwrap();
    sqlx::query("INSERT INTO events(community_id,id,pubkey,created_at,kind,tags,content,sig) VALUES($1,decode(repeat('ab',32),'hex'),$2,now(),0,'[]',$3,decode(repeat('00',64),'hex'))")
        .bind(tenant.community().as_uuid()).bind(member.public_key().as_bytes().as_slice())
        .bind(format!("not JSON {hash}")).execute(state.db.pool()).await.unwrap();
    for keys in [owner, member] {
        for method in ["GET", "HEAD"] {
            let response = router(state.clone())
                .oneshot(get(keys, host, &hash, &hash, method, None))
                .await
                .unwrap();
            assert_eq!(
                response.status(),
                StatusCode::OK,
                "avatar remains readable beside >1024 x-only artifacts"
            );
        }
    }
    sqlx::query("UPDATE events SET deleted_at=now() WHERE community_id=$1 AND id=$2")
        .bind(tenant.community().as_uuid())
        .bind(profile.id.as_bytes().as_slice())
        .execute(state.db.pool())
        .await
        .unwrap();
}
