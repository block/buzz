//! `buzz repos state` — the NIP-34 repository state (kind:30618) desktop reads
//! to show a repository's live branches, tags, and HEAD.

use buzz_core::kind::KIND_GIT_REPO_STATE;
use nostr::Event;
use serde_json::{json, Value};

use crate::client::BuzzClient;
use crate::error::CliError;

const QUERY_LIMIT: u32 = 10;

/// The relay's NIP-11 `self` pubkey, or `None` when it advertises none or an
/// invalid one. A failed or malformed info document is an error.
async fn fetch_relay_self(client: &BuzzClient) -> Result<Option<String>, CliError> {
    let raw = client
        .get_public("/")
        .await
        .map_err(|e| CliError::Other(format!("failed to fetch relay info document: {e}")))?;
    let info: Value = serde_json::from_str(&raw)
        .map_err(|e| CliError::Other(format!("relay info document is not valid JSON: {e}")))?;
    Ok(info
        .get("self")
        .and_then(Value::as_str)
        .filter(|key| key.len() == 64 && key.chars().all(|c| c.is_ascii_hexdigit()))
        .map(str::to_ascii_lowercase))
}

fn tag_pairs(event: &Event) -> impl Iterator<Item = (&str, &str)> {
    event.tags.iter().filter_map(|tag| {
        let [name, value, ..] = tag.as_slice() else {
            return None;
        };
        (!name.is_empty() && !value.is_empty()).then_some((name.as_str(), value.as_str()))
    })
}

/// Project a state event the way desktop's `eventToRepoState` does.
fn project(event: &Event, repo_id: &str, owner: &str) -> Value {
    let mut branches = Vec::new();
    let mut tags = Vec::new();
    let mut head = Value::Null;
    for (name, value) in tag_pairs(event) {
        if let Some(branch) = name.strip_prefix("refs/heads/") {
            branches.push(json!({"name": branch, "commit": value}));
        } else if let Some(tag) = name.strip_prefix("refs/tags/") {
            tags.push(json!({"name": tag, "commit": value}));
        } else if name == "HEAD" {
            let target = value
                .strip_prefix("ref:")
                .map_or(value, |rest| rest.trim_start());
            head = json!(target.strip_prefix("refs/heads/").unwrap_or(target));
        }
    }
    json!({
        "repo_id": repo_id,
        "owner": owner,
        "head": head,
        "branches": branches,
        "tags": tags,
        "updated_at": event.created_at.as_secs(),
    })
}

/// The newest valid state event for `repo_id` from a trusted author. Ties on
/// `created_at` go to the lowest event id, as NIP-01 specifies.
fn select_head<'a>(
    events: &'a [Event],
    repo_id: &str,
    trusted: &[String],
) -> Result<Option<&'a Event>, CliError> {
    let head = events
        .iter()
        .filter(|event| {
            event.kind.as_u16() as u32 == KIND_GIT_REPO_STATE
                && trusted.contains(&event.pubkey.to_hex())
                && event.tags.identifier() == Some(repo_id)
                && event.verify().is_ok()
        })
        .min_by(|a, b| {
            b.created_at
                .cmp(&a.created_at)
                .then_with(|| a.id.to_hex().cmp(&b.id.to_hex()))
        });
    if head.is_none() && !events.is_empty() {
        return Err(CliError::Other(
            "the relay returned repository state events that are untrusted or fail verification"
                .into(),
        ));
    }
    Ok(head)
}

async fn read_state(
    client: &BuzzClient,
    repo_id: &str,
    owner: Option<&str>,
) -> Result<Value, CliError> {
    crate::validate::validate_repo_id(repo_id)?;
    let owner = match owner {
        Some(owner) => {
            crate::validate::validate_hex64(owner)?;
            owner.to_ascii_lowercase()
        }
        None => client.keys().public_key().to_hex(),
    };
    let mut trusted = vec![owner.clone()];
    if let Some(relay_self) = fetch_relay_self(client).await? {
        if relay_self != owner {
            trusted.push(relay_self);
        }
    }

    let filter = json!({
        "kinds": [KIND_GIT_REPO_STATE],
        "authors": trusted,
        "#d": [repo_id],
        "limit": QUERY_LIMIT,
    });
    let raw = client.query(&filter).await?;
    let events: Vec<Event> = serde_json::from_str(&raw)
        .map_err(|e| CliError::Other(format!("failed to parse relay response: {e}")))?;
    let event = select_head(&events, repo_id, &trusted)?.ok_or_else(|| {
        CliError::NotFound(format!(
            "no repository state found for {repo_id} owned by {owner}; \
             it appears after the first push"
        ))
    })?;
    Ok(project(event, repo_id, &owner))
}

