//! Read-only team catalogs: `buzz teams list` (the owner's kind:30176 team
//! definitions) and `buzz teams catalog` (the community's shared kind:30178
//! team projections with embedded members).

use buzz_core::kind::{event_is_shared, KIND_TEAM, KIND_TEAM_CATALOG};
use buzz_core::team_catalog::parse_team_catalog_content;
use serde_json::{json, Value};

use super::catalog::{
    fetch_heads, optional_string, safe_avatar, sanitize_validation_error, warn_skipped, Head,
    VALIDATION_ERROR_KEY,
};
use crate::client::BuzzClient;
use crate::error::CliError;
use crate::TeamsCmd;

/// A team event's `persona_ids`: `None` when the publisher predates
/// always-publish (membership unknown), otherwise the string entries.
pub(crate) fn persona_ids_from_content(content: &Value) -> Option<Vec<String>> {
    content
        .get("persona_ids")
        .and_then(Value::as_array)
        .map(|ids| {
            ids.iter()
                .filter_map(|id| id.as_str().map(str::to_string))
                .collect()
        })
}

fn project_team(head: &Head) -> Option<Value> {
    let content: Value = serde_json::from_str(&head.event.content).ok()?;
    let name = content.get("name")?.as_str()?;
    Some(json!({
        "event_id": head.event.id.to_hex(),
        "owner_pubkey": head.event.pubkey.to_hex(),
        "team_id": head.d_tag,
        "created_at": head.event.created_at.as_secs(),
        "name": name,
        "description": optional_string(content.get("description")),
        "instructions": optional_string(content.get("instructions")),
        "persona_ids": persona_ids_from_content(&content),
    }))
}

fn non_blank(value: &Option<String>) -> Option<&str> {
    value.as_deref().filter(|value| !value.trim().is_empty())
}

/// `None` only when the content is not a JSON object. A team Desktop would
/// refuse to adopt (one invalid member rejects the whole team) is listed with
/// its `validation_error` and no content, so rejected text never reaches an
/// agent.
fn project_catalog_team(head: &Head) -> Option<Value> {
    let raw: Value = serde_json::from_str(&head.event.content).ok()?;
    raw.as_object()?;
    let mut team = json!({
        "event_id": head.event.id.to_hex(),
        "owner_pubkey": head.event.pubkey.to_hex(),
        "team_id": head.d_tag,
        "created_at": head.event.created_at.as_secs(),
    });
    let content = match parse_team_catalog_content(&head.event.content) {
        Ok(content) => content,
        Err(error) => {
            team[VALIDATION_ERROR_KEY] = Value::String(sanitize_validation_error(&error));
            return Some(team);
        }
    };
    let members: Vec<Value> = content
        .members
        .iter()
        .map(|member| {
            json!({
                "member_key": member.member_key,
                "display_name": member.display_name,
                "system_prompt": member.system_prompt.as_deref().unwrap_or_default(),
                "avatar_url": safe_avatar(member.avatar_url.as_deref()),
                "runtime": non_blank(&member.runtime),
                "model": non_blank(&member.model),
                "provider": non_blank(&member.provider),
            })
        })
        .collect();
    team["name"] = json!(content.name);
    team["description"] = json!(non_blank(&content.description));
    team["instructions"] = json!(non_blank(&content.instructions));
    team["members"] = json!(members);
    team[VALIDATION_ERROR_KEY] = Value::Null;
    Some(team)
}

fn sort_by_name(teams: &mut [Value]) {
    teams.sort_by_cached_key(|team| {
        (
            !team[VALIDATION_ERROR_KEY].is_null(),
            team["name"].as_str().unwrap_or("").to_lowercase(),
            team["owner_pubkey"].as_str().unwrap_or("").to_owned(),
            team["team_id"].as_str().unwrap_or("").to_owned(),
        )
    });
}

fn project_all(
    heads: &[Head],
    project: fn(&Head) -> Option<Value>,
    shared_only: bool,
) -> Vec<Value> {
    let mut skipped = Vec::new();
    let mut teams: Vec<Value> = Vec::new();
    for head in heads
        .iter()
        .filter(|head| !shared_only || event_is_shared(&head.event))
    {
        match project(head) {
            Some(team) => teams.push(team),
            None => skipped.push(format!("{}:{}", head.event.pubkey.to_hex(), head.d_tag)),
        }
    }
    warn_skipped("teams", &skipped);
    sort_by_name(&mut teams);
    teams
}

