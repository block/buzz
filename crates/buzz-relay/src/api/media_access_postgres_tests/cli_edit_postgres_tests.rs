//! Exercise the real CLI, HTTP bridge, ingest, and media reads in enforce mode.
use super::*;

// Keep the large ingest future off the enclosing test thread's stack.
async fn publish(
    state: &Arc<AppState>,
    tenant: &buzz_core::TenantContext,
    keys: &Keys,
    event: nostr::Event,
) -> Result<crate::handlers::ingest::IngestResult, crate::handlers::ingest::IngestError> {
    Box::pin(post(state, tenant, keys, event)).await
}

async fn lookup(
    state: &Arc<AppState>,
    host: &str,
    keys: &Keys,
    channel: Uuid,
    target: nostr::EventId,
) -> (StatusCode, serde_json::Value) {
    let body = serde_json::to_vec(&serde_json::json!([{"kinds":[9,40002],"ids":[target.to_hex()],"#h":[channel.to_string()],"effective_edit":1,"consistency":"strong"}])).unwrap();
    let proof = EventBuilder::new(Kind::Custom(27235), "")
        .tags([
            Tag::parse(["u", &format!("http://{host}/query")]).unwrap(),
            Tag::parse(["method", "POST"]).unwrap(),
            Tag::parse(["payload", &hex::encode(Sha256::digest(&body))]).unwrap(),
            Tag::parse(["nonce", &Uuid::new_v4().to_string()]).unwrap(),
        ])
        .sign_with_keys(keys)
        .unwrap();
    let request = Request::builder()
        .method("POST")
        .uri("/query")
        .header(header::HOST, host)
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::AUTHORIZATION,
            format!(
                "Nostr {}",
                base64::engine::general_purpose::STANDARD.encode(proof.as_json())
            ),
        )
        .body(Body::from(body))
        .unwrap();
    let response = crate::router::build_router(state.clone())
        .oneshot(request)
        .await
        .unwrap();
    let status = response.status();
    (
        status,
        serde_json::from_slice(&to_bytes(response.into_body(), 1_000_000).await.unwrap()).unwrap(),
    )
}

