//! `buzz sidebar sections` — your own channel sections, the same synced
//! preference the desktop sidebar edits (kind:30078, d=`channel-sections`,
//! NIP-44 encrypted to yourself).
//!
//! Every write is one read of the current head followed by one publish, so a
//! multi-channel `move` lands atomically. A head this CLI cannot read is never
//! overwritten.

use serde_json::{json, Value};

use super::sidebar_doc::{self, Edit, Projection, Section, Tree};
use super::sidebar_sync::{self, channel_ids, Lane};
use super::{sidebar_flags, sidebar_sort};
use crate::client::BuzzClient;
use crate::error::CliError;
use crate::{SidebarCmd, SidebarSectionsCmd};

const LANE: Lane = Lane {
    d_tag: "channel-sections",
    label: "sidebar sections",
};

async fn fetch_head(client: &BuzzClient) -> Result<Tree, CliError> {
    Ok(
        sidebar_sync::fetch_tree(client, LANE, &sidebar_doc::SECTIONS)
            .await?
            .1,
    )
}

/// The live sections and assignments in your current sections document.
pub(super) async fn fetch_view(client: &BuzzClient) -> Result<Projection, CliError> {
    Ok(sidebar_doc::project(&fetch_head(client).await?))
}

/// A live section by exact id, or by exact name when that name is unique.
pub(super) fn resolve<'a>(view: &'a Projection, key: &str) -> Result<&'a Section, CliError> {
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

/// Reads the head, applies `apply`, and publishes the result once. Returns
/// the write response, or an `unchanged` response without publishing.
async fn edit(
    client: &BuzzClient,
    apply: impl FnOnce(&mut Edit, &Projection) -> Result<(), CliError>,
) -> Result<Value, CliError> {
    sidebar_sync::edit_tree(
        client,
        LANE,
        &sidebar_doc::SECTIONS,
        |tree| {
            if sidebar_doc::project(tree).within_limits() {
                return Ok(());
            }
            Err(CliError::Usage(format!(
                "the sidebar supports at most {} sections and {} channel assignments",
                sidebar_doc::MAX_SECTIONS,
                sidebar_doc::MAX_ASSIGNMENTS
            )))
        },
        |edit, head| apply(edit, &sidebar_doc::project(head)),
    )
    .await
}

async fn run(cmd: SidebarSectionsCmd, client: &BuzzClient) -> Result<Value, CliError> {
    match cmd {
        SidebarSectionsCmd::List => {
            let tree = fetch_head(client).await?;
            serde_json::to_value(sidebar_doc::project(&tree))
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
                    edit.clear(&["a", channel]);
                }
                Ok(())
            })
            .await
        }
    }
}

pub async fn dispatch(cmd: SidebarCmd, client: &BuzzClient) -> Result<(), CliError> {
    let output = match cmd {
        SidebarCmd::Sections(cmd) => run(cmd, client).await?,
        SidebarCmd::Sort(cmd) => sidebar_sort::run(cmd, client).await?,
        SidebarCmd::Stars(cmd) => {
            sidebar_flags::run(&sidebar_flags::STARS, cmd.into(), client).await?
        }
        SidebarCmd::Mutes(cmd) => {
            sidebar_flags::run(&sidebar_flags::MUTES, cmd.into(), client).await?
        }
        SidebarCmd::Projects(cmd) => {
            sidebar_flags::run(&sidebar_flags::PROJECTS, cmd.into(), client).await?
        }
    };
    println!("{output}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::sidebar_doc::Stamp;
    use super::super::sidebar_sync::now;
    use super::super::sidebar_sync::test_support::{self, Shared};
    use super::*;
    use nostr::Keys;

    async fn setup() -> (BuzzClient, Shared) {
        test_support::setup().await
    }

    fn seed_at(client: &BuzzClient, relay: &Shared, plaintext: &str, created_at: u64) -> Value {
        test_support::seed_at(client, relay, LANE, plaintext, created_at)
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
        sidebar_doc::encode(&sidebar_doc::SECTIONS, &tree).to_string()
    }

    use test_support::{published, published_count};

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
        assert_eq!(event.kind.as_u16(), 30078);
        assert!(event.tags.iter().any(|t| t.as_slice() == ["d", LANE.d_tag]));
        assert!(event.tags.iter().any(|t| t.as_slice() == ["t", LANE.d_tag]));
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
