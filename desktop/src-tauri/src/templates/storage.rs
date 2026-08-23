use std::{fs, path::PathBuf};

use tauri::AppHandle;
use tauri::Manager;
use uuid::Uuid;

use crate::templates::ChannelTemplateRecord;
use crate::util::now_iso;

fn channel_templates_base_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("app data dir: {error}"))?
        .join("templates");
    fs::create_dir_all(&dir).map_err(|error| format!("failed to create templates dir: {error}"))?;
    Ok(dir)
}

fn channel_templates_store_path(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(channel_templates_base_dir(app)?.join("channel-templates.json"))
}

pub fn sort_channel_templates(records: &mut [ChannelTemplateRecord]) {
    records.sort_by(|left, right| {
        let left_builtin = if left.is_builtin { 0 } else { 1 };
        let right_builtin = if right.is_builtin { 0 } else { 1 };
        left_builtin
            .cmp(&right_builtin)
            .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
            .then_with(|| left.id.cmp(&right.id))
    });
}

fn migrate_missing_identity_fields(
    records: &mut [ChannelTemplateRecord],
    migration_timestamp: &str,
) -> bool {
    let mut migrated = false;
    for record in records {
        if record.id.trim().is_empty() {
            record.id = Uuid::new_v4().to_string();
            migrated = true;
        }
        if record.created_at.trim().is_empty() {
            record.created_at = migration_timestamp.to_string();
            migrated = true;
        }
        if record.updated_at.trim().is_empty() {
            record.updated_at.clone_from(&record.created_at);
            migrated = true;
        }
    }
    migrated
}

pub fn load_channel_templates(app: &AppHandle) -> Result<Vec<ChannelTemplateRecord>, String> {
    let path = channel_templates_store_path(app)?;

    let mut records = if path.exists() {
        let content = fs::read_to_string(&path)
            .map_err(|error| format!("failed to read channel templates store: {error}"))?;
        serde_json::from_str::<Vec<ChannelTemplateRecord>>(&content)
            .map_err(|error| format!("failed to parse channel templates store: {error}"))?
    } else {
        Vec::new()
    };

    if migrate_missing_identity_fields(&mut records, &now_iso()) {
        save_channel_templates(app, &records)?;
    }

    sort_channel_templates(&mut records);
    Ok(records)
}

pub fn save_channel_templates(
    app: &AppHandle,
    records: &[ChannelTemplateRecord],
) -> Result<(), String> {
    let mut sorted = records.to_vec();
    sort_channel_templates(&mut sorted);

    let path = channel_templates_store_path(app)?;
    let payload = serde_json::to_vec_pretty(&sorted)
        .map_err(|error| format!("failed to serialize channel templates store: {error}"))?;
    fs::write(&path, payload)
        .map_err(|error| format!("failed to write channel templates store: {error}"))
}