async fn cli_edit(url: &str, keys: &Keys, id: nostr::EventId, clear: bool, content: &str) {
    let mut args = vec![
        "buzz".to_owned(),
        "--relay".into(),
        url.into(),
        "--private-key".into(),
        keys.secret_key().to_secret_hex(),
        "messages".into(),
        "edit".into(),
        "--event".into(),
        id.to_hex(),
        "--content".into(),
        content.into(),
    ];
    if clear {
        args.push("--clear-attachments".into());
    }
    assert_eq!(
        Box::pin(buzz_cli::run_from_args(args)).await,
        0,
        "CLI edit failed"
    );
}
async fn readable(
    state: &Arc<AppState>,
    host: &str,
    hashes: &[String],
    keys: &[&Keys],
    expected: &[StatusCode],
) {
    for (hash, status) in hashes.iter().zip(expected) {
        for key in keys {
            for method in ["GET", "HEAD"] {
                let response = router(state.clone())
                    .oneshot(get(key, host, hash, hash, method, None))
                    .await
                    .unwrap();
                assert_eq!(response.status(), *status, "{method} {hash}");
                assert_eq!(
                    response.headers()[header::CACHE_CONTROL],
                    "private, no-store"
                );
            }
        }
    }
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn cli_successive_edits_preserve_and_revoke_attachments() {
    let bucket = Bucket::default();
    let s3_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", s3_listener.local_addr().unwrap());
    let service = Router::new()
        .route("/{*key}", any(object))
        .with_state(bucket);
    let s3_server = tokio::spawn(async move {
        axum::serve(s3_listener, service).await.unwrap();
    });
    let state = fixture(
        &endpoint,
        super::super::media_access::MediaReadMode::Enforce,
    )
    .await;
    let http_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let host_string = http_listener.local_addr().unwrap().to_string();
    let host = host_string.as_str();
    state.db.ensure_configured_community(host).await.unwrap();
    let tenant = crate::tenant::bind_community(&state.db, host)
        .await
        .unwrap();
    let owner = Keys::generate();
    let member = Keys::generate();
    let outsider = Keys::generate();
    for key in [&owner, &member, &outsider] {
        state
            .db
            .ensure_user(tenant.community(), key.public_key().as_bytes())
            .await
            .unwrap();
    }
    let channel = state
        .db
        .create_channel(
            tenant.community(),
            "cli-edit",
            ChannelType::Stream,
            ChannelVisibility::Private,
            None,
            owner.public_key().as_bytes(),
            None,
        )
        .await
        .unwrap()
        .id;
    state
        .db
        .add_member(
            tenant.community(),
            channel,
            member.public_key().as_bytes(),
            MemberRole::Member,
            Some(owner.public_key().as_bytes()),
        )
        .await
        .unwrap();
    let mut media = Vec::new();
    let mut hashes = Vec::new();
    for bytes in [
        b"cli retained file".as_slice(),
        b"cli removed file".as_slice(),
    ] {
        let hash = hex::encode(Sha256::digest(bytes));
        let upload = Request::builder()
            .method("PUT")
            .uri("/upload")
            .header(header::HOST, host)
            .header(header::AUTHORIZATION, proof(&owner, host, &hash, "upload"))
            .header("X-SHA-256", &hash)
            .body(Body::from(bytes))
            .unwrap();
        let response = router(state.clone()).oneshot(upload).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let descriptor: serde_json::Value =
            serde_json::from_slice(&to_bytes(response.into_body(), 100_000).await.unwrap())
                .unwrap();
        media.push(vec![
            "imeta".into(),
            format!("url {}", descriptor["url"].as_str().unwrap()),
            format!("x {hash}"),
            "m application/octet-stream".into(),
            format!("size {}", bytes.len()),
            "alt original label".into(),
        ]);
        hashes.push(hash);
    }
    let original = EventBuilder::new(Kind::Custom(9), "two files")
        .tags(
            std::iter::once(Tag::parse(["h", &channel.to_string()]).unwrap())
                .chain(media.iter().map(|t| Tag::parse(t.clone()).unwrap())),
        )
        .custom_created_at(Timestamp::from(Timestamp::now().as_secs() - 20))
        .sign_with_keys(&owner)
        .unwrap();
    publish(&state, &tenant, &owner, original.clone())
        .await
        .unwrap();
    let url = format!("http://{host}");
    let http_app = crate::router::build_router(state.clone());
    let http_server = tokio::spawn(async move {
        axum::serve(http_listener, http_app).await.unwrap();
    });
    for index in 0..2 {
        cli_edit(
            &url,
            &owner,
            original.id,
            false,
            &format!("typo fix {index}"),
        )
        .await;
        readable(
            &state,
            host,
            &hashes,
            &[&owner, &member],
            &[StatusCode::OK; 2],
        )
        .await;
        readable(
            &state,
            host,
            &hashes,
            &[&outsider],
            &[StatusCode::NOT_FOUND; 2],
        )
        .await;
    }
    // Model a desktop/SDK edit removing just the second file. Keep its URL in
    // the body to prove text cannot grant access or revive removed imeta.
    let body = format!("retained one, removed URL {}", media[1][1]);
    let partial = buzz_sdk::build_edit_with_media(channel, original.id, &body, &media[..1])
        .unwrap()
        .custom_created_at(Timestamp::from(Timestamp::now().as_secs() + 5))
        .sign_with_keys(&owner)
        .unwrap();
    publish(&state, &tenant, &owner, partial.clone())
        .await
        .unwrap();
    for _ in 0..2 {
        cli_edit(&url, &owner, original.id, false, &body).await;
        readable(
            &state,
            host,
            &hashes,
            &[&owner, &member],
            &[StatusCode::OK, StatusCode::NOT_FOUND],
        )
        .await;
    }
    // A newer edit that restores both files is then deleted. Resolve the
    // latest live edit, not this deleted replacement or the original.
    let deleted =
        buzz_sdk::build_edit_with_media(channel, original.id, "deleted replacement", &media)
            .unwrap()
            .custom_created_at(Timestamp::from(Timestamp::now().as_secs() + 20))
            .sign_with_keys(&owner)
            .unwrap();
    // Seed a historical deleted replacement without publishing a new live
    // restoration. The uploader still has authority to republish these bytes.
    state
        .db
        .insert_event(tenant.community(), &deleted, Some(channel))
        .await
        .unwrap();
    state
        .db
        .soft_delete_event_and_update_thread(tenant.community(), deleted.id.as_bytes(), None, None)
        .await
        .unwrap();
    cli_edit(&url, &owner, original.id, false, &body).await;
    readable(
        &state,
        host,
        &hashes,
        &[&owner, &member],
        &[StatusCode::OK, StatusCode::NOT_FOUND],
    )
    .await;
    cli_edit(&url, &owner, original.id, true, &body).await;
    cli_edit(&url, &owner, original.id, false, &body).await;
    readable(
        &state,
        host,
        &hashes,
        &[&owner, &member],
        &[StatusCode::NOT_FOUND; 2],
    )
    .await;

    // Mixed-case removal must stay removed from this message even when a
    // second live reference would authorize republication of the same bytes.
    let second = EventBuilder::new(Kind::Custom(9), "another reference")
        .tags(
            std::iter::once(Tag::parse(["h", &channel.to_string()]).unwrap())
                .chain(media[..1].iter().map(|t| Tag::parse(t.clone()).unwrap())),
        )
        .sign_with_keys(&owner)
        .unwrap();
    publish(&state, &tenant, &owner, second.clone())
        .await
        .unwrap();
    let restored = buzz_sdk::build_edit_with_media(channel, original.id, "both", &media)
        .unwrap()
        .custom_created_at(Timestamp::from(Timestamp::now().as_secs() + 30))
        .sign_with_keys(&owner)
        .unwrap();
    publish(&state, &tenant, &owner, restored).await.unwrap();
    let mixed = original
        .id
        .to_hex()
        .chars()
        .enumerate()
        .map(|(i, c)| {
            if i % 2 == 0 {
                c.to_ascii_uppercase()
            } else {
                c
            }
        })
        .collect::<String>();
    let removal = EventBuilder::new(Kind::Custom(40003), "only second")
        .tags([
            Tag::parse(["h", &channel.to_string()]).unwrap(),
            Tag::parse(["e", &mixed]).unwrap(),
            Tag::parse(media[1].clone()).unwrap(),
        ])
        .custom_created_at(Timestamp::from(Timestamp::now().as_secs() + 31))
        .sign_with_keys(&owner)
        .unwrap();
    publish(&state, &tenant, &owner, removal).await.unwrap();
    cli_edit(&url, &owner, original.id, false, "after mixed-case removal").await;
    let current = state
        .db
        .message_edit_state(
            tenant.community(),
            channel,
            original.id.as_bytes(),
            owner.public_key().as_bytes(),
        )
        .await
        .unwrap()
        .unwrap()
        .edit
        .unwrap();
    let carried: Vec<_> = current
        .tags
        .iter()
        .filter(|t| t.as_slice()[0] == "imeta")
        .map(|t| t.as_slice().to_vec())
        .collect();
    assert_eq!(
        carried,
        vec![media[1].clone()],
        "must not resurrect the first file"
    );
    readable(
        &state,
        host,
        &hashes,
        &[&owner, &member],
        &[StatusCode::OK; 2],
    )
    .await;
    state
        .db
        .soft_delete_event_and_update_thread(tenant.community(), second.id.as_bytes(), None, None)
        .await
        .unwrap();
    readable(
        &state,
        host,
        &hashes,
        &[&owner, &member],
        &[StatusCode::NOT_FOUND, StatusCode::OK],
    )
    .await;

    // An agent owner outside this private channel has no attachment read or
    // publication authority. A direct retaining edit must also be rejected.
    state
        .db
        .set_agent_owner(
            tenant.community(),
            owner.public_key().as_bytes(),
            outsider.public_key().as_bytes(),
        )
        .await
        .unwrap();
    let owner_edit =
        buzz_sdk::build_edit_with_media(channel, original.id, "owner edit", &media[1..])
            .unwrap()
            .sign_with_keys(&outsider)
            .unwrap();
    let Err(error) = publish(&state, &tenant, &outsider, owner_edit).await else {
        panic!("owner edit was accepted");
    };
    assert!(
        matches!(error, crate::handlers::ingest::IngestError::Rejected(ref reason)
        if reason == "restricted: attachment is not accessible"),
        "{error:?}"
    );
    assert_ne!(
        Box::pin(buzz_cli::run_from_args(vec![
            "buzz".to_string(),
            "--relay".into(),
            url.clone(),
            "--private-key".into(),
            outsider.secret_key().to_secret_hex(),
            "messages".into(),
            "edit".into(),
            "--event".into(),
            original.id.to_hex(),
            "--content".into(),
            "owner edit".into()
        ]))
        .await,
        0
    );

    // Knowing a private hash never lets another member attach it to their own
    // message/edit. This exercises the common publication check on kind 40003.
    let open = state
        .db
        .create_channel(
            tenant.community(),
            "attacker",
            ChannelType::Stream,
            ChannelVisibility::Open,
            None,
            outsider.public_key().as_bytes(),
            None,
        )
        .await
        .unwrap()
        .id;
    let harmless = EventBuilder::new(Kind::Custom(9), "no files")
        .tags([Tag::parse(["h", &open.to_string()]).unwrap()])
        .sign_with_keys(&outsider)
        .unwrap();
    publish(&state, &tenant, &outsider, harmless.clone())
        .await
        .unwrap();
    let copied = buzz_sdk::build_edit_with_media(open, harmless.id, "stolen", &media)
        .unwrap()
        .sign_with_keys(&outsider)
        .unwrap();
    assert!(publish(&state, &tenant, &outsider, copied).await.is_err());
    readable(
        &state,
        host,
        &hashes,
        &[&outsider],
        &[StatusCode::NOT_FOUND; 2],
    )
    .await;
    let forbidden = lookup(&state, host, &outsider, channel, original.id).await;
    let missing = lookup(
        &state,
        host,
        &outsider,
        channel,
        nostr::EventId::from_byte_array([0x42; 32]),
    )
    .await;
    assert_eq!(
        forbidden, missing,
        "inaccessible and missing states must be indistinguishable"
    );
    assert_eq!(forbidden.0, StatusCode::NOT_FOUND);
    state
        .db
        .remove_member(
            tenant.community(),
            channel,
            member.public_key().as_bytes(),
            owner.public_key().as_bytes(),
        )
        .await
        .unwrap();
    assert_eq!(
        lookup(&state, host, &member, channel, original.id).await,
        missing,
        "fresh snapshot must reject a removed member"
    );
    assert_eq!(
        lookup(&state, host, &member, open, harmless.id).await.0,
        StatusCode::OK,
        "open-channel reader needs no membership"
    );
    http_server.abort();
    s3_server.abort();
}
