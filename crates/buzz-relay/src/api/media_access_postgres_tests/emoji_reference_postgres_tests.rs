use super::*;

// Extend the enforce-mode HTTP journey, including replacement of a shortcode.
pub(super) async fn run(
    state: &Arc<AppState>,
    tenant: &buzz_core::TenantContext,
    host: &str,
    owner: &Keys,
    member: &Keys,
) {
    let mut hashes = std::collections::HashSet::new();
    for kind in [30180, 30030, 10030] {
        let mut previous: Option<String> = None;
        for revision in 0..2 {
            // Unique payloads cannot borrow another journey step's reference.
            let bytes =
                format!("emoji-media-authorization-{kind}-revision-{revision}").into_bytes();
            let hash = hex::encode(Sha256::digest(&bytes));
            assert!(
                hashes.insert(hash.clone()),
                "emoji fixtures must have distinct hashes"
            );
            let request = Request::builder()
                .method("PUT")
                .uri("/upload")
                .header(header::HOST, host)
                .header(header::AUTHORIZATION, proof(owner, host, &hash, "upload"))
                .header("X-SHA-256", &hash)
                .body(Body::from(bytes))
                .unwrap();
            let response = router(state.clone()).oneshot(request).await.unwrap();
            assert_eq!(response.status(), StatusCode::OK, "upload kind {kind}");
            let body = to_bytes(response.into_body(), 100_000).await.unwrap();
            let descriptor: serde_json::Value = serde_json::from_slice(&body).unwrap();
            let url = descriptor["url"].as_str().unwrap();
            // The asset is fresh: only the uploader can preview it.
            for (keys, expected) in [(owner, StatusCode::OK), (member, StatusCode::NOT_FOUND)] {
                check(state, host, keys, &hash, expected).await;
            }
            let builder = if kind == 30180 {
                buzz_sdk::build_emoji_entry("media_test", url).unwrap()
            } else {
                EventBuilder::new(Kind::Custom(kind), "").tags([
                    Tag::parse(["d", "media_test"]).unwrap(),
                    Tag::parse(["emoji", "media_test", url]).unwrap(),
                ])
            };
            let event = builder
                .custom_created_at(Timestamp::from(
                    Timestamp::now().as_secs() + revision as u64,
                ))
                .sign_with_keys(owner)
                .unwrap();
            post(state, tenant, owner, event).await.unwrap();
            for keys in [owner, member] {
                check(state, host, keys, &hash, StatusCode::OK).await;
                if let Some(old_hash) = &previous {
                    // Replacing an entry/set retires the old reference and cannot
                    // revive uploader preview after publication consumed it.
                    check(state, host, keys, old_hash, StatusCode::NOT_FOUND).await;
                }
            }
            previous = Some(hash);
        }
    }
    assert_eq!(hashes.len(), 6);
}

async fn check(state: &Arc<AppState>, host: &str, keys: &Keys, hash: &str, status: StatusCode) {
    for method in ["GET", "HEAD"] {
        let response = router(state.clone())
            .oneshot(get(keys, host, hash, hash, method, None))
            .await
            .unwrap();
        assert_eq!(response.status(), status, "emoji {method} {hash}");
    }
}
