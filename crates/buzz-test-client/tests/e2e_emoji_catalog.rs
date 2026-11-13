//! End-to-end test: the custom emoji catalog (NIP-CE) over HTTP.
//!
//! Members publish kind:30180 entries through `POST /events` and read the
//! catalog they build through `POST /emoji/catalog`.
//!
//! # Running
//!
//! Start the relay, then run:
//!
//! ```text
//! RELAY_URL=ws://localhost:3001 cargo test -p buzz-test-client --test e2e_emoji_catalog -- --ignored
//! ```

use std::time::Duration;

use nostr::Keys;
use reqwest::Client;
use serde_json::{json, Value};

fn relay_http_url() -> String {
    std::env::var("RELAY_URL")
        .unwrap_or_else(|_| "ws://localhost:3001".to_string())
        .replace("wss://", "https://")
        .replace("ws://", "http://")
        .trim_end_matches('/')
        .to_string()
}

/// Submit a signed event; returns whether it was accepted and the relay's message.
async fn submit(client: &Client, keys: &Keys, builder: nostr::EventBuilder) -> (bool, String) {
    let event = builder.sign_with_keys(keys).expect("sign event");
    let resp = client
        .post(format!("{}/events", relay_http_url()))
        .header("X-Pubkey", keys.public_key().to_hex())
        .json(&event)
        .send()
        .await
        .expect("submit event");
    let ok = resp.status().is_success();
    let body: Value = resp.json().await.expect("parse response");
    if ok {
        (
            body["accepted"].as_bool().unwrap_or(false),
            body["message"].as_str().unwrap_or("").to_string(),
        )
    } else {
        (false, body["error"].as_str().unwrap_or("").to_string())
    }
}

async fn catalog(client: &Client, keys: &Keys, body: Value) -> (u16, Value) {
    let url = format!("{}/emoji/catalog", relay_http_url());
    let body = serde_json::to_vec(&body).expect("encode body");
    let resp = client
        .post(&url)
        .header(
            "Authorization",
            buzz_test_client::loadgen::identity::nip98_header(keys, "POST", &url, &body),
        )
        .header("Content-Type", "application/json")
        .body(body)
        .send()
        .await
        .expect("read catalog");
    let status = resp.status().as_u16();
    (status, resp.json().await.unwrap_or(Value::Null))
}

/// The rows for `names`, as `(shortcode, url, author pubkey)`.
fn rows(body: &Value, names: &[&str]) -> Vec<(String, Option<String>, String)> {
    body["emoji"]
        .as_array()
        .expect("emoji rows")
        .iter()
        .filter(|row| names.contains(&row[0].as_str().expect("shortcode")))
        .map(|row| {
            let author = row[3].as_u64().expect("author index") as usize;
            (
                row[0].as_str().expect("shortcode").to_owned(),
                row[2].as_str().map(str::to_owned),
                body["authors"][author].as_str().expect("author").to_owned(),
            )
        })
        .collect()
}

/// Entries published over HTTP build the catalog that `POST /emoji/catalog`
/// serves: a shortcode has one owner, a removal reaches a client that reads
/// changes, and a first read leaves removed shortcodes out.
#[tokio::test]
#[ignore = "requires a running relay"]
async fn emoji_entries_are_read_back_as_catalog_changes() {
    let client = Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .expect("build HTTP client");
    let (owner, other) = (Keys::generate(), Keys::generate());
    let owner_hex = owner.public_key().to_hex();
    // Shortcodes are unique in the community, so each run uses its own.
    let run = &uuid::Uuid::new_v4().simple().to_string()[..12];
    let (kept, removed) = (format!("kept_{run}"), format!("removed_{run}"));
    let names = [kept.as_str(), removed.as_str()];
    let url = "https://example.com/emoji.png";
    // Entries are dated before the removal: within one second the relay keeps
    // the lower event ID, whichever arrived first.
    let earlier = nostr::Timestamp::from(nostr::Timestamp::now().as_secs() - 5);
    let entry = |name: &str, url: &str| {
        buzz_sdk::build_emoji_entry(name, url)
            .expect("entry")
            .custom_created_at(earlier)
    };

    // Where the catalog stands before this run.
    let (status, before) = catalog(&client, &owner, json!({ "since": 0 })).await;
    assert_eq!(status, 200, "{before}");
    let mut since = 0;
    let mut page = before;
    loop {
        if let Some(last) = page["emoji"].as_array().and_then(|rows| rows.last()) {
            since = last[1].as_i64().expect("change");
        }
        if page["more"] != true {
            break;
        }
        page = catalog(&client, &owner, json!({ "since": since })).await.1;
    }

    for name in names {
        let (accepted, message) = submit(&client, &owner, entry(name, url)).await;
        assert!(accepted, "{message}");
    }
    let (accepted, message) =
        submit(&client, &other, entry(&kept, "https://example.com/x.png")).await;
    assert!(
        !accepted && message.contains("belongs to another member"),
        "{message}"
    );

    let (_, added) = catalog(&client, &other, json!({ "since": since })).await;
    assert_eq!(
        rows(&added, &names),
        vec![
            (kept.clone(), Some(url.to_owned()), owner_hex.clone()),
            (removed.clone(), Some(url.to_owned()), owner_hex.clone()),
        ]
    );
    let after_add = added["emoji"]
        .as_array()
        .expect("rows")
        .last()
        .expect("row")[1]
        .as_i64()
        .expect("change");
    assert!(
        after_add >= since + 2,
        "each write takes a new change number"
    );

    let removal = buzz_sdk::build_emoji_entry_removal(&removed).expect("removal");
    let (accepted, message) = submit(&client, &owner, removal).await;
    assert!(accepted, "{message}");

    // A client that holds the emoji learns it was removed.
    let (_, changed) = catalog(&client, &other, json!({ "since": after_add })).await;
    assert_eq!(
        rows(&changed, &names),
        vec![(removed.clone(), None, owner_hex.clone())]
    );
    // Lookup by name returns live emoji only, and normalizes the names.
    let (_, found) = catalog(
        &client,
        &other,
        json!({ "shortcodes": [format!(":{}:", kept.to_uppercase()), removed] }),
    )
    .await;
    assert_eq!(
        rows(&found, &names),
        vec![(kept, Some(url.to_owned()), owner_hex)]
    );

    let (status, _) = catalog(&client, &other, json!({})).await;
    assert_eq!(status, 400);
}
