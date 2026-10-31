//! `buzz sidebar sections` — your own channel sections, the same synced
//! preference the desktop sidebar edits (kind:30078, d=`channel-sections`,
//! NIP-44 encrypted to yourself).
//!
//! Every write is one read of the current head followed by one publish, so a
//! multi-channel `move` lands atomically. A head this CLI cannot read is never
//! overwritten.

use std::time::SystemTime;

use buzz_core::kind::KIND_READ_STATE;
use nostr::nips::nip44;
use nostr::{EventBuilder, Kind, Tag, Timestamp};
use serde_json::{json, Value};

use super::parse_write_response;
use super::sidebar_doc::{self, Edit, Projection, Section, Stamp, Tree};
use crate::client::BuzzClient;
use crate::error::CliError;
use crate::{SidebarCmd, SidebarSectionsCmd};

const D_TAG: &str = "channel-sections";
/// Relay event size ceiling shared with desktop's sidebar blobs.
const MAX_CIPHERTEXT_BYTES: usize = 65_535;

struct Head {
    created_at: u64,
    tree: Tree,
}

fn now() -> std::time::Duration {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
}

/// The relay's current sections event for this identity, decoded. An absent
/// head is an empty tree; an unreadable one is an error, never an empty list.
/// The read is pinned to the writer (`consistency: strong`): a write derives a
/// full snapshot from it, and a lagging replica would drop the caller's own
/// just-accepted edits.
async fn fetch_head(client: &BuzzClient) -> Result<Head, CliError> {
    let me = client.keys().public_key();
    let filter = json!({
        "kinds": [KIND_READ_STATE],
        "authors": [me.to_hex()],
        "#d": [D_TAG],
        "limit": 1,
        "consistency": "strong",
    });
    let raw = client.query(&filter).await?;
    let events: Vec<Value> = serde_json::from_str(&raw)
        .map_err(|e| CliError::Other(format!("failed to parse sidebar sections query: {e}")))?;
    let returned = events.len();
    let head = events
        .into_iter()
        .filter_map(|event| serde_json::from_value::<nostr::Event>(event).ok())
        .filter(|event| {
            event.pubkey == me && event.tags.identifier() == Some(D_TAG) && event.verify().is_ok()
        })
        .min_by(|a, b| {
            b.created_at
                .cmp(&a.created_at)
                .then_with(|| a.id.to_hex().cmp(&b.id.to_hex()))
        });
    let Some(event) = head else {
        if returned > 0 {
            return Err(CliError::Other(
                "the relay returned a sidebar sections event that is not yours or fails verification; refusing to overwrite it".into(),
            ));
        }
        return Ok(Head {
            created_at: 0,
            tree: Tree::new(),
        });
    };

    let unreadable = |detail: String| {
        CliError::Other(format!(
            "your sidebar sections event {} is unreadable ({detail}); refusing to read or overwrite it",
            event.id.to_hex()
        ))
    };
    let plaintext = nip44::decrypt(client.keys().secret_key(), &me, &event.content)
        .map_err(|e| unreadable(e.to_string()))?;
    let json: Value = serde_json::from_str(&plaintext).map_err(|e| unreadable(e.to_string()))?;
    let created_at = event.created_at.as_secs();
    let legacy_v = i64::try_from(created_at.saturating_mul(1_000)).unwrap_or(i64::MAX);
    let tree = sidebar_doc::decode(&json, legacy_v).ok_or_else(|| {
        unreadable("unsupported format; a newer client may have written it".into())
    })?;
    Ok(Head { created_at, tree })
}

/// A live section by exact id, or by exact name when that name is unique.
fn resolve<'a>(view: &'a Projection, key: &str) -> Result<&'a Section, CliError> {
    if let Some(section) = view.sections.iter().find(|s| s.id == key) {
        return Ok(section);
    }
    let key = key.trim();
    let mut named = view.sections.iter().filter(|s| s.name == key);
    match (named.next(), named.next()) {
        (Some(section), None) => Ok(section),
        (Some(_), Some(_)) => Err(CliError::Usage(format!(
            "more than one section is named \"{key}\"; pass the section id from `buzz sidebar sections list`"
        ))),
        (None, _) => Err(CliError::NotFound(format!(
            "no section has the id or name \"{key}\""
        ))),
    }
}

fn section_name(name: &str) -> Result<String, CliError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(CliError::Usage("section name must not be empty".into()));
    }
    Ok(name.to_string())
}

fn section_icon(icon: Option<&str>) -> Result<Option<String>, CliError> {
    match icon.map(str::trim) {
        Some("") => Err(CliError::Usage(
            "--icon must not be empty; use --clear-icon to remove an icon".into(),
        )),
        icon => Ok(icon.map(str::to_string)),
    }
}

