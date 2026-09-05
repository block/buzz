//! Exhaustive `ManagedAgentRecord` field-authority fixture (NIP-PMA "Field
//! authority").
//!
//! `classify!` destructures the record with no `..`, so adding a field to
//! `ManagedAgentRecord` fails to compile here until the field is classified.
//! Each classification is then checked against the codec as implemented:
//! mutating a field that rides the kind:30179 private config must change the
//! payload body `retain_private_agent_record` compares, and mutating a field
//! that does not must leave it byte-identical (no phantom republish).

use super::*;
use crate::managed_agents::{
    BackendKind, CatalogSource, RelayMeshConfig, RespondTo, TeamMemberCatalogSource,
};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Authority {
    /// The aggregate coordinate itself (`d` tag / `agent_pubkey`).
    Coordinate,
    /// kind:30177 instance projection; also carried privately so a fresh
    /// device can reconstruct the instance without the public head.
    InstanceProjection,
    /// kind:30175 is authoritative for a definition-linked instance; carried
    /// privately as the runnable snapshot for definition-less instances.
    DefinitionMirror,
    /// kind:30175 definition/catalog projection only. Public display and
    /// provenance data; never enters the private config.
    DefinitionProjection,
    /// Private portable canonical: secrets and durable runnable settings.
    PrivatePortable,
    /// Private, re-validated on each device before use.
    PrivateDeviceValidated,
    /// Local device policy or derived from the local catalog/toolchain.
    LocalDerived,
    /// Legacy conversion only: create-time mirrors and deprecated knobs.
    LegacyOnly,
    /// Transient local only: process and last-run receipts.
    Transient,
    /// Timestamps. `updated_at` rides the payload as advisory bookkeeping but
    /// is excluded from body equality; `created_at` stays local.
    Bookkeeping,
}

impl Authority {
    fn rides_private_config(self) -> bool {
        matches!(
            self,
            Self::Coordinate
                | Self::InstanceProjection
                | Self::DefinitionMirror
                | Self::PrivatePortable
                | Self::PrivateDeviceValidated
        )
    }
}

struct Field {
    name: &'static str,
    authority: Authority,
    mutate: fn(&mut ManagedAgentRecord),
}

/// One entry per record field. The destructure is the compile-time tripwire:
/// a field missing from (or duplicated in) this list is a build error.
macro_rules! classify {
    ($record:expr; $($field:ident => $authority:ident, $mutate:expr;)*) => {{
        let ManagedAgentRecord { $($field: _,)* } = $record;
        vec![$(Field {
            name: stringify!($field),
            authority: Authority::$authority,
            mutate: $mutate,
        },)*]
    }};
}

