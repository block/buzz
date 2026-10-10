use super::*;
use nostr::Timestamp;

fn snapshot(keys: &Keys, channel: Uuid, time: u64, labels: &[&str]) -> Event {
    let mut tags = vec![
        Tag::parse(["d", &channel.to_string()]).unwrap(),
        Tag::parse(["t", "stream"]).unwrap(),
    ];
    if !labels.is_empty() {
        tags.push(Tag::parse(["L", "nip-cl"]).unwrap());
        for label in labels {
            tags.push(Tag::parse(["l", label, "nip-cl"]).unwrap());
        }
    }
    EventBuilder::new(Kind::Custom(39000), "")
        .tags(tags)
        .custom_created_at(Timestamp::from(time))
        .sign_with_keys(keys)
        .unwrap()
}

#[tokio::test]
async fn verified_reads_use_nip01_replacement_order_and_find_uses_generic_filters() {
    let f = Fixture::new(vec![]).await;
    let channel = Uuid::new_v4();
    let old = snapshot(&f.relay, channel, 10, &["a"]);
    let mut tied = [
        snapshot(&f.relay, channel, 20, &["b"]),
        snapshot(&f.relay, channel, 20, &["c"]),
    ];
    tied.sort_by_key(|event| event.id);
    let other = snapshot(&f.relay, Uuid::new_v4(), 30, &["a"]);
    let events = vec![other.clone(), tied[1].clone(), tied[0].clone(), old];
    f.state.lock().unwrap().snapshots = events.clone();
    let result = read(
        &f.client,
        f.relay.public_key(),
        json!({"kinds":[39000]}),
        None,
    )
    .await
    .unwrap();
    assert_eq!(result.as_array().unwrap().len(), 2);
    assert_eq!(result[0]["event"]["id"], json!(other.id));
    assert_eq!(result[1]["event"]["id"], json!(tied[0].id));
    assert!(read(
        &f.client,
        f.relay.public_key(),
        json!({"kinds":[39000]}),
        Some(channel)
    )
    .await
    .is_err());
    dispatch(
        ChannelLabelsCmd::Find {
            labels: vec!["a".into(), "b".into()],
            channel_type: Some(crate::ChannelType::Stream),
            limit: 3,
            trusted_relay: Some(f.relay.public_key().to_hex()),
        },
        &f.client,
    )
    .await
    .unwrap();
    assert_eq!(
        f.state.lock().unwrap().filters.last().unwrap(),
        &json!([{
            "kinds":[39000], "authors":[f.relay.public_key()], "#L":["nip-cl"], "#l":["a","b"], "#t":["stream"], "limit":3
        }])
    );
}

#[tokio::test]
async fn untrusted_or_noncanonical_reads_are_errors_never_empty_authoritative_labels() {
    let f = Fixture::new(vec![]).await;
    let channel = Uuid::new_v4();
    let valid = snapshot(&f.relay, channel, 20, &["a"]);
    let mut wrong_signature = valid.clone();
    wrong_signature.content = "tampered".into();
    let mut cases = vec![
        snapshot(&Keys::generate(), channel, 20, &["a"]),
        wrong_signature,
        snapshot(&f.relay, channel, 20, &["z", "a"]),
        snapshot(&f.relay, channel, 20, &["a", "a"]),
        snapshot(&f.relay, Uuid::new_v4(), 20, &["a"]),
    ];
    for malformed in [
        vec!["label", "legacy"],
        vec!["l", "unmarked"],
        vec!["L", "elsewhere"],
        vec!["d", "duplicate"],
    ] {
        cases.push(
            EventBuilder::new(Kind::Custom(39000), "")
                .tags(
                    valid
                        .tags
                        .iter()
                        .cloned()
                        .chain([Tag::parse(malformed).unwrap()]),
                )
                .sign_with_keys(&f.relay)
                .unwrap(),
        );
    }
    for event in cases {
        f.state.lock().unwrap().snapshots = vec![event];
        assert!(read(
            &f.client,
            f.relay.public_key(),
            json!({"kinds":[39000]}),
            Some(channel)
        )
        .await
        .is_err());
    }
    f.state.lock().unwrap().snapshots = vec![snapshot(&f.relay, channel, 40, &[])];
    let result = read(
        &f.client,
        f.relay.public_key(),
        json!({"kinds":[39000]}),
        Some(channel),
    )
    .await
    .unwrap();
    assert_eq!(result[0]["labels"], json!([]));
}