/// Channel ids in the lowercase hyphenated form desktop keys assignments by.
fn channel_ids(channels: &[String]) -> Result<Vec<String>, CliError> {
    channels
        .iter()
        .map(|channel| {
            uuid::Uuid::parse_str(channel.trim())
                .map(|id| id.hyphenated().to_string())
                .map_err(|_| CliError::Usage(format!("invalid channel UUID: {channel}")))
        })
        .collect()
}

/// Reads the head, applies `apply`, and publishes the result once. Returns
/// the write response, or an `unchanged` response without publishing.
async fn edit(
    client: &BuzzClient,
    apply: impl FnOnce(&mut Edit, &Projection) -> Result<(), CliError>,
) -> Result<Value, CliError> {
    let head = fetch_head(client).await?;
    let view = sidebar_doc::project(&head.tree);
    let mut tree = head.tree.clone();
    let stamp = Stamp {
        now_ms: i64::try_from(now().as_millis()).unwrap_or(i64::MAX),
        dev: format!("{:016x}", rand::random::<u64>()),
    };
    apply(&mut Edit::new(&mut tree, &stamp), &view)?;
    if tree == head.tree {
        return Ok(json!({"event_id": null, "accepted": true, "message": "unchanged"}));
    }
    if !sidebar_doc::project(&tree).within_limits() {
        return Err(CliError::Usage(format!(
            "the sidebar supports at most {} sections and {} channel assignments",
            sidebar_doc::MAX_SECTIONS,
            sidebar_doc::MAX_ASSIGNMENTS
        )));
    }

    let keys = client.keys();
    let plaintext = serde_json::to_string(&sidebar_doc::encode(&tree))
        .map_err(|e| CliError::Other(format!("failed to encode sidebar sections: {e}")))?;
    let too_large =
        || CliError::Usage("sidebar sections would exceed the relay's event size limit".into());
    // NIP-44 rejects longer plaintexts outright.
    if plaintext.len() > u16::MAX as usize {
        return Err(too_large());
    }
    let content = nip44::encrypt(
        keys.secret_key(),
        &keys.public_key(),
        plaintext,
        nip44::Version::V2,
    )
    .map_err(|e| CliError::Other(format!("failed to encrypt sidebar sections: {e}")))?;
    if content.len() > MAX_CIPHERTEXT_BYTES {
        return Err(too_large());
    }
    let created_at = now().as_secs().max(head.created_at + 1);
    let builder = EventBuilder::new(Kind::Custom(KIND_READ_STATE as u16), content)
        .tags([Tag::identifier(D_TAG), Tag::hashtag(D_TAG)])
        .custom_created_at(Timestamp::from(created_at));
    let raw = client.submit_event(client.sign_event(builder)?).await?;
    let normalized = parse_write_response(
        &raw,
        "a newer sidebar sections update won; run the command again",
    )?;
    serde_json::from_str(&normalized)
        .map_err(|e| CliError::Other(format!("relay response is not JSON: {e}")))
}

async fn run(cmd: SidebarSectionsCmd, client: &BuzzClient) -> Result<Value, CliError> {
    match cmd {
        SidebarSectionsCmd::List => {
            let head = fetch_head(client).await?;
            serde_json::to_value(sidebar_doc::project(&head.tree))
                .map_err(|e| CliError::Other(format!("failed to encode sections: {e}")))
        }
        SidebarSectionsCmd::Create { name, icon } => {
            let name = section_name(&name)?;
            let icon = section_icon(icon.as_deref())?;
            let id = uuid::Uuid::new_v4().to_string();
            let mut response = edit(client, |edit, _| {
                edit.create_section(&id, &name, icon.as_deref());
                Ok(())
            })
            .await?;
            response["section_id"] = json!(id);
            Ok(response)
        }
        SidebarSectionsCmd::Update {
            section,
            name,
            icon,
            clear_icon,
        } => {
            if name.is_none() && icon.is_none() && !clear_icon {
                return Err(CliError::Usage(
                    "pass --name, --icon, or --clear-icon".into(),
                ));
            }
            let name = name.as_deref().map(section_name).transpose()?;
            let icon = section_icon(icon.as_deref())?;
            edit(client, |edit, view| {
                let id = resolve(view, &section)?.id.clone();
                if let Some(name) = name {
                    edit.set(&["s", &id, "name"], json!(name));
                }
                if clear_icon || icon.is_some() {
                    edit.set(&["s", &id, "icon"], json!(icon));
                }
                Ok(())
            })
            .await
        }
        SidebarSectionsCmd::Delete { section } => {
            edit(client, |edit, view| {
                let id = resolve(view, &section)?.id.clone();
                edit.set(&["s", &id, "live"], json!(false));
                Ok(())
            })
            .await
        }
        SidebarSectionsCmd::Reorder { sections } => {
            edit(client, |edit, view| {
                let ids = sections
                    .iter()
                    .map(|key| Ok(resolve(view, key)?.id.clone()))
                    .collect::<Result<Vec<_>, CliError>>()?;
                edit.reorder(&ids);
                Ok(())
            })
            .await
        }
        SidebarSectionsCmd::Move { channels, section } => {
            let channels = channel_ids(&channels)?;
            edit(client, |edit, view| {
                let id = resolve(view, &section)?.id.clone();
                for channel in &channels {
                    edit.set(&["a", channel], json!(id));
                }
                Ok(())
            })
            .await
        }
        SidebarSectionsCmd::Unassign { channels } => {
            let channels = channel_ids(&channels)?;
            edit(client, |edit, _| {
                for channel in &channels {
                    edit.unassign(channel);
                }
                Ok(())
            })
            .await
        }
    }
}

