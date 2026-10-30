//! Read-only team catalogs: `buzz teams list` (the owner's kind:30176 team
//! definitions) and `buzz teams catalog` (the community's shared kind:30178
//! team projections with embedded members).

use buzz_core::kind::{event_is_shared, KIND_TEAM, KIND_TEAM_CATALOG};
use serde_json::{json, Value};

use super::catalog::{fetch_heads, optional_string, safe_avatar, warn_skipped, Head};
use crate::client::BuzzClient;
use crate::error::CliError;
use crate::TeamsCmd;

const TEAM_CATALOG_SCHEMA_VERSION: u64 = 1;
const MAX_CATALOG_MEMBERS: usize = 64;

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

fn project_member(member: &Value) -> Option<Value> {
    let display_name = member.get("display_name")?.as_str()?;
    let member_key = member.get("member_key")?.as_str()?;
    if display_name.trim().is_empty() || member_key.is_empty() {
        return None;
    }
    Some(json!({
        "member_key": member_key,
        "display_name": display_name,
        "system_prompt": member.get("system_prompt").and_then(Value::as_str).unwrap_or_default(),
        "avatar_url": safe_avatar(member.get("avatar_url")),
        "runtime": optional_string(member.get("runtime")),
        "model": optional_string(member.get("model")),
        "provider": optional_string(member.get("provider")),
    }))
}

/// All-or-nothing, like desktop: one invalid member rejects the whole team.
fn project_catalog_team(head: &Head) -> Option<Value> {
    let content: Value = serde_json::from_str(&head.event.content).ok()?;
    if content.get("v")?.as_u64()? != TEAM_CATALOG_SCHEMA_VERSION {
        return None;
    }
    let name = content.get("name")?.as_str()?;
    let members = content.get("members")?.as_array()?;
    if name.trim().is_empty() || members.len() > MAX_CATALOG_MEMBERS {
        return None;
    }
    let members = members
        .iter()
        .map(project_member)
        .collect::<Option<Vec<_>>>()?;
    Some(json!({
        "event_id": head.event.id.to_hex(),
        "owner_pubkey": head.event.pubkey.to_hex(),
        "team_id": head.d_tag,
        "created_at": head.event.created_at.as_secs(),
        "name": name,
        "description": optional_string(content.get("description")),
        "instructions": optional_string(content.get("instructions")),
        "members": members,
    }))
}

fn sort_by_name(teams: &mut [Value]) {
    teams.sort_by_cached_key(|team| {
        (
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

        let teams = catalog(&client).await.unwrap();
        let teams = teams.as_array().unwrap();
        assert_eq!(teams.len(), 1, "{teams:?}");
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
