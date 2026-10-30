//! `buzz sidebar sort` — how each sidebar group orders its channels, the same
//! synced preference the desktop sidebar edits (kind:30078, d=`channel-sort`,
//! NIP-44 encrypted to yourself).
//!
//! One register per group key, merged per field like the sections document.
//! Every write is one sort-head read and one publish; naming a section also
//! reads the sections document. Desktop never prunes the `section:<id>` key of
//! a deleted section, so neither does this command.

use serde_json::{json, Value};

use super::sidebar::{fetch_view, resolve};
use super::sidebar_doc::{self, Edit, MAX_SORT_GROUPS, SORT};
use super::sidebar_sync::{self, Lane};
use crate::client::BuzzClient;
use crate::error::CliError;
use crate::{SidebarSortCmd, SidebarSortMode};

const LANE: Lane = Lane {
    d_tag: "channel-sort",
    label: "sidebar sort modes",
};

const FIXED_GROUPS: [&str; 4] = ["starred", "channels", "forums", "dms"];
const SECTION_PREFIX: &str = "section:";

fn wire(mode: SidebarSortMode) -> &'static str {
    match mode {
        SidebarSortMode::Alpha => "alpha",
        SidebarSortMode::Recent => "recent",
    }
}

/// The group key desktop stores for `group`. A fixed group name wins over a
/// section of the same name. Setting a mode needs a live section; resetting
/// accepts any `section:<id>` so a deleted section's key can be cleared.
async fn group_key(
    client: &BuzzClient,
    group: &str,
    needs_live_section: bool,
) -> Result<String, CliError> {
    let group = group.trim();
    if FIXED_GROUPS.contains(&group) {
        return Ok(group.to_string());
    }
    if let Some(id) = group.strip_prefix(SECTION_PREFIX) {
        if id.is_empty() {
            return Err(CliError::Usage("section: needs a section id".into()));
        }
        if needs_live_section {
            resolve_section(client, id).await?;
        }
        return Ok(group.to_string());
    }
    let id = resolve_section(client, group).await?;
    Ok(format!("{SECTION_PREFIX}{id}"))
}

async fn resolve_section(client: &BuzzClient, key: &str) -> Result<String, CliError> {
    let view = fetch_view(client).await?;
    Ok(resolve(&view, key)?.id.clone())
}

async fn edit(
    client: &BuzzClient,
    apply: impl FnOnce(&mut Edit) -> Result<(), CliError>,
) -> Result<Value, CliError> {
    sidebar_sync::edit_tree(
        client,
        LANE,
        &SORT,
        |tree| {
            if sidebar_doc::sort_groups(tree).len() <= MAX_SORT_GROUPS {
                return Ok(());
            }
            Err(CliError::Usage(format!(
                "the sidebar keeps at most {MAX_SORT_GROUPS} sort modes; reset unused groups with `buzz sidebar sort reset`"
            )))
        },
        |edit, _| apply(edit),
    )
    .await
}

