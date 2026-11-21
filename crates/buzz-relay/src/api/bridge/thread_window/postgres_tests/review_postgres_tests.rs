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
async fn thread_window_signed_reaction_removal_and_readd() {
    let f = Fixture::new().await;
    let ts = f.root.created_at.as_secs();
    let reaction = |offset| {
        buzz_sdk::build_reaction(f.root.id, "👍")
            .unwrap()
            .custom_created_at(Timestamp::from(ts + offset))
            .sign_with_keys(&f.keys)
            .unwrap()
    };
    let original = reaction(1);
    let replacement = reaction(3);
    let filter = f.filter();

    ingest(&f, &original).await;
    let page = f.query(&filter).await;
    assert_eq!(ids(&page, Some(7)), [original.id.to_hex()]);

    let deletion = buzz_sdk::build_remove_reaction(original.id)
        .unwrap()
        .custom_created_at(Timestamp::from(ts + 2))
        .sign_with_keys(&f.keys)
        .unwrap();
    ingest(&f, &deletion).await;
    let removed_row_exists: bool = sqlx::query_scalar(
        "SELECT deleted_at IS NOT NULL FROM events WHERE community_id = $1 AND id = $2",
    )
    .bind(f.community.as_uuid())
    .bind(original.id.as_bytes().as_slice())
    .fetch_one(&f.pool)
    .await
    .unwrap();
    assert!(removed_row_exists, "the removed reaction is still stored");

    ingest(&f, &replacement).await;
    let signed_timestamp = chrono::DateTime::from_timestamp((ts + 3) as i64, 0).unwrap();
    let source_key: (bool, Option<chrono::DateTime<chrono::Utc>>) = sqlx::query_as(
        "SELECT e.reaction_read_projected, r.reaction_event_created_at \
         FROM events e JOIN reactions r ON r.community_id = e.community_id \
         AND r.reaction_event_id = e.id WHERE e.community_id = $1 AND e.id = $2",
    )
    .bind(f.community.as_uuid())
    .bind(replacement.id.as_bytes().as_slice())
    .fetch_one(&f.pool)
    .await
    .unwrap();
    assert_eq!(source_key, (true, Some(signed_timestamp)));

    let channel = json!({"top_level":true,"#h":[f.channel],"kinds":[9],"include_aux":true});
    let by_target = json!({"#e":[f.root.id.to_hex()],"kinds":[7]});
    for read in [&filter, &channel, &by_target] {
        let page = f.query(read).await;
        assert_eq!(ids(&page, Some(7)), [replacement.id.to_hex()]);
        assert!(
            !ids(&page, Some(5)).contains(&deletion.id.to_hex().as_str()),
            "the auxiliary scan followed a removed reaction"
        );
    }

    let explicit = f.query(&json!({"ids":[deletion.id.to_hex()]})).await;
    assert_eq!(ids(&explicit, Some(5)), [deletion.id.to_hex()]);
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
    let reaction = EventBuilder::new(Kind::Reaction, "+")
        .tags([
            Tag::parse(["e", &a]).unwrap(),
            Tag::parse(["e", &b]).unwrap(),
        ])
        .sign_with_keys(&f.keys)
        .unwrap();
    ingest(&f, &reaction).await;

    let filter = f.filter();
    let page = f.query(&filter).await;
    assert_eq!(f.bounds(&page, &filter)["has_more"], false);
    assert_eq!(ids(&page, Some(7)), [reaction.id.to_hex()]);
    let aux = ids(&page, Some(40003));
    for (n, edit) in edits.iter().enumerate() {
        assert_eq!(aux.contains(&edit.id.to_hex().as_str()), n >= 2, "case {n}");
    }
    // Both non-target shapes really were stored and remain reachable under B.
    let mut other_filter = filter;
    other_filter["#e"] = json!([b]);
    let other_page = f.query(&other_filter).await;
    assert_eq!(ids(&other_page, Some(40003)).len(), 3);
    assert_eq!(ids(&other_page, Some(7)), [reaction.id.to_hex()]);
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
        .contains("payload byte budget"));
    let mut without_aux = filter;
    without_aux["include_aux"] = json!(false);
    let recovered = f.query(&without_aux).await;
    assert_eq!(f.bounds(&recovered, &without_aux)["has_more"], false);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn thread_window_signed_large_replies_without_aux_are_bounded() {
    let f = Fixture::new().await;
    for n in 0..201 {
        let reply = event(
            &f.keys,
            f.channel,
            9,
            &"x".repeat(256 * 1024),
            Some(&f.root),
            f.root.created_at.as_secs() + n,
        );
        ingest(&f, &reply).await;
    }
    let mut filter = f.filter();
    filter["include_aux"] = json!(false);
    filter["limit"] = json!(200);
    let (status, body) = f.post(&f.keys, "/query", json!([filter])).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
    assert!(body["error"]
        .as_str()
        .unwrap()
        .contains("payload byte budget"));
    assert!(
        body.as_array().is_none(),
        "no partial rows or signed bounds on failure"
    );
    filter["limit"] = json!(20);
    let page = f.query(&filter).await;
    assert_eq!(ids(&page, Some(9)).len(), 20);
    assert_eq!(f.bounds(&page, &filter)["has_more"], true);
    let (status, body) = f.post(&f.keys, "/query", json!([filter, filter])).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
    assert!(body["error"]
        .as_str()
        .unwrap()
        .contains("payload byte budget"));
}
