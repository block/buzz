use super::*;

async fn ingest(f: &Fixture, event: &Event) {
    let (status, body) = f
        .post(&f.keys, "/events", serde_json::to_value(event).unwrap())
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["accepted"], true, "{body}");
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn thread_window_signed_aux_requires_positional_target() {
    let f = Fixture::new().await;
    let other = event(
        &f.keys,
        f.channel,
        9,
        "other root",
        None,
        f.root.created_at.as_secs(),
    );
    ingest(&f, &other).await;
    let a = f.root.id.to_hex();
    let b = other.id.to_hex();
    let mut edits = Vec::new();
    for (n, tags) in [
        vec![vec!["e", b.as_str(), a.as_str()]],
        vec![vec!["e", b.as_str()], vec!["x", "e", a.as_str()]],
        vec![vec!["e", a.as_str()]],
        vec![vec!["e", b.as_str()], vec!["e", a.as_str()]],
    ]
    .into_iter()
    .enumerate()
    {
        let edit = EventBuilder::new(Kind::Custom(40003), format!("edit {n}"))
            .tags(
                std::iter::once(Tag::parse(["h", &f.channel.to_string()]).unwrap())
                    .chain(tags.into_iter().map(|tag| Tag::parse(tag).unwrap())),
            )
            .sign_with_keys(&f.keys)
            .unwrap();
        ingest(&f, &edit).await;
        edits.push(edit);
    }
    let filter = f.filter();
    let page = f.query(&filter).await;
    assert_eq!(f.bounds(&page, &filter)["has_more"], false);
    let aux = ids(&page, Some(40003));
    for (n, edit) in edits.iter().enumerate() {
        assert_eq!(aux.contains(&edit.id.to_hex().as_str()), n >= 2, "case {n}");
    }
    // Both non-target shapes really were stored and remain reachable under B.
    let mut other_filter = filter;
    other_filter["#e"] = json!([b]);
    assert_eq!(ids(&f.query(&other_filter).await, Some(40003)).len(), 3);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn thread_window_unsupported_signed_root_has_no_false_bounds() {
    let f = Fixture::new().await;
    let root = EventBuilder::new(Kind::Custom(40008), "diff --git a/file b/file")
        .tags([
            Tag::parse(["h", &f.channel.to_string()]).unwrap(),
            Tag::parse(["repo", "https://github.com/block/buzz"]).unwrap(),
            Tag::parse(["commit", "1234567"]).unwrap(),
        ])
        .sign_with_keys(&f.keys)
        .unwrap();
    ingest(&f, &root).await;
    let reply = event(
        &f.keys,
        f.channel,
        9,
        "reply to diff",
        Some(&root),
        root.created_at.as_secs(),
    );
    ingest(&f, &reply).await;
    let legacy = json!({"#h":[f.channel],"#e":[root.id.to_hex()],"kinds":[9],"depth_limit":100});
    assert_eq!(ids(&f.query(&legacy).await, Some(9)), [reply.id.to_hex()]);
    let mut filter = f.filter();
    filter["#e"] = json!([root.id.to_hex()]);
    for include_aux in [false, true] {
        filter["include_aux"] = json!(include_aux);
        assert_eq!(
            f.query(&filter).await,
            json!([]),
            "unsupported is not exhausted"
        );
    }
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn thread_window_signed_large_aux_fails_with_recoverable_budget_error() {
    let f = Fixture::new().await;
    // No forged bulk copies: prove these maximum-sized payloads pass ingest.
    for n in 0..40 {
        let edit = event(
            &f.keys,
            f.channel,
            40003,
            &"x".repeat(256 * 1024),
            Some(&f.root),
            f.root.created_at.as_secs() + n,
        );
        ingest(&f, &edit).await;
    }
    let filter = f.filter();
    let (status, body) = f.post(&f.keys, "/query", json!([filter])).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
    assert!(body["error"]
        .as_str()
        .unwrap()
        .contains("auxiliary byte budget"));
    let mut without_aux = filter;
    without_aux["include_aux"] = json!(false);
    let recovered = f.query(&without_aux).await;
    assert_eq!(f.bounds(&recovered, &without_aux)["has_more"], false);
}