pub(super) async fn cmd_repo_state(
    client: &BuzzClient,
    repo_id: &str,
    owner: Option<&str>,
) -> Result<(), CliError> {
    println!("{}", read_state(client, repo_id, owner).await?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Bytes, extract::State, routing::get, routing::post, Json, Router};
    use nostr::{EventBuilder, Keys, Kind, Tag, Timestamp};
    use std::sync::{Arc, Mutex};
    use tokio::net::TcpListener;

    #[derive(Default)]
    struct Relay {
        info: Value,
        events: Vec<Value>,
        filters: Vec<Value>,
    }
    type Shared = Arc<Mutex<Relay>>;

    async fn fake_relay(relay: Shared) -> String {
        let app = Router::new()
            .route(
                "/",
                get(|State(relay): State<Shared>| async move {
                    Json(relay.lock().unwrap().info.clone())
                }),
            )
            .route(
                "/query",
                post(|State(relay): State<Shared>, body: Bytes| async move {
                    let filters: Vec<Value> = serde_json::from_slice(&body).unwrap();
                    let [filter] = filters.as_slice() else {
                        panic!("one filter expected: {filters:?}")
                    };
                    let mut relay = relay.lock().unwrap();
                    relay.filters.push(filter.clone());
                    let authors = filter["authors"].as_array().unwrap().clone();
                    let matching: Vec<_> = relay
                        .events
                        .iter()
                        .filter(|e| {
                            filter["kinds"] == json!([e["kind"]])
                                && authors.contains(&e["pubkey"])
                                && e["tags"][0] == json!(["d", filter["#d"][0]])
                        })
                        .cloned()
                        .collect();
                    Json(json!(matching))
                }),
            )
            .with_state(relay);
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://{addr}")
    }

    async fn setup(relay_keys: &Keys) -> (BuzzClient, Shared) {
        let relay = Shared::default();
        relay.lock().unwrap().info = json!({"self": relay_keys.public_key().to_hex()});
        let url = fake_relay(relay.clone()).await;
        (
            BuzzClient::new(url, Keys::generate(), None, None).unwrap(),
            relay,
        )
    }

    fn state(keys: &Keys, d: &str, refs: &[(&str, &str)], created_at: u64) -> Event {
        let mut tags = vec![Tag::identifier(d)];
        tags.extend(refs.iter().map(|(n, v)| Tag::parse([*n, *v]).unwrap()));
        EventBuilder::new(Kind::Custom(KIND_GIT_REPO_STATE as u16), "")
            .tags(tags)
            .custom_created_at(Timestamp::from(created_at))
            .sign_with_keys(keys)
            .unwrap()
    }

    fn seed(relay: &Shared, event: &Event) {
        relay
            .lock()
            .unwrap()
            .events
            .push(serde_json::to_value(event).unwrap());
    }

    const OWNER_REFS: &[(&str, &str)] = &[
        ("HEAD", "ref: refs/heads/main"),
        ("refs/heads/main", "aaaa"),
        ("refs/heads/feat/x", "bbbb"),
        ("refs/tags/v1", "cccc"),
        ("refs/heads/empty", ""),
        ("refs/notes/other", "dddd"),
    ];

    #[tokio::test]
    async fn reads_the_newest_trusted_state_in_desktop_shape() {
        let relay_keys = Keys::generate();
        let (client, relay) = setup(&relay_keys).await;
        let me = client.keys().clone();
        let owner = me.public_key().to_hex();
        seed(
            &relay,
            &state(&me, "demo", &[("refs/heads/old", "0000")], 100),
        );
        seed(&relay, &state(&me, "demo", OWNER_REFS, 200));
        seed(
            &relay,
            &state(&relay_keys, "demo", &[("refs/heads/relay", "eeee")], 150),
        );
        seed(
            &relay,
            &state(&me, "other", &[("refs/heads/o", "ffff")], 900),
        );

        let got = read_state(&client, "demo", None).await.unwrap();
        assert_eq!(
            got,
            json!({
                "repo_id": "demo",
                "owner": owner,
                "head": "main",
                "branches": [
                    {"name": "main", "commit": "aaaa"},
                    {"name": "feat/x", "commit": "bbbb"},
                ],
                "tags": [{"name": "v1", "commit": "cccc"}],
                "updated_at": 200,
            })
        );

        let filter = relay.lock().unwrap().filters.last().cloned().unwrap();
        assert_eq!(filter["kinds"], json!([KIND_GIT_REPO_STATE]));
        assert_eq!(filter["#d"], json!(["demo"]));
        assert_eq!(
            filter["authors"],
            json!([owner, relay_keys.public_key().to_hex()])
        );
        assert_eq!(filter["limit"], json!(QUERY_LIMIT));

        seed(
            &relay,
            &state(&relay_keys, "demo", &[("refs/heads/relay", "eeee")], 300),
        );
        let got = read_state(&client, "demo", Some(&owner)).await.unwrap();
        assert_eq!(got["updated_at"], json!(300));
        assert_eq!(
            got["branches"],
            json!([{"name": "relay", "commit": "eeee"}])
        );
        assert_eq!(got["head"], Value::Null);
        assert_eq!(got["tags"], json!([]));
    }

    #[tokio::test]
    async fn explicit_owner_scopes_the_query_and_untrusted_authors_never_win() {
        let relay_keys = Keys::generate();
        let (client, relay) = setup(&relay_keys).await;
        let owner = Keys::generate();
        let stranger = Keys::generate();
        seed(
            &relay,
            &state(&owner, "demo", &[("refs/heads/real", "1111")], 10),
        );

        let got = read_state(&client, "demo", Some(&owner.public_key().to_hex()))
            .await
            .unwrap();
        assert_eq!(got["owner"], json!(owner.public_key().to_hex()));
        assert_eq!(got["branches"], json!([{"name": "real", "commit": "1111"}]));
        let filter = relay.lock().unwrap().filters.last().cloned().unwrap();
        assert_eq!(
            filter["authors"],
            json!([
                owner.public_key().to_hex(),
                relay_keys.public_key().to_hex()
            ])
        );

        let forged = state(&stranger, "demo", &[("refs/heads/evil", "9999")], 99);
        assert!(select_head(
            std::slice::from_ref(&forged),
            "demo",
            &[owner.public_key().to_hex()]
        )
        .is_err());
        let real = state(&owner, "demo", &[("refs/heads/real", "1111")], 10);
        let both = [forged, real.clone()];
        let picked = select_head(&both, "demo", &[owner.public_key().to_hex()])
            .unwrap()
            .unwrap();
        assert_eq!(picked.id, real.id);
    }

    #[tokio::test]
    async fn failures_never_look_like_an_empty_state() {
        let relay_keys = Keys::generate();
        let (client, relay) = setup(&relay_keys).await;
        assert!(matches!(
            read_state(&client, "demo", None).await,
            Err(CliError::NotFound(_))
        ));
        assert!(matches!(
            read_state(&client, "bad id", None).await,
            Err(CliError::Usage(_))
        ));
        assert!(matches!(
            read_state(&client, "demo", Some("nothex")).await,
            Err(CliError::Usage(_))
        ));

        let me = client.keys().clone();
        let me_hex = [me.public_key().to_hex()];
        let wrong_repo = state(&me, "other", &[("refs/heads/x", "1")], 5);
        assert!(select_head(&[wrong_repo], "demo", &me_hex).is_err());
        let (a, b) = (
            state(&me, "demo", &[("refs/heads/a", "1")], 7),
            state(&me, "demo", &[("refs/heads/b", "2")], 7),
        );
        let lowest = std::cmp::min(a.id.to_hex(), b.id.to_hex());
        let tied = [a, b];
        let picked = select_head(&tied, "demo", &me_hex).unwrap().unwrap();
        assert_eq!(picked.id.to_hex(), lowest);
        let mut tampered = serde_json::to_value(state(&me, "demo", OWNER_REFS, 5)).unwrap();
        tampered["tags"][1] = json!(["refs/heads/main", "forged"]);
        relay.lock().unwrap().events.push(tampered);
        assert!(matches!(
            read_state(&client, "demo", None).await,
            Err(CliError::Other(_))
        ));
    }

    #[tokio::test]
    async fn relay_without_a_self_key_trusts_only_the_owner() {
        let (client, relay) = setup(&Keys::generate()).await;
        relay.lock().unwrap().info = json!({"self": "nothex"});
        let me = client.keys().clone();
        seed(&relay, &state(&me, "demo", OWNER_REFS, 5));
        read_state(&client, "demo", None).await.unwrap();
        let filter = relay.lock().unwrap().filters.last().cloned().unwrap();
        assert_eq!(filter["authors"], json!([me.public_key().to_hex()]));
    }

    #[test]
    fn head_follows_desktop_normalization() {
        let keys = Keys::generate();
        for (value, want) in [
            ("ref: refs/heads/trunk", json!("trunk")),
            ("ref:refs/heads/trunk", json!("trunk")),
            ("refs/heads/trunk", json!("trunk")),
            ("ref: refs/tags/x", json!("refs/tags/x")),
        ] {
            let event = state(&keys, "demo", &[("HEAD", value)], 1);
            assert_eq!(project(&event, "demo", "o")["head"], want, "{value}");
        }
    }

    #[test]
    fn cli_parses_state_and_templates_list() {
        use clap::Parser;
        for args in [
            vec!["buzz", "repos", "state", "--repo", "demo"],
            vec!["buzz", "repos", "state", "--id", "demo", "--owner", "ab"],
            vec!["buzz", "channels", "templates", "list"],
            vec![
                "buzz",
                "channels",
                "templates",
                "list",
                "--templates-file",
                "/x",
            ],
        ] {
            assert!(crate::Cli::try_parse_from(&args).is_ok(), "{args:?}");
        }
        assert!(crate::Cli::try_parse_from(["buzz", "repos", "state"]).is_err());
    }
}
