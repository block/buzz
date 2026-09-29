//! Per-agent text-to-speech choices for one local huddle session.

use std::collections::{BTreeMap, HashSet};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};

use crate::app_state::AppState;

use super::{
    tts_settings::{
        pocket_voice_reference, resolve_voice_for_backend_in_registry, voice_registry,
        VoiceRegistryEntry, POCKET_BACKEND_ID,
    },
    HuddlePhase, HuddleState,
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentVoiceSettings {
    pub enabled: bool,
    pub voice_key: String,
}

struct AgentVoiceCatalog {
    default_voice_key: String,
    voices: Vec<VoiceRegistryEntry>,
}

fn catalog(app: &AppHandle, state: &AppState) -> Result<AgentVoiceCatalog, String> {
    let registry = voice_registry(app);
    let settings = state
        .huddle_audio
        .tts
        .lock()
        .map_err(|error| format!("text-to-speech settings lock poisoned: {error}"))?
        .clone();
    let voices: Vec<_> = registry
        .iter()
        .filter(|voice| {
            voice.backend == POCKET_BACKEND_ID
                && matches!(voice.availability.as_str(), "bundled" | "installed")
        })
        .cloned()
        .collect();
    let default_voice_key = resolve_voice_for_backend_in_registry(
        &settings.voice_preferences,
        POCKET_BACKEND_ID,
        &voices,
    )?
    .key;
    Ok(AgentVoiceCatalog {
        default_voice_key,
        voices,
    })
}

fn stable_voice_index(agent_pubkey: &str, huddle_generation: u64, len: usize) -> usize {
    let hash = agent_pubkey.bytes().fold(
        0xcbf2_9ce4_8422_2325_u64 ^ huddle_generation,
        |hash, byte| hash.wrapping_mul(0x0000_0100_0000_01b3) ^ u64::from(byte),
    );
    (hash as usize) % len
}

pub(crate) fn sync_agent_voice_assignments(
    huddle: &mut HuddleState,
    agent_pubkeys: &[String],
    default_voice_key: &str,
    voices: &[VoiceRegistryEntry],
) -> bool {
    let previous = huddle.agent_voice_settings.clone();
    let available_keys: Vec<_> = voices.iter().map(|voice| voice.key.clone()).collect();
    let available: HashSet<_> = available_keys.iter().cloned().collect();
    let agents: HashSet<_> = agent_pubkeys.iter().cloned().collect();
    huddle.agent_voice_settings.retain(|pubkey, settings| {
        agents.contains(pubkey) && available.contains(&settings.voice_key)
    });

    let mut used: HashSet<_> = huddle
        .agent_voice_settings
        .values()
        .map(|settings| settings.voice_key.clone())
        .collect();
    for (index, pubkey) in agent_pubkeys.iter().enumerate() {
        if huddle.agent_voice_settings.contains_key(pubkey) {
            continue;
        }
        let preferred = if index == 0 && !used.contains(default_voice_key) {
            Some(default_voice_key.to_owned())
        } else {
            let unused_alternates: Vec<_> = available_keys
                .iter()
                .filter(|key| key.as_str() != default_voice_key && !used.contains(*key))
                .cloned()
                .collect();
            let unused: Vec<_> = available_keys
                .iter()
                .filter(|key| !used.contains(*key))
                .cloned()
                .collect();
            let candidates = if unused_alternates.is_empty() {
                if unused.is_empty() {
                    &available_keys
                } else {
                    &unused
                }
            } else {
                &unused_alternates
            };
            (!candidates.is_empty()).then(|| {
                candidates[stable_voice_index(pubkey, huddle.huddle_generation, candidates.len())]
                    .clone()
            })
        };
        if let Some(voice_key) = preferred {
            used.insert(voice_key.clone());
            huddle.agent_voice_settings.insert(
                pubkey.clone(),
                AgentVoiceSettings {
                    enabled: true,
                    voice_key,
                },
            );
        }
    }
    huddle.agent_voice_settings != previous
}

fn ensure_with_catalog(
    huddle: &mut HuddleState,
    catalog: &AgentVoiceCatalog,
    extra_agent: Option<&str>,
) -> bool {
    let mut agents = huddle
        .agent_pubkeys
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .clone();
    if let Some(pubkey) = extra_agent {
        if !agents.iter().any(|agent| agent == pubkey) {
            agents.push(pubkey.to_owned());
        }
    }
    sync_agent_voice_assignments(huddle, &agents, &catalog.default_voice_key, &catalog.voices)
}

fn require_active_huddle(huddle: &HuddleState) -> Result<(), String> {
    matches!(huddle.phase, HuddlePhase::Connected | HuddlePhase::Active)
        .then_some(())
        .ok_or_else(|| "No active huddle".to_owned())
}

#[tauri::command]
pub fn ensure_huddle_agent_voice_settings(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<BTreeMap<String, AgentVoiceSettings>, String> {
    let catalog = catalog(&app, &state)?;
    let (changed, settings) = {
        let mut huddle = state.huddle()?;
        if !matches!(huddle.phase, HuddlePhase::Connected | HuddlePhase::Active) {
            return Ok(BTreeMap::new());
        }
        let changed = ensure_with_catalog(&mut huddle, &catalog, None);
        (changed, huddle.agent_voice_settings.clone())
    };
    if changed {
        state.emit_huddle_state_changed();
    }
    Ok(settings)
}

#[tauri::command]
pub fn set_huddle_agent_tts_enabled(
    agent_pubkey: String,
    enabled: bool,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<AgentVoiceSettings, String> {
    let catalog = catalog(&app, &state)?;
    let settings = {
        let mut huddle = state.huddle()?;
        require_active_huddle(&huddle)?;
        ensure_with_catalog(&mut huddle, &catalog, Some(&agent_pubkey));
        let settings = huddle
            .agent_voice_settings
            .get_mut(&agent_pubkey)
            .ok_or("Agent is not in the active huddle")?;
        settings.enabled = enabled;
        settings.clone()
    };
    state.emit_huddle_state_changed();
    Ok(settings)
}

#[tauri::command]
pub fn set_huddle_agent_voice(
    agent_pubkey: String,
    voice_key: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<AgentVoiceSettings, String> {
    let catalog = catalog(&app, &state)?;
    if !catalog.voices.iter().any(|voice| voice.key == voice_key) {
        return Err("The selected Pocket voice is not available on this device".to_owned());
    }
    let settings = {
        let mut huddle = state.huddle()?;
        require_active_huddle(&huddle)?;
        ensure_with_catalog(&mut huddle, &catalog, Some(&agent_pubkey));
        let settings = huddle
            .agent_voice_settings
            .get_mut(&agent_pubkey)
            .ok_or("Agent is not in the active huddle")?;
        settings.voice_key = voice_key;
        settings.clone()
    };
    state.emit_huddle_state_changed();
    Ok(settings)
}

pub(crate) fn voice_reference_for_agent(
    app: &AppHandle,
    state: &AppState,
    agent_pubkey: &str,
) -> Result<Option<String>, String> {
    let catalog = catalog(app, state)?;
    let (changed, settings) = {
        let mut huddle = state.huddle()?;
        require_active_huddle(&huddle)?;
        let changed = ensure_with_catalog(&mut huddle, &catalog, Some(agent_pubkey));
        let settings = huddle.agent_voice_settings.get(agent_pubkey).cloned();
        (changed, settings)
    };
    if changed {
        state.emit_huddle_state_changed();
    }
    let Some(settings) = settings else {
        return Err("Agent is not in the active huddle".to_owned());
    };
    if !settings.enabled {
        return Ok(None);
    }
    pocket_voice_reference(app, &[settings.voice_key]).map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::huddle::tts_settings::bundled_voice_registry;

    #[test]
    fn first_agent_uses_default_and_additional_agents_are_distinct() {
        let agents = vec!["first".to_owned(), "second".to_owned(), "third".to_owned()];
        let mut huddle = HuddleState {
            huddle_generation: 9,
            ..HuddleState::default()
        };

        assert!(sync_agent_voice_assignments(
            &mut huddle,
            &agents,
            "pocket:vera",
            &bundled_voice_registry(),
        ));

        assert_eq!(
            huddle.agent_voice_settings["first"].voice_key,
            "pocket:vera"
        );
        let distinct: HashSet<_> = huddle
            .agent_voice_settings
            .values()
            .map(|settings| settings.voice_key.as_str())
            .collect();
        assert_eq!(distinct.len(), 3);
    }

    #[test]
    fn explicit_session_choices_survive_roster_resync() {
        let agents = vec!["first".to_owned(), "second".to_owned()];
        let voices = bundled_voice_registry();
        let mut huddle = HuddleState::default();
        sync_agent_voice_assignments(&mut huddle, &agents, "pocket:mary", &voices);
        huddle
            .agent_voice_settings
            .get_mut("second")
            .unwrap()
            .enabled = false;
        huddle
            .agent_voice_settings
            .get_mut("second")
            .unwrap()
            .voice_key = "pocket:jane".into();

        assert!(!sync_agent_voice_assignments(
            &mut huddle,
            &agents,
            "pocket:mary",
            &voices,
        ));
        assert_eq!(
            huddle.agent_voice_settings["second"],
            AgentVoiceSettings {
                enabled: false,
                voice_key: "pocket:jane".into(),
            }
        );
    }
}
