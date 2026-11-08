use super::*;

// Exercise the client wire formats through real upload, ingest and download paths.
pub(super) async fn run(
    state: &Arc<AppState>,
    tenant: &buzz_core::TenantContext,
    host: &str,
    owner: &Keys,
    member: &Keys,
    survivor: &Keys,
    channel: Uuid,
) {
    let images: [&[u8]; 2] = [
        include_bytes!("../../../../buzz-media/tests/fixtures/ios/uikit-sanitized.png"),
        include_bytes!(
            "../../../../buzz-media/tests/fixtures/android/sanitized/bitmap-srgb-sanitized.png"
        ),
    ];
    for (index, format) in ["snapshot", "animated", "image"].into_iter().enumerate() {
        let mut assets = Vec::new();
        for bytes in images {
            let hash = hex::encode(Sha256::digest(bytes));
            let request = Request::builder()
                .method("PUT")
                .uri("/upload")
                .header(header::HOST, host)
                .header(header::AUTHORIZATION, proof(owner, host, &hash, "upload"))
                .header("X-SHA-256", &hash)
                .body(Body::from(bytes))
                .unwrap();
            let response = router(state.clone()).oneshot(request).await.unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            let body = to_bytes(response.into_body(), 100_000).await.unwrap();
            let descriptor: serde_json::Value = serde_json::from_slice(&body).unwrap();
            assets.push((hash, descriptor["url"].as_str().unwrap().to_owned()));
        }
        let [(image_hash, image), (favicon_hash, favicon)] = assets.as_slice() else {
            unreachable!()
        };
        let (kind,content,tags) = match format {
            "snapshot" => (Kind::Custom(9),"https://example.com/page".to_owned(),vec![
                Tag::parse(["h",&channel.to_string()]).unwrap(),
                Tag::parse(["link-preview","snapshot","1","https://example.com/page","title","site","description",image,image_hash,favicon,favicon_hash]).unwrap(),
            ]),
            "animated" => (Kind::Metadata,serde_json::json!({"picture":format!("{image}#buzz-anim={}",url::form_urlencoded::byte_serialize(favicon.as_bytes()).collect::<String>())}).to_string(),vec![]),
            _ => (Kind::Metadata,serde_json::json!({"image":image,"banner":favicon}).to_string(),vec![]),
        };
        // Owning an account or knowing a descriptor never substitutes for bytes.
        let copied = EventBuilder::new(kind, &content)
            .tags(tags.clone())
            .sign_with_keys(member)
            .unwrap();
        assert!(
            crate::handlers::media_access::validate_media_references(tenant, state, &copied)
                .await
                .is_err(),
            "copied {format}"
        );
        if format == "animated" {
            let request = Request::builder()
                .method("PUT")
                .uri("/upload")
                .header(header::HOST, host)
                .header(
                    header::AUTHORIZATION,
                    proof(member, host, image_hash, "upload"),
                )
                .header("X-SHA-256", image_hash)
                .body(Body::from(images[0]))
                .unwrap();
            assert_eq!(
                router(state.clone())
                    .oneshot(request)
                    .await
                    .unwrap()
                    .status(),
                StatusCode::OK
            );
            assert!(
                crate::handlers::media_access::validate_media_references(tenant, state, &copied)
                    .await
                    .is_err(),
                "owning only the poster cannot publish the private animation"
            );
        }
        let event = EventBuilder::new(kind, content)
            .tags(tags)
            .custom_created_at(Timestamp::from(
                Timestamp::now().as_secs() + 30 + index as u64,
            ))
            .sign_with_keys(owner)
            .unwrap();
        post(state, tenant, owner, event.clone()).await.unwrap();
        for (hash, _) in &assets {
            for keys in [owner, survivor] {
                let response = router(state.clone())
                    .oneshot(get(keys, host, hash, hash, "GET", None))
                    .await
                    .unwrap();
                assert_eq!(response.status(), StatusCode::OK, "posted {format}");
            }
            let response = router(state.clone())
                .oneshot(get(member, host, hash, hash, "GET", None))
                .await
                .unwrap();
            assert_eq!(
                response.status(),
                if format == "snapshot" {
                    StatusCode::NOT_FOUND
                } else {
                    StatusCode::OK
                }
            );
        }
        if format == "snapshot" {
            for (revision, suppress) in [(1, false), (2, true), (3, false)] {
                let mut tags = vec![
                    Tag::parse(["h", &channel.to_string()]).unwrap(),
                    Tag::parse(["e", &event.id.to_hex()]).unwrap(),
                ];
                if suppress {
                    tags.push(Tag::parse(["link-preview", "none"]).unwrap());
                }
                let edit =
                    EventBuilder::new(Kind::Custom(40003), "edited https://example.com/page")
                        .tags(tags)
                        .custom_created_at(Timestamp::from(event.created_at.as_secs() + revision))
                        .sign_with_keys(owner)
                        .unwrap();
                post(state, tenant, owner, edit).await.unwrap();
                for (hash, _) in &assets {
                    for keys in [owner, survivor, member] {
                        for method in ["GET", "HEAD"] {
                            let response = router(state.clone())
                                .oneshot(get(keys, host, hash, hash, method, None))
                                .await
                                .unwrap();
                            assert_eq!(
                                response.status(),
                                if revision >= 2 || keys.public_key() == member.public_key() {
                                    StatusCode::NOT_FOUND
                                } else {
                                    StatusCode::OK
                                },
                                "latest edit preview suppression={suppress}"
                            );
                        }
                    }
                }
            }
        }
        sqlx::query("UPDATE events SET deleted_at=now() WHERE community_id=$1 AND id=$2")
            .bind(tenant.community().as_uuid())
            .bind(event.id.as_bytes().as_slice())
            .execute(state.db.pool())
            .await
            .unwrap();
        for (hash, _) in &assets {
            let response = router(state.clone())
                .oneshot(get(owner, host, hash, hash, "GET", None))
                .await
                .unwrap();
            assert_eq!(
                response.status(),
                StatusCode::NOT_FOUND,
                "deleting {format} must not revive uploader preview"
            );
        }
    }
    // Persona definitions are readable only by their owner until shared. Each
    // gets fresh bytes so another profile cannot hide a missing reference.
    for shared in [false, true] {
        let bytes = format!("persona-media-review21-shared-{shared}");
        let hash = hex::encode(Sha256::digest(bytes.as_bytes()));
        let request = Request::builder()
            .method("PUT")
            .uri("/upload")
            .header(header::HOST, host)
            .header(header::AUTHORIZATION, proof(owner, host, &hash, "upload"))
            .header("X-SHA-256", &hash)
            .body(Body::from(bytes))
            .unwrap();
        let response = router(state.clone()).oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let descriptor: serde_json::Value =
            serde_json::from_slice(&to_bytes(response.into_body(), 100_000).await.unwrap())
                .unwrap();
        let mut tags = vec![Tag::parse([
            "d",
            if shared {
                "shared-media"
            } else {
                "private-media"
            },
        ])
        .unwrap()];
        if shared {
            tags.push(Tag::parse(["shared", "true"]).unwrap());
        }
        let persona = EventBuilder::new(
            Kind::Custom(30175),
            serde_json::json!({"name":"Media persona","avatar_url":descriptor["url"]}).to_string(),
        )
        .tags(tags)
        .sign_with_keys(owner)
        .unwrap();
        post(state, tenant, owner, persona).await.unwrap();
        for keys in [owner, member] {
            let report = EventBuilder::new(Kind::Custom(1984), "media report")
                .allow_self_tagging()
                .tags([
                    Tag::parse(["p", &owner.public_key().to_hex()]).unwrap(),
                    Tag::parse(["x", &hash, "spam"]).unwrap(),
                ])
                .sign_with_keys(keys)
                .unwrap();
            let result = post(state, tenant, keys, report).await;
            if shared || keys.public_key() == owner.public_key() {
                if let Err(error) = result {
                    panic!("readable persona blob report shared={shared}: {error:?}");
                }
            } else {
                assert!(
                    matches!(result,Err(crate::handlers::ingest::IngestError::Rejected(message)) if message.contains("report target blob not found"))
                );
                let absent = EventBuilder::new(Kind::Custom(1984), "missing report")
                    .tags([
                        Tag::parse(["p", &owner.public_key().to_hex()]).unwrap(),
                        Tag::parse(["x", &"9".repeat(64), "spam"]).unwrap(),
                    ])
                    .sign_with_keys(keys)
                    .unwrap();
                assert!(
                    matches!(post(state,tenant,keys,absent).await,Err(crate::handlers::ingest::IngestError::Rejected(message)) if message.contains("report target blob not found"))
                );
            }
            for method in ["GET", "HEAD"] {
                let response = router(state.clone())
                    .oneshot(get(keys, host, &hash, &hash, method, None))
                    .await
                    .unwrap();
                assert_eq!(
                    response.status(),
                    if shared || keys.public_key() == owner.public_key() {
                        StatusCode::OK
                    } else {
                        StatusCode::NOT_FOUND
                    },
                    "persona shared={shared}"
                );
            }
        }
    }
}
