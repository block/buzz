//! Exercise the real CLI, HTTP bridge, ingest, and media reads in enforce mode.
use super::*;

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
    assert_eq!(buzz_cli::run_from_args(args).await, 0, "CLI edit failed");
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
    // Each PostgreSQL test has its own database, so this host is test-local.
    let host = "127.0.0.1";
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
    post(&state, &tenant, &owner, original.clone())
        .await
        .unwrap();
    let http_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", http_listener.local_addr().unwrap());
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
    post(&state, &tenant, &owner, partial.clone())
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
    // The removed file no longer has publication authority: reupload proof
    // is needed to publish a restoration, so insert this historical event
    // through the database test seam rather than granting new upload proof.
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
    post(&state, &tenant, &outsider, harmless.clone())
        .await
        .unwrap();
    let copied = buzz_sdk::build_edit_with_media(open, harmless.id, "stolen", &media)
        .unwrap()
        .sign_with_keys(&outsider)
        .unwrap();
    assert!(post(&state, &tenant, &outsider, copied).await.is_err());
    readable(
        &state,
        host,
        &hashes,
        &[&outsider],
        &[StatusCode::NOT_FOUND; 2],
    )
    .await;
    http_server.abort();
    s3_server.abort();
}