fn classified_fields(record: &ManagedAgentRecord) -> Vec<Field> {
    classify!(record;
        pubkey => Coordinate, |r| r.pubkey = "22".repeat(32);
        name => InstanceProjection, |r| r.name = "renamed".into();
        persona_id => InstanceProjection, |r| r.persona_id = Some("def".into());
        team_id => PrivatePortable, |r| r.team_id = Some("team".into());
        private_key_nsec => PrivatePortable, |r| r.private_key_nsec = "nsec-other".into();
        auth_tag => PrivatePortable, |r| r.auth_tag = Some("[\"auth\"]".into());
        relay_url => PrivateDeviceValidated, |r| r.relay_url = "wss://other".into();
        avatar_url => DefinitionProjection, |r| r.avatar_url = Some("https://a/v.png".into());
        acp_command => LocalDerived, |r| r.acp_command = "/usr/local/bin/buzz-acp".into();
        agent_command => LegacyOnly, |r| r.agent_command = "claude".into();
        agent_command_override => PrivateDeviceValidated, |r| r.agent_command_override = Some("claude".into());
        agent_args => PrivateDeviceValidated, |r| r.agent_args.push("--flag".into());
        mcp_command => LegacyOnly, |r| r.mcp_command = "buzz-dev-mcp".into();
        turn_timeout_seconds => LegacyOnly, |r| r.turn_timeout_seconds += 1;
        idle_timeout_seconds => PrivatePortable, |r| r.idle_timeout_seconds = Some(90);
        max_turn_duration_seconds => PrivatePortable, |r| r.max_turn_duration_seconds = Some(900);
        parallelism => InstanceProjection, |r| r.parallelism += 1;
        system_prompt => DefinitionMirror, |r| r.system_prompt = Some("other prompt".into());
        model => DefinitionMirror, |r| r.model = Some("other-model".into());
        provider => DefinitionMirror, |r| r.provider = Some("other-provider".into());
        persona_source_version => LegacyOnly, |r| r.persona_source_version = Some("v2".into());
        env_vars => PrivatePortable, |r| { r.env_vars.insert("K".into(), "v".into()); };
        start_on_app_launch => LocalDerived, |r| r.start_on_app_launch = !r.start_on_app_launch;
        auto_restart_on_config_change => LocalDerived, |r| {
            r.auto_restart_on_config_change = !r.auto_restart_on_config_change;
        };
        runtime_pid => Transient, |r| r.runtime_pid = Some(4242);
        backend => PrivatePortable, |r| {
            r.backend = BackendKind::Provider {
                id: "blox".into(),
                config: serde_json::json!({"region": "x"}),
            };
        };
        backend_agent_id => PrivateDeviceValidated, |r| r.backend_agent_id = Some("remote-1".into());
        provider_policy_pending => LocalDerived, |r| {
            r.provider_policy_pending = !r.provider_policy_pending;
        };
        provider_binary_path => LocalDerived, |r| r.provider_binary_path = Some("/opt/p".into());
        persona_team_dir => LocalDerived, |r| r.persona_team_dir = Some(PathBuf::from("/teams/t"));
        persona_name_in_team => PrivatePortable, |r| r.persona_name_in_team = Some("scout".into());
        created_at => Bookkeeping, |r| r.created_at = "2027-01-01T00:00:00Z".into();
        updated_at => Bookkeeping, |r| r.updated_at = "2027-01-01T00:00:00Z".into();
        last_started_at => Transient, |r| r.last_started_at = Some("2027-01-01T00:00:00Z".into());
        last_stopped_at => Transient, |r| r.last_stopped_at = Some("2027-01-01T00:00:00Z".into());
        last_exit_code => Transient, |r| r.last_exit_code = Some(1);
        last_error => Transient, |r| r.last_error = Some("boom".into());
        last_error_code => Transient, |r| r.last_error_code = Some(7);
        respond_to => InstanceProjection, |r| r.respond_to = RespondTo::Anyone;
        respond_to_allowlist => InstanceProjection, |r| r.respond_to_allowlist.push("ab".repeat(32));
        display_name => DefinitionProjection, |r| r.display_name = Some("Display".into());
        description => DefinitionProjection, |r| r.description = Some("desc".into());
        slug => DefinitionProjection, |r| r.slug = Some("slug".into());
        runtime => DefinitionMirror, |r| r.runtime = Some("claude".into());
        name_pool => DefinitionProjection, |r| r.name_pool.push("Nova".into());
        is_builtin => DefinitionProjection, |r| r.is_builtin = !r.is_builtin;
        is_active => DefinitionProjection, |r| r.is_active = !r.is_active;
        shared => LegacyOnly, |r| r.shared = !r.shared;
        source_team => DefinitionProjection, |r| r.source_team = Some("team".into());
        source_team_persona_slug => DefinitionProjection, |r| {
            r.source_team_persona_slug = Some("scout".into());
        };
        catalog_source => DefinitionProjection, |r| {
            r.catalog_source = Some(CatalogSource {
                owner_pubkey: "33".repeat(32),
                persona_id: "p".into(),
            });
        };
        team_catalog_source => DefinitionProjection, |r| {
            r.team_catalog_source = Some(TeamMemberCatalogSource {
                owner_pubkey: "33".repeat(32),
                team_d_tag: "t".into(),
                member_key: "m".into(),
                projection_hash: "h".into(),
            });
        };
        definition_respond_to => DefinitionProjection, |r| {
            r.definition_respond_to = Some("anyone".into());
        };
        definition_respond_to_allowlist => DefinitionProjection, |r| {
            r.definition_respond_to_allowlist.push("ab".repeat(32));
        };
        definition_parallelism => DefinitionProjection, |r| r.definition_parallelism = Some(3);
        relay_mesh => DefinitionMirror, |r| {
            r.relay_mesh = Some(RelayMeshConfig {
                model_ref: "Qwen3".into(),
            });
        };
        effort_level => PrivatePortable, |r| r.effort_level = Some("high".into());
    )
}

#[test]
fn every_record_field_is_classified_and_the_codec_agrees() {
    let owner = "11".repeat(32);
    let base = sample_record(&"aa".repeat(32), "classified");
    let base_payload = private_payload_from_record(&base, &owner, 1, None).unwrap();

    for field in classified_fields(&base) {
        let mut mutated = base.clone();
        (field.mutate)(&mut mutated);
        assert_ne!(
            mutated, base,
            "{}: mutator must change the record",
            field.name
        );
        let payload = private_payload_from_record(&mutated, &owner, 1, None).unwrap();
        let rides = !private_payload_body_eq(&base_payload, &payload);
        assert_eq!(
            rides,
            field.authority.rides_private_config(),
            "{} is classified {:?} but the kind:30179 payload body {}",
            field.name,
            field.authority,
            if rides { "changed" } else { "did not change" }
        );
    }
}
