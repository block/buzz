//! `buzz personas list` — the community's shared persona catalog (kind:30175
//! heads carrying `["shared","true"]`), projected the way desktop's catalog
//! browser reads them.

use buzz_core::definition_validation::validate_persona_object;
use buzz_core::kind::{event_is_shared, KIND_PERSONA};
use serde_json::{json, Value};

use super::catalog::{
    fetch_heads, optional_string, safe_avatar, warn_skipped, Head, VALIDATION_ERROR_KEY,
};
use crate::client::BuzzClient;
use crate::error::CliError;
use crate::PersonasCmd;

fn identity(head: &Head) -> Value {
    json!({
        "event_id": head.event.id.to_hex(),
        "owner_pubkey": head.event.pubkey.to_hex(),
        "persona_id": head.d_tag,
        "created_at": head.event.created_at.as_secs(),
    })
}

/// `None` only when the content is unreadable. Content Desktop would refuse to
/// adopt is listed with its `validation_error` and no content, so rejected text
/// never reaches an agent.
fn project(head: &Head) -> Option<Value> {
    let content: Value = serde_json::from_str(&head.event.content).ok()?;
    let object = content.as_object()?;
    let display_name = object.get("display_name")?.as_str()?;
    let mut persona = identity(head);
    if let Err(error) = validate_persona_object(object) {
        persona[VALIDATION_ERROR_KEY] = Value::String(error);
        return Some(persona);
    }
    let respond_to = match object.get("respond_to").and_then(Value::as_str) {
        Some("allowlist") => Some("owner-only"),
        Some(value @ ("owner-only" | "anyone")) => Some(value),
        _ => None,
    };
    let parallelism = object
        .get("parallelism")
        .and_then(Value::as_u64)
        .filter(|value| (1..=32).contains(value));
    let session_policy = match object.get("session_policy").and_then(Value::as_str) {
        Some("thread") => "thread",
        _ => "channel",
    };
    let name_pool: Vec<&str> = object
        .get("name_pool")
        .and_then(Value::as_array)
        .map(|values| values.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    persona["display_name"] = json!(display_name);
    persona["description"] = json!(optional_string(object.get("description")));
    persona["system_prompt"] = json!(object
        .get("system_prompt")
        .and_then(Value::as_str)
        .unwrap_or_default());
    persona["avatar_url"] = json!(safe_avatar(
        object.get("avatar_url").and_then(Value::as_str)
    ));
    persona["runtime"] = json!(optional_string(object.get("runtime")));
    persona["acp_command"] = json!(object.get("acp_command").and_then(Value::as_str));
    persona["model"] = json!(optional_string(object.get("model")));
    persona["provider"] = json!(optional_string(object.get("provider")));
    persona["name_pool"] = json!(name_pool);
    persona["respond_to"] = json!(respond_to);
    persona["parallelism"] = json!(parallelism);
    persona["session_policy"] = json!(session_policy);
    persona[VALIDATION_ERROR_KEY] = Value::Null;
    Some(persona)
}

pub(crate) fn project_heads(heads: &[Head]) -> Vec<Value> {
    let mut skipped = Vec::new();
    let mut personas: Vec<Value> = Vec::new();
    // A newest unshared or malformed head still owns its coordinate, so it
    // hides any older shared definition rather than resurrecting it.
    for head in heads.iter().filter(|head| event_is_shared(&head.event)) {
        match project(head) {
            Some(persona) => personas.push(persona),
            None => skipped.push(format!("{}:{}", head.event.pubkey.to_hex(), head.d_tag)),
        }
    }
    warn_skipped("personas", &skipped);
    personas.sort_by_cached_key(|persona| {
        (
            !persona[VALIDATION_ERROR_KEY].is_null(),
            persona["display_name"]
                .as_str()
                .unwrap_or("")
                .to_lowercase(),
            persona["owner_pubkey"].as_str().unwrap_or("").to_owned(),
            persona["persona_id"].as_str().unwrap_or("").to_owned(),
        )
    });
    personas
}

async fn list(client: &BuzzClient) -> Result<Value, CliError> {
    let heads = fetch_heads(client, KIND_PERSONA, None).await?;
    Ok(Value::Array(project_heads(&heads)))
}

pub async fn dispatch(cmd: PersonasCmd, client: &BuzzClient) -> Result<(), CliError> {
    match cmd {
        PersonasCmd::List => println!("{}", list(client).await?),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::catalog::test_support::*;
    use nostr::Keys;

    fn persona(name: &str) -> String {
        json!({"display_name": name, "system_prompt": format!("You are {name}.")}).to_string()
    }

    #[tokio::test]
    async fn lists_only_shared_heads_and_pages_past_one_relay_page() {
        let (client, relay) = setup().await;
        let alice = Keys::generate();
        let bob = Keys::generate();
        let shared = ["shared", "true"];

        let rich = json!({
            "display_name": "Zed",
            "system_prompt": "p",
            "description": "  ",
            "avatar_url": "data:image/png;base64,AAAA",
            "runtime": "goose",
            "model": " ",
            "respond_to": "allowlist",
            "parallelism": 99,
            "session_policy": "thread",
            "name_pool": ["a", 1, "b"],
        })
        .to_string();
        seed(
            &relay,
            event(&alice, 30175, &rich, &[&["d", "rich"], &shared], 50),
        );

        // Newest head is unshared: it hides the older shared definition.
        seed(
            &relay,
            event(
                &alice,
                30175,
                &persona("Old"),
                &[&["d", "hidden"], &shared],
                10,
            ),
        );
        seed(
            &relay,
            event(&alice, 30175, &persona("Old"), &[&["d", "hidden"]], 20),
        );
        // Malformed shared head: skipped, and it still hides older ones.
        seed(
            &relay,
            event(&bob, 30175, "not json", &[&["d", "broken"], &shared], 30),
        );
        seed(
            &relay,
            event(
                &bob,
                30175,
                &persona("Hid"),
                &[&["d", "broken"], &shared],
                5,
            ),
        );
        // Ambiguous tags are never shared / never a coordinate.
        seed(
            &relay,
            event(
                &bob,
                30175,
                &persona("Twice"),
                &[&["d", "t"], &shared, &shared],
                40,
            ),
        );
        seed(
            &relay,
            event(
                &bob,
                30175,
                &persona("TwoD"),
                &[&["d", "a"], &["d", "b"], &shared],
                40,
            ),
        );
        // Forged: content changed after signing.
        let mut forged = event(
            &bob,
            30175,
            &persona("Real"),
            &[&["d", "forged"], &shared],
            60,
        );
        forged["content"] = json!(persona("Forged"));
        seed(&relay, forged);

        // Desktop would refuse these: concealed text, a machine-path command,
        // and a non-string description. They are listed with the reason and
        // none of their content.
        for (id, content) in [
            ("concealed", json!({"display_name": "Re\u{200B}v"})),
            (
                "path",
                json!({"display_name": "Path", "acp_command": "/bin/sh"}),
            ),
            ("typed", json!({"display_name": "Typed", "description": 5})),
        ] {
            seed(
                &relay,
                event(
                    &bob,
                    30175,
                    &content.to_string(),
                    &[&["d", id], &shared],
                    70,
                ),
            );
        }

        // Enough extra shared personas to need a second relay page.
        for i in 0..520 {
            let name = format!("Bulk {i:04}");
            seed(
                &relay,
                event(
                    &bob,
                    30175,
                    &persona(&name),
                    &[&["d", &name], &shared],
                    1_000 + i,
                ),
            );
        }

        let list = list(&client).await.unwrap();
        let list = list.as_array().unwrap();
        assert_eq!(list.len(), 524, "{}", list.len());
        let rejected = &list[521..];
        for (id, fragment) in [
            ("concealed", "prohibited invisible"),
            ("path", "ACP command must be"),
            ("typed", "description must be a string"),
        ] {
            let entry = rejected
                .iter()
                .find(|p| p["persona_id"] == id)
                .unwrap_or_else(|| panic!("{id} missing"));
            assert!(entry["validation_error"]
                .as_str()
                .unwrap()
                .contains(fragment));
            for key in ["display_name", "system_prompt", "acp_command"] {
                assert!(entry.get(key).is_none(), "{key} leaked: {entry}");
            }
        }
        assert!(list[..521]
            .iter()
            .all(|p| p["validation_error"] == Value::Null));
        let rich = list[..521]
            .iter()
            .find(|p| p["persona_id"] == "rich")
            .unwrap();
        assert_eq!(rich["display_name"], "Zed");
        assert_eq!(rich["persona_id"], "rich");
        assert_eq!(rich["owner_pubkey"], alice.public_key().to_hex());
        assert_eq!(rich["description"], Value::Null);
        assert_eq!(rich["avatar_url"], Value::Null);
        assert_eq!(rich["model"], Value::Null);
        assert_eq!(rich["runtime"], "goose");
        assert_eq!(rich["respond_to"], "owner-only");
        assert_eq!(rich["parallelism"], Value::Null);
        assert_eq!(rich["session_policy"], "thread");
        assert_eq!(rich["name_pool"], json!(["a", "b"]));
        assert_eq!(list[0]["display_name"], "Bulk 0000");
        let names: Vec<&str> = list
            .iter()
            .filter_map(|p| p["display_name"].as_str())
            .collect();
        for absent in ["Old", "Hid", "Twice", "TwoD", "Real", "Forged"] {
            assert!(!names.contains(&absent), "{absent} must not be listed");
        }
        assert!(every_filter_names_kinds(&relay));
        assert!(relay.lock().unwrap().filters.len() >= 2, "paged");
    }
}
