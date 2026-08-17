use std::collections::{HashMap, HashSet};
use std::path::Path;

use buzz_core::agent_skill::{
    validate_and_decrypt_skill_pointer, validate_and_decrypt_skill_version, SkillPointerV1,
    SkillVersionV1,
};
use buzz_core::kind::{KIND_AGENT_SKILL_POINTER, KIND_AGENT_SKILL_VERSION};
use nostr::{Alphabet, Event, Filter, Keys, Kind, PublicKey, SecretKey, SingleLetterTag};

use super::materialize::{materialize_active_skills, MaterializeError};
use super::registry::{RegistryError, SkillRegistry};
use crate::relay::RestClient;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct RebuildReport {
    pub versions: usize,
    pub active: usize,
    pub isolated_versions: usize,
    pub isolated_pointers: usize,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum RebuildError {
    #[error(transparent)]
    Registry(#[from] RegistryError),
    #[error(transparent)]
    Materialize(#[from] MaterializeError),
    #[error("relay skill query failed")]
    Relay,
}

pub(crate) fn skill_rebuild_filters(agent: &PublicKey, owner: &PublicKey) -> [Filter; 2] {
    let p_tag = SingleLetterTag::lowercase(Alphabet::P);
    [
        Filter::new()
            .kind(Kind::Custom(KIND_AGENT_SKILL_VERSION as u16))
            .author(*agent)
            .custom_tags(p_tag, [owner.to_hex()])
            .limit(5_000),
        Filter::new()
            .kind(Kind::Custom(KIND_AGENT_SKILL_POINTER as u16))
            .author(*agent)
            .custom_tags(p_tag, [owner.to_hex()])
            .limit(5_000),
    ]
}

pub(crate) async fn rebuild_registry(
    rest: &RestClient,
    agent_keys: &Keys,
    owner: &PublicKey,
    registry: &SkillRegistry,
    skill_root: &Path,
) -> Result<RebuildReport, RebuildError> {
    let value = rest
        .query(&skill_rebuild_filters(&agent_keys.public_key(), owner))
        .await
        .map_err(|_| RebuildError::Relay)?;
    let values = value.as_array().ok_or(RebuildError::Relay)?;
    let events = values
        .iter()
        .filter_map(|value| serde_json::from_value::<Event>(value.clone()).ok())
        .collect::<Vec<_>>();
    rebuild_registry_from_events(
        registry,
        skill_root,
        &events,
        &agent_keys.public_key(),
        owner,
        agent_keys.secret_key(),
        owner,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn rebuild_registry_from_events(
    registry: &SkillRegistry,
    skill_root: &Path,
    events: &[Event],
    expected_agent: &PublicKey,
    expected_owner: &PublicKey,
    my_seckey: &SecretKey,
    their_pubkey: &PublicKey,
) -> Result<RebuildReport, RebuildError> {
    let mut report = RebuildReport::default();
    let mut versions = HashMap::<String, SkillVersionV1>::new();
    let mut version_bytes = HashMap::<String, Vec<u8>>::new();
    let mut pointers = Vec::<(Event, SkillPointerV1)>::new();

    for event in events {
        match event.kind.as_u16() as u32 {
            KIND_AGENT_SKILL_VERSION => match validate_and_decrypt_skill_version(
                event,
                expected_agent,
                expected_owner,
                my_seckey,
                their_pubkey,
            ) {
                Ok(version) => {
                    let encoded = serde_json::to_vec(&version).map_err(|_| RebuildError::Relay)?;
                    match version_bytes.get(&version.version_id) {
                        Some(existing) if existing != &encoded => report.isolated_versions += 1,
                        Some(_) => {}
                        None => {
                            version_bytes.insert(version.version_id.clone(), encoded);
                            versions.insert(version.version_id.clone(), version);
                        }
                    }
                }
                Err(_) => report.isolated_versions += 1,
            },
            KIND_AGENT_SKILL_POINTER => match validate_and_decrypt_skill_pointer(
                event,
                expected_agent,
                expected_owner,
                my_seckey,
                their_pubkey,
            ) {
                Ok(pointer) => pointers.push((event.clone(), pointer)),
                Err(_) => report.isolated_pointers += 1,
            },
            _ => {}
        }
    }

    pointers.sort_by(|(left_event, _), (right_event, _)| {
        right_event
            .created_at
            .cmp(&left_event.created_at)
            .then_with(|| right_event.id.cmp(&left_event.id))
    });
    let mut active = HashMap::<String, String>::new();
    let mut resolved_skills = HashSet::new();
    for (_, pointer) in pointers {
        if resolved_skills.contains(&pointer.skill_id) {
            continue;
        }
        let valid_target = versions
            .get(&pointer.active_version_id)
            .is_some_and(|version| {
                version.skill_id == pointer.skill_id
                    && version.scope == pointer.scope
                    && version.specialist_id == pointer.specialist_id
                    && version.team_id == pointer.team_id
            });
        if valid_target {
            resolved_skills.insert(pointer.skill_id.clone());
            active.insert(pointer.skill_id, pointer.active_version_id);
        } else {
            report.isolated_pointers += 1;
        }
    }

    registry.replace_authoritative(&versions.values().cloned().collect::<Vec<_>>(), &active)?;
    let active_versions = active
        .values()
        .filter_map(|version_id| versions.get(version_id).cloned())
        .collect::<Vec<_>>();
    materialize_active_skills(skill_root, &active_versions)?;
    report.versions = versions.len();
    report.active = active.len();
    Ok(report)
}