pub async fn dispatch(cmd: SidebarCmd, client: &BuzzClient) -> Result<(), CliError> {
    let SidebarCmd::Sections(cmd) = cmd;
    let output = run(cmd, client).await?;
    println!("{output}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Bytes, extract::State, routing::post, Json, Router};
    use nostr::Keys;
    use std::sync::{Arc, Mutex};
    use tokio::net::TcpListener;

    #[derive(Default)]
    struct Relay {
        events: Vec<Value>,
        duplicate: bool,
    }
    type Shared = Arc<Mutex<Relay>>;

    /// Fake relay: `/query` checks the head filter and returns every stored
    /// event matching its author and d-tag (more than the relay's `limit`, so
    /// head selection is exercised); `/events` stores one.
    async fn fake_relay(relay: Shared) -> String {
        let app = Router::new()
            .route(
                "/query",
                post(|State(relay): State<Shared>, body: Bytes| async move {
                    let filters: Vec<Value> = serde_json::from_slice(&body).unwrap();
                    let [filter] = filters.as_slice() else {
                        panic!("one filter expected: {filters:?}")
                    };
                    assert_eq!(filter["kinds"], json!([KIND_READ_STATE]));
                    assert_eq!(filter["#d"], json!([D_TAG]));
                    assert_eq!(filter["limit"], json!(1));
                    assert_eq!(filter["consistency"], json!("strong"));
                    let author = &filter["authors"][0];
                    let relay = relay.lock().unwrap();
                    let matching: Vec<_> = relay
                        .events
                        .iter()
                        .filter(|e| &e["pubkey"] == author && e["tags"][0] == json!(["d", D_TAG]))
                        .cloned()
                        .collect();
                    Json(json!(matching))
                }),
            )
            .route(
                "/events",
                post(|State(relay): State<Shared>, body: Bytes| async move {
                    let event: Value = serde_json::from_slice(&body).unwrap();
                    let mut relay = relay.lock().unwrap();
                    let message = if relay.duplicate {
                        "duplicate: newer head"
                    } else {
                        ""
                    };
                    if !relay.duplicate {
                        relay.events.push(event.clone());
                    }
                    Json(json!({"event_id": event["id"], "accepted": true, "message": message}))
                }),
            )
            .with_state(relay);
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://{addr}")
    }

    async fn setup() -> (BuzzClient, Shared) {
        let relay = Shared::default();
        let url = fake_relay(relay.clone()).await;
        let client = BuzzClient::new(url, Keys::generate(), None, None).unwrap();
        (client, relay)
    }

    fn seed_at(client: &BuzzClient, relay: &Shared, plaintext: &str, created_at: u64) -> Value {
        let keys = client.keys();
        let content = nip44::encrypt(
            keys.secret_key(),
            &keys.public_key(),
            plaintext,
            nip44::Version::V2,
        )
        .unwrap();
        let event = EventBuilder::new(Kind::Custom(KIND_READ_STATE as u16), content)
            .tags([Tag::identifier(D_TAG)])
            .custom_created_at(Timestamp::from(created_at))
            .sign_with_keys(keys)
            .unwrap();
        let event = serde_json::to_value(event).unwrap();
        relay.lock().unwrap().events.push(event.clone());
        event
    }

    fn seed(client: &BuzzClient, relay: &Shared, plaintext: &str) {
        seed_at(client, relay, plaintext, now().as_secs());
    }

    fn doc_with_sections(names: &[&str]) -> String {
        let mut tree = Tree::new();
        let stamp = Stamp {
            now_ms: 1,
            dev: "0123456789abcdef".into(),
        };
        let mut edit = Edit::new(&mut tree, &stamp);
        for name in names {
            edit.create_section(&format!("id-{name}"), name, None);
        }
        sidebar_doc::encode(&tree).to_string()
    }

    /// The decrypted document of the newest published event.
    fn published(client: &BuzzClient, relay: &Shared) -> (nostr::Event, Value) {
        let raw = relay.lock().unwrap().events.last().cloned().unwrap();
        let event: nostr::Event = serde_json::from_value(raw).unwrap();
        let keys = client.keys();
        let plaintext =
            nip44::decrypt(keys.secret_key(), &keys.public_key(), &event.content).unwrap();
        (event, serde_json::from_str(&plaintext).unwrap())
    }

    fn published_count(relay: &Shared) -> usize {
        relay.lock().unwrap().events.len()
    }

    const C1: &str = "11111111-1111-4111-8111-111111111111";
    const C2: &str = "22222222-2222-4222-8222-222222222222";
    const C3: &str = "abcdef01-2345-4678-89ab-cdef01234567";

    fn mv(channels: &[&str], section: &str) -> SidebarSectionsCmd {
        SidebarSectionsCmd::Move {
            channels: channels.iter().map(|c| c.to_string()).collect(),
            section: section.into(),
        }
    }

    #[tokio::test]
    async fn sets_up_a_sidebar_in_one_pass_and_syncs_desktop_format() {
        let (client, relay) = setup().await;
        let list = run(SidebarSectionsCmd::List, &client).await.unwrap();
        assert_eq!(list, json!({"sections": [], "assignments": {}}));

        let eng = run(
            SidebarSectionsCmd::Create {
                name: "  Engineering ".into(),
                icon: Some("🛠".into()),
            },
            &client,
        )
        .await
        .unwrap();
        let eng_id = eng["section_id"].as_str().unwrap().to_string();
        run(
            SidebarSectionsCmd::Create {
                name: "Design".into(),
                icon: None,
            },
            &client,
        )
        .await
        .unwrap();

        // Both channels land in one publish, addressed by section name.
        let before = published_count(&relay);
        run(mv(&[C1, C2], "Engineering"), &client).await.unwrap();
        assert_eq!(published_count(&relay), before + 1);

        let (event, doc) = published(&client, &relay);
        assert_eq!(event.kind.as_u16() as u32, KIND_READ_STATE);
        assert!(event.tags.iter().any(|t| t.as_slice() == ["d", D_TAG]));
        assert!(event.tags.iter().any(|t| t.as_slice() == ["t", D_TAG]));
        assert_eq!(doc["meta"]["v"], json!(1));
        assert_eq!(doc["assignments"][C1], json!(eng_id));
        assert_eq!(doc["sections"][0]["icon"], json!("🛠"));

        // A repeated command publishes nothing.
        let again = run(mv(&[C1, C2], &eng_id), &client).await.unwrap();
        assert_eq!(again["message"], json!("unchanged"));
        assert_eq!(published_count(&relay), before + 1);

        // Invalid input fails before any publish.
        let err = run(mv(&[C1, "not-a-uuid"], "Design"), &client)
            .await
            .unwrap_err();
        assert!(matches!(err, CliError::Usage(_)), "{err:?}");
        let err = run(mv(&[C1], "Nope"), &client).await.unwrap_err();
        assert!(matches!(err, CliError::NotFound(_)), "{err:?}");
        assert_eq!(published_count(&relay), before + 1);

        run(
            SidebarSectionsCmd::Reorder {
                sections: vec!["Design".into()],
            },
            &client,
        )
        .await
        .unwrap();
        run(
            SidebarSectionsCmd::Update {
                section: eng_id.clone(),
                name: Some("Eng".into()),
                icon: None,
                clear_icon: true,
            },
            &client,
        )
        .await
        .unwrap();
        run(
            SidebarSectionsCmd::Unassign {
                channels: vec![C2.into()],
            },
            &client,
        )
        .await
        .unwrap();
        run(
            SidebarSectionsCmd::Delete {
                section: "Design".into(),
            },
            &client,
        )
        .await
        .unwrap();

        let list = run(SidebarSectionsCmd::List, &client).await.unwrap();
        assert_eq!(
            list,
            json!({
                "sections": [{"id": eng_id, "name": "Eng", "order": 0}],
                "assignments": {C1: eng_id},
            })
        );
        let (_, doc) = published(&client, &relay);
        assert_eq!(doc["sections"], list["sections"]);
    }

    #[tokio::test]
    async fn keeps_a_mobile_head_and_never_overwrites_an_unreadable_one() {
        let (client, relay) = setup().await;
        seed(
            &client,
            &relay,
            r#"{"version":1,"sections":[{"id":"m1","name":"Mobile","order":0}],"assignments":{}}"#,
        );
        run(mv(&[C1], "Mobile"), &client).await.unwrap();
        let list = run(SidebarSectionsCmd::List, &client).await.unwrap();
        assert_eq!(list["sections"][0]["name"], json!("Mobile"));
        assert_eq!(list["assignments"][C1], json!("m1"));

        seed_at(
            &client,
            &relay,
            r#"{"version":1,"meta":{"v":2}}"#,
            now().as_secs() + 100,
        );
        let count = published_count(&relay);
        assert!(run(SidebarSectionsCmd::List, &client).await.is_err());
        assert!(run(mv(&[C2], "Mobile"), &client).await.is_err());
        assert_eq!(published_count(&relay), count);
    }

    #[tokio::test]
    async fn picks_the_newest_head_and_publishes_after_it() {
        let (client, relay) = setup().await;
        let future = now().as_secs() + 1_000;
        seed_at(&client, &relay, &doc_with_sections(&["Old"]), future - 1);
        let a = seed_at(&client, &relay, &doc_with_sections(&["A"]), future);
        let b = seed_at(&client, &relay, &doc_with_sections(&["B"]), future);
        // Equal `created_at`: the lower id wins, as on the relay.
        let winner = if a["id"].as_str() < b["id"].as_str() {
            "A"
        } else {
            "B"
        };
        let list = run(SidebarSectionsCmd::List, &client).await.unwrap();
        assert_eq!(list["sections"][0]["name"], json!(winner));

        // Another author's head is never read as ours.
        let other = BuzzClient::new(String::new(), Keys::generate(), None, None).unwrap();
        seed_at(&other, &relay, &doc_with_sections(&["Theirs"]), future + 5);

        // An uppercase channel id is stored in desktop's canonical form.
        run(mv(&[&C3.to_uppercase()], winner), &client)
            .await
            .unwrap();
        let (event, doc) = published(&client, &relay);
        assert_eq!(event.created_at.as_secs(), future + 1);
        assert_eq!(doc["assignments"][C3], json!(format!("id-{winner}")));
    }

    #[tokio::test]
    async fn refuses_a_tampered_head_and_oversized_documents() {
        let (client, relay) = setup().await;
        let mut head = seed_at(&client, &relay, &doc_with_sections(&["A"]), 10);
        head["content"] = json!("tampered");
        relay.lock().unwrap().events = vec![head];
        let err = run(mv(&[C1], "A"), &client).await.unwrap_err();
        assert!(matches!(err, CliError::Other(_)), "{err:?}");
        assert_eq!(published_count(&relay), 1);

        let names: Vec<String> = (0..sidebar_doc::MAX_SECTIONS)
            .map(|i| i.to_string())
            .collect();
        let names: Vec<&str> = names.iter().map(String::as_str).collect();
        relay.lock().unwrap().events.clear();
        seed(&client, &relay, &doc_with_sections(&names));
        let over_count = SidebarSectionsCmd::Create {
            name: "one too many".into(),
            icon: None,
        };
        let rename = |len: usize| SidebarSectionsCmd::Update {
            section: "0".into(),
            name: Some("x".repeat(len)),
            icon: None,
            clear_icon: false,
        };
        let err = run(over_count, &client).await.unwrap_err();
        assert!(matches!(err, CliError::Usage(_)), "{err:?}");

        // Ciphertext over the relay cap, then plaintext over the NIP-44 cap.
        relay.lock().unwrap().events.clear();
        seed(&client, &relay, &doc_with_sections(&["0"]));
        for cmd in [rename(25_000), rename(70_000)] {
            let err = run(cmd, &client).await.unwrap_err();
            assert!(matches!(err, CliError::Usage(_)), "{err:?}");
        }
        assert_eq!(published_count(&relay), 1);

        let err = run(
            SidebarSectionsCmd::Update {
                section: "0".into(),
                name: None,
                icon: Some("  ".into()),
                clear_icon: false,
            },
            &client,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, CliError::Usage(_)), "{err:?}");
    }

    #[tokio::test]
    async fn a_dominated_write_is_a_conflict() {
        let (client, relay) = setup().await;
        relay.lock().unwrap().duplicate = true;
        let err = run(
            SidebarSectionsCmd::Create {
                name: "Late".into(),
                icon: None,
            },
            &client,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, CliError::Conflict(_)), "{err:?}");
    }
}