pub fn validate_channel_template_deletion(template: &ChannelTemplateRecord) -> Result<(), String> {
    if template.is_builtin {
        return Err("Built-in templates cannot be deleted.".to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{migrate_missing_identity_fields, sort_channel_templates};
    use crate::templates::{
        validate_channel_template_deletion, ChannelTemplateRecord, TemplateAgentRoster,
    };

    fn template(id: &str, name: &str) -> ChannelTemplateRecord {
        ChannelTemplateRecord {
            id: id.to_string(),
            name: name.to_string(),
            description: None,
            channel_type: "stream".to_string(),
            visibility: "open".to_string(),
            canvas_template: None,
            agents: TemplateAgentRoster::default(),
            members: Vec::new(),
            is_builtin: false,
            created_at: "2026-05-11T00:00:00Z".to_string(),
            updated_at: "2026-05-11T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn sort_alphabetical_case_insensitive() {
        let mut templates = vec![
            template("3", "Zulu"),
            template("1", "alpha"),
            template("2", "Bravo"),
        ];
        sort_channel_templates(&mut templates);

        let names: Vec<&str> = templates.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, vec!["alpha", "Bravo", "Zulu"]);
    }

    #[test]
    fn sort_builtin_first() {
        let mut builtin = template("b", "Zebra");
        builtin.is_builtin = true;
        let custom = template("a", "Apple");

        let mut templates = vec![custom, builtin];
        sort_channel_templates(&mut templates);

        assert!(templates[0].is_builtin);
        assert_eq!(templates[0].name, "Zebra");
        assert_eq!(templates[1].name, "Apple");
    }

    #[test]
    fn sort_breaks_ties_by_id() {
        let mut templates = vec![template("b", "same"), template("a", "same")];
        sort_channel_templates(&mut templates);

        let ids: Vec<&str> = templates.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, vec!["a", "b"]);
    }

    #[test]
    fn sort_empty_is_noop() {
        let mut templates: Vec<ChannelTemplateRecord> = Vec::new();
        sort_channel_templates(&mut templates);
        assert!(templates.is_empty());
    }

    #[test]
    fn serialization_round_trip() {
        use crate::templates::{
            TemplateAgentEntry, TemplateBackend, TemplateMemberEntry, TemplateTeamEntry,
        };

        let original = ChannelTemplateRecord {
            id: "t1".to_string(),
            name: "Sprint Planning".to_string(),
            description: Some("Template for sprint channels".to_string()),
            channel_type: "stream".to_string(),
            visibility: "private".to_string(),
            canvas_template: Some("# {channel.name}\n\nSprint goals here".to_string()),
            agents: TemplateAgentRoster {
                personas: vec![TemplateAgentEntry {
                    persona_id: "builtin:fizz".to_string(),
                    runtime: Some("claude".to_string()),
                    model: Some("opus".to_string()),
                    role: Some("bot".to_string()),
                    backend: Some(TemplateBackend::Local),
                }],
                teams: vec![TemplateTeamEntry {
                    team_id: "team-1".to_string(),
                    runtime: None,
                    model: None,
                    backend: Some(TemplateBackend::Provider {
                        id: "provider-1".to_string(),
                    }),
                }],
            },
            members: vec![TemplateMemberEntry {
                pubkey: "member-1".to_string(),
                role: "guest".to_string(),
            }],
            is_builtin: false,
            created_at: "2026-05-11T00:00:00Z".to_string(),
            updated_at: "2026-05-11T00:00:00Z".to_string(),
        };

        let json = serde_json::to_string_pretty(&original).unwrap();
        let parsed: ChannelTemplateRecord = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.id, original.id);
        assert_eq!(parsed.name, original.name);
        assert_eq!(parsed.description, original.description);
        assert_eq!(parsed.channel_type, original.channel_type);
        assert_eq!(parsed.visibility, original.visibility);
        assert_eq!(parsed.canvas_template, original.canvas_template);
        assert_eq!(parsed.agents.personas.len(), 1);
        assert_eq!(parsed.agents.teams.len(), 1);
        assert_eq!(parsed.agents.personas[0].persona_id, "builtin:fizz");
        assert_eq!(parsed.agents.personas[0].runtime.as_deref(), Some("claude"));
        assert_eq!(parsed.agents.teams[0].team_id, "team-1");
        assert_eq!(parsed.members[0].pubkey, "member-1");
        assert_eq!(parsed.members[0].role, "guest");
        assert!(!parsed.is_builtin);
    }

    #[test]
    fn deserialization_defaults() {
        let json = r#"{"id":"t1","name":"Minimal","created_at":"2026-05-11T00:00:00Z","updated_at":"2026-05-11T00:00:00Z"}"#;
        let parsed: ChannelTemplateRecord = serde_json::from_str(json).unwrap();

        assert_eq!(parsed.channel_type, "stream");
        assert_eq!(parsed.visibility, "open");
        assert!(!parsed.is_builtin);
        assert!(parsed.description.is_none());
        assert!(parsed.canvas_template.is_none());
        assert!(parsed.agents.personas.is_empty());
        assert!(parsed.agents.teams.is_empty());
        assert!(parsed.members.is_empty());
    }

    #[test]
    fn deserializes_live_legacy_record_without_identity_fields() {
        let json = r#"{
          "name": "Code Team",
          "description": "Coding collaboration",
          "channel_type": "stream",
          "visibility": "private",
          "canvas_template": "",
          "agents": {
            "teams": [],
            "personas": [{ "personaId": "persona-1" }]
          }
        }"#;

        let mut parsed: ChannelTemplateRecord = serde_json::from_str(json).unwrap();

        assert!(parsed.id.is_empty());
        assert!(parsed.created_at.is_empty());
        assert!(parsed.updated_at.is_empty());
        assert_eq!(parsed.agents.personas[0].persona_id, "persona-1");
        assert!(parsed.members.is_empty());

        assert!(migrate_missing_identity_fields(
            std::slice::from_mut(&mut parsed),
            "2026-08-23T19:30:00Z",
        ));
        assert!(uuid::Uuid::parse_str(&parsed.id).is_ok());
        assert_eq!(parsed.created_at, "2026-08-23T19:30:00Z");
        assert_eq!(parsed.updated_at, parsed.created_at);
        assert!(!migrate_missing_identity_fields(
            std::slice::from_mut(&mut parsed),
            "2026-08-24T00:00:00Z",
        ));
    }

    #[test]
    fn deserialization_backward_compat_provider_alias() {
        use crate::templates::{TemplateAgentEntry, TemplateTeamEntry};

        let agent_json = r#"{"personaId":"builtin:fizz","provider":"goose"}"#;
        let agent: TemplateAgentEntry = serde_json::from_str(agent_json).unwrap();
        assert_eq!(agent.runtime.as_deref(), Some("goose"));

        let team_json = r#"{"teamId":"team-1","provider":"claude"}"#;
        let team: TemplateTeamEntry = serde_json::from_str(team_json).unwrap();
        assert_eq!(team.runtime.as_deref(), Some("claude"));
    }

    #[test]
    fn validate_deletion_rejects_builtin() {
        let mut t = template("builtin-1", "Builtin");
        t.is_builtin = true;

        let err = validate_channel_template_deletion(&t).unwrap_err();
        assert_eq!(err, "Built-in templates cannot be deleted.");
    }

    #[test]
    fn validate_deletion_allows_custom() {
        let t = template("custom-1", "Custom");
        assert!(validate_channel_template_deletion(&t).is_ok());
    }
}