pub(super) async fn run(cmd: SidebarSortCmd, client: &BuzzClient) -> Result<Value, CliError> {
    match cmd {
        SidebarSortCmd::List => {
            let (_, tree) = sidebar_sync::fetch_tree(client, LANE, &SORT).await?;
            Ok(json!({"default": "alpha", "groups": sidebar_doc::sort_groups(&tree)}))
        }
        SidebarSortCmd::Set { group, mode } => {
            let key = group_key(client, &group, true).await?;
            let mut response = edit(client, |edit| {
                edit.set(&["g", &key], json!(wire(mode)));
                Ok(())
            })
            .await?;
            response["group"] = json!(key);
            Ok(response)
        }
        SidebarSortCmd::Reset { group } => {
            let key = group_key(client, &group, false).await?;
            let mut response = edit(client, |edit| {
                edit.clear(&["g", &key]);
                Ok(())
            })
            .await?;
            response["group"] = json!(key);
            Ok(response)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::sidebar_doc::{Stamp, Tree, SECTIONS};
    use super::super::sidebar_sync::now;
    use super::super::sidebar_sync::test_support::{self, published, published_count, Shared};
    use super::*;

    const DEV: &str = "0123456789abcdef";
    const SECTIONS_LANE: Lane = Lane {
        d_tag: "channel-sections",
        label: "sidebar sections",
    };

    fn seed_sort(client: &BuzzClient, relay: &Shared, plaintext: &str, created_at: u64) -> Value {
        test_support::seed_at(client, relay, LANE, plaintext, created_at)
    }

    /// Seeds a sections document; ids are `id-<name>`, and names listed in
    /// `dead` are deleted.
    fn seed_sections(client: &BuzzClient, relay: &Shared, names: &[&str], dead: &[&str]) {
        let mut tree = Tree::new();
        let stamp = Stamp {
            now_ms: 1,
            dev: DEV.into(),
        };
        let mut edit = Edit::new(&mut tree, &stamp);
        for name in names {
            edit.create_section(&format!("id-{name}"), name, None);
        }
        let mut edit = Edit::new(&mut tree, &stamp);
        for name in dead {
            edit.set(&["s", &format!("id-{name}"), "live"], json!(false));
        }
        let plaintext = sidebar_doc::encode(&SECTIONS, &tree).to_string();
        test_support::seed_at(client, relay, SECTIONS_LANE, &plaintext, 5);
    }

    fn set(group: &str, mode: SidebarSortMode) -> SidebarSortCmd {
        SidebarSortCmd::Set {
            group: group.into(),
            mode,
        }
    }

    fn reset(group: &str) -> SidebarSortCmd {
        SidebarSortCmd::Reset {
            group: group.into(),
        }
    }

    fn groups(list: &Value) -> Value {
        list["groups"].clone()
    }

    #[tokio::test]
    async fn sets_and_resets_group_sort_modes_in_desktop_format() {
        let (client, relay) = test_support::setup().await;
        seed_sections(&client, &relay, &["Eng", "Ops"], &[]);
        assert_eq!(
            run(SidebarSortCmd::List, &client).await.unwrap(),
            json!({"default": "alpha", "groups": {}})
        );

        let before = published_count(&relay);
        let response = run(set("dms", SidebarSortMode::Recent), &client)
            .await
            .unwrap();
        assert_eq!(response["group"], json!("dms"));
        assert_eq!(published_count(&relay), before + 1);
        let (event, doc) = published(&client, &relay);
        assert_eq!(event.kind.as_u16(), 30078);
        assert!(event
            .tags
            .iter()
            .any(|t| t.as_slice() == ["d", "channel-sort"]));
        assert!(event
            .tags
            .iter()
            .any(|t| t.as_slice() == ["t", "channel-sort"]));
        assert_eq!(doc["version"], json!(1));
        assert_eq!(doc["groups"], json!({"dms": "recent"}));
        assert_eq!(doc["meta"]["v"], json!(1));
        assert_eq!(doc["meta"]["g"]["dms"][2], json!("recent"));

        // A section is addressed by name, by id, or by its stored key.
        let by_name = run(set("Eng", SidebarSortMode::Recent), &client)
            .await
            .unwrap();
        assert_eq!(by_name["group"], json!("section:id-Eng"));
        run(set("section:id-Ops", SidebarSortMode::Alpha), &client)
            .await
            .unwrap();
        run(set(" starred ", SidebarSortMode::Recent), &client)
            .await
            .unwrap();
        let list = run(SidebarSortCmd::List, &client).await.unwrap();
        assert_eq!(
            groups(&list),
            json!({
                "dms": "recent", "starred": "recent",
                "section:id-Eng": "recent", "section:id-Ops": "alpha",
            })
        );

        // A repeat publishes nothing; an explicit alpha is a real register.
        let count = published_count(&relay);
        let again = run(set("dms", SidebarSortMode::Recent), &client)
            .await
            .unwrap();
        assert_eq!(again["message"], json!("unchanged"));
        assert_eq!(published_count(&relay), count);

        // A reset is a null register that every projection drops.
        run(reset("dms"), &client).await.unwrap();
        let (_, doc) = published(&client, &relay);
        assert_eq!(doc["meta"]["g"]["dms"][2], Value::Null);
        assert!(doc["groups"].get("dms").is_none());
        let count = published_count(&relay);
        for group in ["dms", "forums"] {
            let response = run(reset(group), &client).await.unwrap();
            assert_eq!(response["message"], json!("unchanged"), "{group}");
        }
        assert_eq!(published_count(&relay), count);
        run(set("dms", SidebarSortMode::Alpha), &client)
            .await
            .unwrap();

        // Bad input fails before any publish.
        let count = published_count(&relay);
        for (group, not_found) in [
            ("Nope", true),
            ("section:", false),
            ("section:missing", true),
        ] {
            let err = run(set(group, SidebarSortMode::Recent), &client)
                .await
                .unwrap_err();
            let expected = if not_found {
                matches!(err, CliError::NotFound(_))
            } else {
                matches!(err, CliError::Usage(_))
            };
            assert!(expected, "{group}: {err:?}");
        }
        assert_eq!(published_count(&relay), count);
    }

    #[tokio::test]
    async fn ambiguous_section_names_are_refused() {
        let (client, relay) = test_support::setup().await;
        let mut tree = Tree::new();
        let stamp = Stamp {
            now_ms: 1,
            dev: DEV.into(),
        };
        let mut edit = Edit::new(&mut tree, &stamp);
        edit.create_section("a", "Same", None);
        edit.create_section("b", "Same", None);
        let plaintext = sidebar_doc::encode(&SECTIONS, &tree).to_string();
        test_support::seed_at(&client, &relay, SECTIONS_LANE, &plaintext, 5);
        let err = run(set("Same", SidebarSortMode::Recent), &client)
            .await
            .unwrap_err();
        assert!(matches!(err, CliError::Usage(_)), "{err:?}");
        assert_eq!(published_count(&relay), 1);
    }

    #[tokio::test]
    async fn a_deleted_sections_key_stays_until_reset() {
        let (client, relay) = test_support::setup().await;
        seed_sections(&client, &relay, &["Old"], &[]);
        run(set("Old", SidebarSortMode::Recent), &client)
            .await
            .unwrap();
        relay
            .lock()
            .unwrap()
            .events
            .retain(|e| e["tags"][0][1] != json!("channel-sections"));
        seed_sections(&client, &relay, &["Old"], &["Old"]);

        // Desktop leaves the register behind and still projects it.
        let list = run(SidebarSortCmd::List, &client).await.unwrap();
        assert_eq!(groups(&list), json!({"section:id-Old": "recent"}));
        let err = run(set("section:id-Old", SidebarSortMode::Alpha), &client)
            .await
            .unwrap_err();
        assert!(matches!(err, CliError::NotFound(_)), "{err:?}");
        let err = run(set("Old", SidebarSortMode::Alpha), &client)
            .await
            .unwrap_err();
        assert!(matches!(err, CliError::NotFound(_)), "{err:?}");

        run(reset("section:id-Old"), &client).await.unwrap();
        let list = run(SidebarSortCmd::List, &client).await.unwrap();
        assert_eq!(groups(&list), json!({}));
    }

    #[tokio::test]
    async fn keeps_a_mobile_head_and_never_overwrites_an_unreadable_one() {
        let (client, relay) = test_support::setup().await;
        seed_sort(
            &client,
            &relay,
            r#"{"version":1,"groups":{"dms":"recent","stale":"alpha","bad":"newest"}}"#,
            now().as_secs(),
        );
        let list = run(SidebarSortCmd::List, &client).await.unwrap();
        assert_eq!(groups(&list), json!({"dms": "recent", "stale": "alpha"}));
        run(set("forums", SidebarSortMode::Recent), &client)
            .await
            .unwrap();
        let (_, doc) = published(&client, &relay);
        assert_eq!(
            doc["groups"],
            json!({"dms": "recent", "forums": "recent", "stale": "alpha"})
        );

        seed_sort(
            &client,
            &relay,
            r#"{"version":1,"meta":{"v":2}}"#,
            now().as_secs() + 100,
        );
        let count = published_count(&relay);
        assert!(run(SidebarSortCmd::List, &client).await.is_err());
        assert!(run(set("dms", SidebarSortMode::Alpha), &client)
            .await
            .is_err());
        assert!(run(reset("dms"), &client).await.is_err());
        assert_eq!(published_count(&relay), count);

        // An unreadable sections document blocks only a section lookup.
        relay.lock().unwrap().events.clear();
        test_support::seed_at(
            &client,
            &relay,
            SECTIONS_LANE,
            r#"{"version":1,"meta":{"v":2}}"#,
            5,
        );
        assert!(run(set("Eng", SidebarSortMode::Recent), &client)
            .await
            .is_err());
        run(set("dms", SidebarSortMode::Recent), &client)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn an_edit_is_newer_than_the_head_and_its_registers() {
        let (client, relay) = test_support::setup().await;
        let future = now().as_secs() + 1_000;
        let far_v = (future as i64 + 5_000) * 1_000;
        let doc = json!({
            "version": 1, "groups": {"dms": "recent"},
            "meta": {"v": 1, "g": {"dms": [far_v, DEV, "recent"]}},
        });
        seed_sort(&client, &relay, &doc.to_string(), future);

        run(set("dms", SidebarSortMode::Alpha), &client)
            .await
            .unwrap();
        let (event, doc) = published(&client, &relay);
        assert_eq!(event.created_at.as_secs(), future + 1);
        assert_eq!(doc["meta"]["g"]["dms"][0], json!(far_v + 1));

        // Another author's head is never read as ours.
        let other = BuzzClient::new(String::new(), nostr::Keys::generate(), None, None).unwrap();
        seed_sort(
            &other,
            &relay,
            r#"{"version":1,"groups":{"dms":"recent"}}"#,
            future + 9,
        );
        let list = run(SidebarSortCmd::List, &client).await.unwrap();
        assert_eq!(groups(&list), json!({"dms": "alpha"}));
    }

    #[tokio::test]
    async fn enforces_the_group_cap_and_refuses_bad_heads() {
        let (client, relay) = test_support::setup().await;
        let full: Value = (0..MAX_SORT_GROUPS)
            .map(|i| (format!("section:{i}"), json!("recent")))
            .collect::<serde_json::Map<_, _>>()
            .into();
        seed_sort(
            &client,
            &relay,
            &json!({"version": 1, "groups": full}).to_string(),
            now().as_secs(),
        );
        let count = published_count(&relay);
        let err = run(set("dms", SidebarSortMode::Recent), &client)
            .await
            .unwrap_err();
        assert!(matches!(err, CliError::Usage(_)), "{err:?}");
        assert_eq!(published_count(&relay), count);

        // Resetting a stale key makes room; a change at the cap still works.
        run(reset("section:0"), &client).await.unwrap();
        run(set("dms", SidebarSortMode::Recent), &client)
            .await
            .unwrap();
        assert_eq!(published_count(&relay), count + 2);

        let mut head = test_support::seed_at(
            &client,
            &relay,
            LANE,
            r#"{"version":1,"groups":{}}"#,
            now().as_secs() + 50,
        );
        head["content"] = json!("tampered");
        relay
            .lock()
            .unwrap()
            .events
            .retain(|e| e["tags"][0][1] != json!("channel-sort"));
        relay.lock().unwrap().events.push(head);
        let count = published_count(&relay);
        let err = run(set("dms", SidebarSortMode::Recent), &client)
            .await
            .unwrap_err();
        assert!(matches!(err, CliError::Other(_)), "{err:?}");
        assert_eq!(published_count(&relay), count);
    }

    #[tokio::test]
    async fn a_dominated_write_is_a_conflict() {
        let (client, relay) = test_support::setup().await;
        relay.lock().unwrap().duplicate = true;
        let err = run(set("dms", SidebarSortMode::Recent), &client)
            .await
            .unwrap_err();
        assert!(matches!(err, CliError::Conflict(_)), "{err:?}");
    }
}