pub(crate) fn project_team_heads(heads: &[Head]) -> Vec<Value> {
    project_all(heads, project_team, false)
}

pub(crate) fn project_catalog_heads(heads: &[Head]) -> Vec<Value> {
    project_all(heads, project_catalog_team, true)
}

async fn list(client: &BuzzClient) -> Result<Value, CliError> {
    let owner = client
        .auth_tag_owner_hex()
        .unwrap_or_else(|| client.keys().public_key().to_hex());
    let heads = fetch_heads(client, KIND_TEAM, Some(&owner)).await?;
    Ok(Value::Array(project_team_heads(&heads)))
}

async fn catalog(client: &BuzzClient) -> Result<Value, CliError> {
    let heads = fetch_heads(client, KIND_TEAM_CATALOG, None).await?;
    Ok(Value::Array(project_catalog_heads(&heads)))
}

pub async fn dispatch(cmd: TeamsCmd, client: &BuzzClient) -> Result<(), CliError> {
    let output = match cmd {
        TeamsCmd::List => list(client).await?,
        TeamsCmd::Catalog => catalog(client).await?,
    };
    println!("{output}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::catalog::test_support::*;
    use nostr::Keys;

    #[tokio::test]
    async fn lists_only_the_owners_teams_and_keeps_unknown_membership_unknown() {
        let (client, relay) = setup().await;
        let me = client.keys().clone();
        let other = Keys::generate();
        let team = |content: Value| content.to_string();

        seed(
            &relay,
            event(
                &me,
                30176,
                &team(json!({"name": "Zeta", "persona_ids": []})),
                &[&["d", "z"]],
                10,
            ),
        );
        seed(
            &relay,
            event(
                &me,
                30176,
                &team(
                    json!({"name": "Alpha", "description": "d", "instructions": null, "persona_ids": ["p1", 7, "p2"]}),
                ),
                &[&["d", "a"]],
                10,
            ),
        );
        seed(
            &relay,
            event(
                &me,
                30176,
                &team(json!({"name": "Legacy"})),
                &[&["d", "l"]],
                10,
            ),
        );
        seed(&relay, event(&me, 30176, "garbage", &[&["d", "bad"]], 10));
        seed(
            &relay,
            event(
                &other,
                30176,
                &team(json!({"name": "Theirs"})),
                &[&["d", "t"]],
                10,
            ),
        );

        let teams = list(&client).await.unwrap();
        let names: Vec<&str> = teams
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["Alpha", "Legacy", "Zeta"]);
        assert_eq!(teams[0]["persona_ids"], json!(["p1", "p2"]));
        assert_eq!(teams[0]["description"], "d");
        assert_eq!(teams[0]["instructions"], Value::Null);
        assert_eq!(
            teams[1]["persona_ids"],
            Value::Null,
            "absent stays unknown, not empty"
        );
        assert_eq!(teams[2]["persona_ids"], json!([]));
        assert_eq!(teams[0]["owner_pubkey"], me.public_key().to_hex());
        assert!(every_filter_names_kinds(&relay));
    }

    fn member(key: &str) -> Value {
        json!({"member_key": key, "display_name": format!("M {key}"), "system_prompt": "sp", "avatar_url": "https://x.test/a.png", "runtime": "goose"})
    }

    #[tokio::test]
    async fn catalog_lists_shared_teams_and_rejects_invalid_ones_whole() {
        let (client, relay) = setup().await;
        let alice = Keys::generate();
        let shared = ["shared", "true"];
        let body = |v: u64, members: Vec<Value>| {
            json!({"v": v, "name": "Team", "description": "dd", "instructions": "ii", "members": members}).to_string()
        };

        seed(
            &relay,
            event(
                &alice,
                30178,
                &body(1, vec![member("k1"), member("k2")]),
                &[&["d", "ok"], &shared],
                10,
            ),
        );
        seed(
            &relay,
            event(
                &alice,
                30178,
                &body(1, vec![member("k1")]),
                &[&["d", "private"]],
                10,
            ),
        );
        seed(
            &relay,
            event(
                &alice,
                30178,
                &body(2, vec![member("k1")]),
                &[&["d", "future"], &shared],
                10,
            ),
        );
        let bad_member = json!({"member_key": "", "display_name": "x"});
        seed(
            &relay,
            event(
                &alice,
                30178,
                &body(1, vec![member("k1"), bad_member]),
                &[&["d", "badmember"], &shared],
                10,
            ),
        );
        let too_many: Vec<Value> = (0..65).map(|i| member(&format!("k{i}"))).collect();
        seed(
            &relay,
            event(
                &alice,
                30178,
                &body(1, too_many),
                &[&["d", "big"], &shared],
                10,
            ),
        );
        // Unshared newest head hides the older shared one.
        seed(
            &relay,
            event(
                &alice,
                30178,
                &body(1, vec![member("k1")]),
                &[&["d", "retracted"], &shared],
                5,
            ),
        );
        seed(
            &relay,
            event(
                &alice,
                30178,
                &body(1, vec![member("k1")]),
                &[&["d", "retracted"]],
                6,
            ),
        );

        let concealed = json!({"member_key": "k9", "display_name": "Re\u{200B}v"});
        seed(
            &relay,
            event(
                &alice,
                30178,
                &body(1, vec![concealed]),
                &[&["d", "concealed"], &shared],
                10,
            ),
        );
        let long_v =
            json!({"v": format!("{}\u{202E}", "A".repeat(50_000)), "name": "T", "members": []});
        seed(
            &relay,
            event(
                &alice,
                30178,
                &long_v.to_string(),
                &[&["d", "longv"], &shared],
                10,
            ),
        );
        let dup = json!({"member_key": "k\u{202E}x", "display_name": "D"});
        seed(
            &relay,
            event(
                &alice,
                30178,
                &body(1, vec![dup.clone(), dup]),
                &[&["d", "dupkey"], &shared],
                10,
            ),
        );
        seed(
            &relay,
            event(&alice, 30178, "not json", &[&["d", "broken"], &shared], 10),
        );

        let teams = catalog(&client).await.unwrap();
        let teams = teams.as_array().unwrap();
        assert_eq!(teams.len(), 7, "{teams:?}");
        let rejected: Vec<(&str, &str)> = teams[1..]
            .iter()
            .map(|t| {
                (
                    t["team_id"].as_str().unwrap(),
                    t["validation_error"].as_str().unwrap(),
                )
            })
            .collect();
        assert_eq!(rejected.len(), 6);
        for (_, error) in &rejected {
            assert!(error.is_ascii() && error.chars().count() <= 203, "{error}");
        }
        for (id, fragment) in [
            ("badmember", "a member key is empty"),
            ("big", "65 members"),
            ("concealed", "prohibited invisible"),
            ("future", "unsupported team catalog schema version 2"),
            ("longv", "failed to parse"),
            ("dupkey", "repeats the member key 'k\\u{202e}x'"),
        ] {
            assert!(
                rejected
                    .iter()
                    .any(|(t, e)| *t == id && e.contains(fragment)),
                "{id}: {rejected:?}"
            );
        }
        for team in &teams[1..] {
            for key in ["name", "description", "instructions", "members"] {
                assert!(team.get(key).is_none(), "{key} leaked: {team}");
            }
        }
        assert_eq!(teams[0]["validation_error"], Value::Null);
        assert_eq!(teams[0]["team_id"], "ok");
        assert_eq!(teams[0]["owner_pubkey"], alice.public_key().to_hex());
        assert_eq!(teams[0]["instructions"], "ii");
        assert_eq!(teams[0]["members"].as_array().unwrap().len(), 2);
        assert_eq!(teams[0]["members"][0]["member_key"], "k1");
        assert_eq!(teams[0]["members"][0]["avatar_url"], "https://x.test/a.png");
        assert_eq!(teams[0]["members"][0]["runtime"], "goose");
        assert_eq!(teams[0]["members"][0]["model"], Value::Null);
        assert!(every_filter_names_kinds(&relay));
    }
}
