//! Persona command request types, split from `types.rs` (file-size cap).

use std::collections::BTreeMap;

use serde::Deserialize;

use super::{validate_respond_to_allowlist, PersonaRecord, RespondTo};

/// The NIP-AP behavioral quad as one grouped request field.
///
/// Grouped (not flat) because `update_persona` has legacy callers that don't
/// send behavioral fields at all — flat replace semantics would silently wipe
/// a stored quad on every team-import edit. Absent group = don't touch the
/// stored quad; present group = validate and replace all four as a unit
/// (mode and allowlist must travel together).
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PersonaBehaviorRequest {
    #[serde(default)]
    pub respond_to: Option<RespondTo>,
    #[serde(default)]
    pub respond_to_allowlist: Vec<String>,
    #[serde(default)]
    pub mcp_toolsets: Option<String>,
    #[serde(default)]
    pub parallelism: Option<u32>,
}

/// Validate a behavior group and apply it onto a persona record.
///
/// This is the single write path for definition behavioral fields — both
/// `create_persona` and `update_persona` route through it, so neither can
/// skip validation. `None` leaves the record's stored quad untouched (the
/// legacy-caller wipe hazard); `Some` normalizes the allowlist
/// (`validate_respond_to_allowlist`), rejects allowlist mode with an empty
/// list (the spawn-time crash-loop `build_respond_to_env` errors on), rejects
/// out-of-range parallelism, and stores the quad in wire shape.
pub fn apply_persona_behavior(
    record: &mut PersonaRecord,
    behavior: Option<PersonaBehaviorRequest>,
) -> Result<(), String> {
    let Some(behavior) = behavior else {
        return Ok(());
    };

    let allowlist = validate_respond_to_allowlist(&behavior.respond_to_allowlist)?;
    if behavior.respond_to == Some(RespondTo::Allowlist) && allowlist.is_empty() {
        return Err(
            "respond-to mode 'allowlist' requires at least one pubkey in the allowlist".to_string(),
        );
    }
    if let Some(count) = behavior.parallelism {
        if !(1..=32).contains(&count) {
            return Err(format!(
                "parallelism {count} is out of range (must be between 1 and 32)"
            ));
        }
    }

    record.respond_to = behavior.respond_to.map(|mode| mode.as_str().to_string());
    // The allowlist only means something in allowlist mode; storing it for
    // other modes would republish stale pubkeys the author didn't choose.
    record.respond_to_allowlist = if behavior.respond_to == Some(RespondTo::Allowlist) {
        allowlist
    } else {
        Vec::new()
    };
    record.mcp_toolsets = behavior
        .mcp_toolsets
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    record.parallelism = behavior.parallelism;
    Ok(())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreatePersonaRequest {
    pub display_name: String,
    pub avatar_url: Option<String>,
    pub system_prompt: String,
    #[serde(default)]
    pub runtime: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub name_pool: Vec<String>,
    /// Environment variables for agents created from this persona.
    #[serde(default)]
    pub env_vars: BTreeMap<String, String>,
    /// NIP-AP behavioral quad. Absent = quad stays unset.
    #[serde(default)]
    pub behavior: Option<PersonaBehaviorRequest>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdatePersonaRequest {
    pub id: String,
    pub display_name: String,
    pub avatar_url: Option<String>,
    pub system_prompt: String,
    #[serde(default)]
    pub runtime: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub name_pool: Vec<String>,
    /// Environment variables for agents created from this persona.
    ///
    /// Absent (`None`) = don't touch the stored value (caller didn't include
    /// the field). `Some(map)` = replace entirely (empty map clears all).
    /// Defaulting an omitted field to an empty map would silently erase
    /// stored credentials when an unrelated field is edited.
    #[serde(default)]
    pub env_vars: Option<BTreeMap<String, String>>,
    /// NIP-AP behavioral quad. Same absent-vs-present contract as `env_vars`:
    /// absent = don't touch the stored quad (legacy callers don't send it),
    /// present = validate and replace all four fields as a unit.
    #[serde(default)]
    pub behavior: Option<PersonaBehaviorRequest>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record_with_quad() -> PersonaRecord {
        let mut record = record_without_quad();
        record.respond_to = Some("allowlist".to_string());
        record.respond_to_allowlist = vec!["a".repeat(64)];
        record.mcp_toolsets = Some("developer".to_string());
        record.parallelism = Some(4);
        record
    }

    fn record_without_quad() -> PersonaRecord {
        PersonaRecord {
            id: "p-1".to_string(),
            display_name: "Test".to_string(),
            avatar_url: None,
            system_prompt: "prompt".to_string(),
            runtime: None,
            model: None,
            provider: None,
            name_pool: Vec::new(),
            is_builtin: false,
            is_active: true,
            source_team: None,
            source_team_persona_slug: None,
            env_vars: BTreeMap::new(),
            respond_to: None,
            respond_to_allowlist: Vec::new(),
            mcp_toolsets: None,
            parallelism: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
        }
    }

    /// The anchor regression row: an absent behavior group must leave a
    /// stored quad untouched — legacy update_persona callers (team import,
    /// profile panel) send no behavior field and must not wipe it.
    #[test]
    fn absent_behavior_leaves_stored_quad_untouched() {
        let mut record = record_with_quad();
        apply_persona_behavior(&mut record, None).unwrap();
        assert_eq!(record.respond_to.as_deref(), Some("allowlist"));
        assert_eq!(record.respond_to_allowlist, vec!["a".repeat(64)]);
        assert_eq!(record.mcp_toolsets.as_deref(), Some("developer"));
        assert_eq!(record.parallelism, Some(4));
    }

    #[test]
    fn present_behavior_replaces_all_four_as_a_unit() {
        let mut record = record_with_quad();
        apply_persona_behavior(
            &mut record,
            Some(PersonaBehaviorRequest {
                respond_to: Some(RespondTo::Anyone),
                respond_to_allowlist: Vec::new(),
                mcp_toolsets: None,
                parallelism: None,
            }),
        )
        .unwrap();
        assert_eq!(record.respond_to.as_deref(), Some("anyone"));
        assert!(record.respond_to_allowlist.is_empty());
        assert_eq!(record.mcp_toolsets, None);
        assert_eq!(record.parallelism, None);
    }

    #[test]
    fn allowlist_mode_with_empty_list_is_rejected() {
        let mut record = record_without_quad();
        let err = apply_persona_behavior(
            &mut record,
            Some(PersonaBehaviorRequest {
                respond_to: Some(RespondTo::Allowlist),
                respond_to_allowlist: Vec::new(),
                ..Default::default()
            }),
        )
        .unwrap_err();
        assert!(err.contains("allowlist"), "{err}");
        // Rejection must not half-apply: the record stays untouched.
        assert_eq!(record.respond_to, None);
    }

    #[test]
    fn allowlist_entries_are_normalized_via_the_shared_validator() {
        let mut record = record_without_quad();
        let upper = "A".repeat(64);
        apply_persona_behavior(
            &mut record,
            Some(PersonaBehaviorRequest {
                respond_to: Some(RespondTo::Allowlist),
                respond_to_allowlist: vec![upper.clone(), upper],
                ..Default::default()
            }),
        )
        .unwrap();
        // Lowercased and deduplicated, matching the instance-side chokepoint.
        assert_eq!(record.respond_to_allowlist, vec!["a".repeat(64)]);
    }

    #[test]
    fn invalid_allowlist_entry_is_rejected() {
        let mut record = record_without_quad();
        let err = apply_persona_behavior(
            &mut record,
            Some(PersonaBehaviorRequest {
                respond_to: Some(RespondTo::Allowlist),
                respond_to_allowlist: vec!["not-hex".to_string()],
                ..Default::default()
            }),
        )
        .unwrap_err();
        assert!(err.contains("64 hex"), "{err}");
    }

    #[test]
    fn allowlist_is_dropped_for_non_allowlist_modes() {
        let mut record = record_without_quad();
        apply_persona_behavior(
            &mut record,
            Some(PersonaBehaviorRequest {
                respond_to: Some(RespondTo::OwnerOnly),
                respond_to_allowlist: vec!["b".repeat(64)],
                ..Default::default()
            }),
        )
        .unwrap();
        assert!(
            record.respond_to_allowlist.is_empty(),
            "stale pubkeys must not be stored alongside a non-allowlist mode"
        );
    }

    /// Pinky's loop row: an applied behavior group must flow through
    /// `persona_event_content` so the republished 30175 carries the edited
    /// quad — the write path and the publish path cannot drift apart.
    #[test]
    fn applied_behavior_flows_into_persona_event_content() {
        let mut record = record_without_quad();
        apply_persona_behavior(
            &mut record,
            Some(PersonaBehaviorRequest {
                respond_to: Some(RespondTo::Allowlist),
                respond_to_allowlist: vec!["c".repeat(64)],
                mcp_toolsets: Some("developer".to_string()),
                parallelism: Some(3),
            }),
        )
        .unwrap();
        let content = crate::managed_agents::persona_events::persona_event_content(&record);
        assert_eq!(content.respond_to.as_deref(), Some("allowlist"));
        assert_eq!(content.respond_to_allowlist, vec!["c".repeat(64)]);
        assert_eq!(content.mcp_toolsets.as_deref(), Some("developer"));
        assert_eq!(content.parallelism, Some(3));
    }

    #[test]
    fn parallelism_out_of_range_is_rejected_and_blank_toolsets_normalize_to_none() {
        let mut record = record_without_quad();
        for bad in [0u32, 33] {
            let err = apply_persona_behavior(
                &mut record,
                Some(PersonaBehaviorRequest {
                    parallelism: Some(bad),
                    ..Default::default()
                }),
            )
            .unwrap_err();
            assert!(err.contains("out of range"), "{err}");
        }

        apply_persona_behavior(
            &mut record,
            Some(PersonaBehaviorRequest {
                mcp_toolsets: Some("   ".to_string()),
                parallelism: Some(8),
                ..Default::default()
            }),
        )
        .unwrap();
        assert_eq!(record.mcp_toolsets, None, "blank toolsets never persist");
        assert_eq!(record.parallelism, Some(8));
    }
}
