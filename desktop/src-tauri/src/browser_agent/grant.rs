//! In-memory browser-agent grants keyed by webview label.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BrowserAgentMode {
    Observe,
    Drive,
}

impl BrowserAgentMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Observe => "observe",
            Self::Drive => "drive",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BrowserAgentSurface {
    Playground,
    Pin,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserAgentGrant {
    pub webview_label: String,
    pub surface: BrowserAgentSurface,
    pub surface_id: String,
    pub agent_id: String,
    pub agent_pubkey: String,
    pub channel_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_root: Option<String>,
    pub mode: BrowserAgentMode,
    /// When true, Drive grant stays but the page lock is off so the human can assist.
    #[serde(default)]
    pub user_has_control: bool,
    pub created_at_ms: u64,
}

#[derive(Debug, Default)]
pub struct BrowserAgentGrantStore {
    grants: Mutex<HashMap<String, BrowserAgentGrant>>,
}

impl BrowserAgentGrantStore {
    pub fn get(&self, webview_label: &str) -> Option<BrowserAgentGrant> {
        self.grants
            .lock()
            .ok()?
            .get(webview_label)
            .cloned()
    }

    pub fn list_for_agent(&self, agent_pubkey: &str) -> Vec<BrowserAgentGrant> {
        let Ok(map) = self.grants.lock() else {
            return Vec::new();
        };
        map.values()
            .filter(|g| g.agent_pubkey == agent_pubkey)
            .cloned()
            .collect()
    }

    pub fn list_all(&self) -> Vec<BrowserAgentGrant> {
        let Ok(map) = self.grants.lock() else {
            return Vec::new();
        };
        map.values().cloned().collect()
    }

    /// Insert or replace. Returns the previous grant when replacing another agent.
    pub fn set(
        &self,
        grant: BrowserAgentGrant,
        allow_replace: bool,
    ) -> Result<Option<BrowserAgentGrant>, String> {
        let mut map = self
            .grants
            .lock()
            .map_err(|_| "browser agent grant lock poisoned".to_string())?;
        if let Some(existing) = map.get(&grant.webview_label) {
            if existing.agent_pubkey != grant.agent_pubkey && !allow_replace {
                return Err(
                    "another agent already holds this browser; confirm replace".into(),
                );
            }
        }
        let previous = map.insert(grant.webview_label.clone(), grant);
        Ok(previous)
    }

    pub fn clear(&self, webview_label: &str) -> Option<BrowserAgentGrant> {
        self.grants.lock().ok()?.remove(webview_label)
    }

    pub fn clear_surface(&self, surface_id: &str) -> Vec<BrowserAgentGrant> {
        let Ok(mut map) = self.grants.lock() else {
            return Vec::new();
        };
        let keys: Vec<String> = map
            .iter()
            .filter(|(_, g)| g.surface_id == surface_id)
            .map(|(k, _)| k.clone())
            .collect();
        keys.into_iter()
            .filter_map(|k| map.remove(&k))
            .collect()
    }


    pub fn get_for_surface(&self, surface_id: &str) -> Option<BrowserAgentGrant> {
        let Ok(map) = self.grants.lock() else {
            return None;
        };
        map.values()
            .find(|g| g.surface_id == surface_id)
            .cloned()
    }

    /// Move a surface grant onto `new_label` (detach / host switch). Idempotent when
    /// the grant is already on that label.

    /// Move a grant from one playground tab surface to another (tab switch).
    /// Updates both `surface_id` and `webview_label`. Idempotent when already on target.
    pub fn rebind_across_surfaces(
        &self,
        from_surface_id: &str,
        to_surface_id: &str,
        new_label: &str,
    ) -> Option<BrowserAgentGrant> {
        let mut map = self.grants.lock().ok()?;
        if let Some(existing) = map.get(new_label) {
            if existing.surface_id == to_surface_id {
                return Some(existing.clone());
            }
        }
        let old_key = map
            .iter()
            .find(|(_, g)| g.surface_id == from_surface_id)
            .map(|(k, _)| k.clone())?;
        let mut grant = map.remove(&old_key)?;
        grant.surface_id = to_surface_id.to_string();
        grant.webview_label = new_label.to_string();
        map.insert(new_label.to_string(), grant.clone());
        Some(grant)
    }

    pub fn rebind_surface_to_label(
        &self,
        surface_id: &str,
        new_label: &str,
    ) -> Option<BrowserAgentGrant> {
        let mut map = self.grants.lock().ok()?;
        if let Some(existing) = map.get(new_label) {
            if existing.surface_id == surface_id {
                return Some(existing.clone());
            }
        }
        let old_key = map
            .iter()
            .find(|(_, g)| g.surface_id == surface_id)
            .map(|(k, _)| k.clone())?;
        let mut grant = map.remove(&old_key)?;
        grant.webview_label = new_label.to_string();
        map.insert(new_label.to_string(), grant.clone());
        Some(grant)
    }

    pub fn set_user_has_control(
        &self,
        webview_label: &str,
        user_has_control: bool,
    ) -> Option<BrowserAgentGrant> {
        let mut map = self.grants.lock().ok()?;
        let grant = map.get_mut(webview_label)?;
        grant.user_has_control = user_has_control;
        Some(grant.clone())
    }

    pub fn require_mode(
        &self,
        webview_label: &str,
        agent_pubkey: &str,
        mode: BrowserAgentMode,
    ) -> Result<BrowserAgentGrant, String> {
        let grant = self
            .get(webview_label)
            .ok_or_else(|| "no browser agent grant for this webview".to_string())?;
        if grant.agent_pubkey != agent_pubkey {
            return Err("caller is not the granted agent for this webview".into());
        }
        match (mode, grant.mode) {
            (BrowserAgentMode::Observe, BrowserAgentMode::Observe)
            | (BrowserAgentMode::Observe, BrowserAgentMode::Drive)
            | (BrowserAgentMode::Drive, BrowserAgentMode::Drive) => Ok(grant),
            (BrowserAgentMode::Drive, BrowserAgentMode::Observe) => {
                Err("drive requires Drive mode grant".into())
            }
        }
    }
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Canonical live webview label for a playground sid. Window is ignored —
/// one WKWebView is reparented across hosts instead of forking siblings.
pub fn playground_label(sid: &str, _window_label: &str) -> String {
    format!("playground-{sid}")
}

pub fn pin_label(pin_id: &str, window_label: &str) -> String {
    if window_label == "main" || window_label.is_empty() {
        format!("pin-{pin_id}")
    } else {
        format!("pin-{pin_id}--{window_label}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(label: &str, pubkey: &str, mode: BrowserAgentMode) -> BrowserAgentGrant {
        BrowserAgentGrant {
            webview_label: label.into(),
            surface: BrowserAgentSurface::Playground,
            surface_id: "demo".into(),
            agent_id: pubkey.into(),
            agent_pubkey: pubkey.into(),
            channel_id: "chan".into(),
            thread_root: None,
            mode,
            user_has_control: false,
            created_at_ms: 1,
        }
    }

    #[test]
    fn one_agent_replace_requires_flag() {
        let store = BrowserAgentGrantStore::default();
        store
            .set(sample("playground-a", "aa", BrowserAgentMode::Observe), false)
            .unwrap();
        let err = store
            .set(sample("playground-a", "bb", BrowserAgentMode::Drive), false)
            .unwrap_err();
        assert!(err.contains("confirm replace"));
        store
            .set(sample("playground-a", "bb", BrowserAgentMode::Drive), true)
            .unwrap();
        assert_eq!(
            store.get("playground-a").unwrap().agent_pubkey,
            "bb"
        );
    }

    #[test]
    fn observe_grant_allows_observe_not_drive() {
        let store = BrowserAgentGrantStore::default();
        store
            .set(sample("playground-a", "aa", BrowserAgentMode::Observe), false)
            .unwrap();
        assert!(store
            .require_mode("playground-a", "aa", BrowserAgentMode::Observe)
            .is_ok());
        assert!(store
            .require_mode("playground-a", "aa", BrowserAgentMode::Drive)
            .is_err());
    }

    #[test]
    fn drive_grant_allows_observe_and_drive() {
        let store = BrowserAgentGrantStore::default();
        store
            .set(sample("playground-a", "aa", BrowserAgentMode::Drive), false)
            .unwrap();
        assert!(store
            .require_mode("playground-a", "aa", BrowserAgentMode::Observe)
            .is_ok());
        assert!(store
            .require_mode("playground-a", "aa", BrowserAgentMode::Drive)
            .is_ok());
    }

    #[test]
    fn clear_surface_removes_all_labels() {
        let store = BrowserAgentGrantStore::default();
        store
            .set(sample("playground-demo", "aa", BrowserAgentMode::Observe), false)
            .unwrap();
        store
            .set(
                sample("playground-demo--pop", "aa", BrowserAgentMode::Observe),
                false,
            )
            .unwrap();
        let removed = store.clear_surface("demo");
        assert_eq!(removed.len(), 2);
        assert!(store.get("playground-demo").is_none());
    }

    #[test]
    fn rebind_moves_grant_across_window_labels() {
        let store = BrowserAgentGrantStore::default();
        store
            .set(sample("playground-demo", "aa", BrowserAgentMode::Drive), false)
            .unwrap();
        let next = store
            .rebind_surface_to_label("demo", "playground-demo--pop")
            .expect("rebind");
        assert_eq!(next.webview_label, "playground-demo--pop");
        assert!(store.get("playground-demo").is_none());
        assert_eq!(
            store.get("playground-demo--pop").unwrap().agent_pubkey,
            "aa"
        );
    }

    #[test]
    fn user_has_control_pauses_without_clearing() {
        let store = BrowserAgentGrantStore::default();
        store
            .set(sample("playground-a", "aa", BrowserAgentMode::Drive), false)
            .unwrap();
        let paused = store
            .set_user_has_control("playground-a", true)
            .expect("pause");
        assert!(paused.user_has_control);
        assert!(matches!(paused.mode, BrowserAgentMode::Drive));
        assert!(store.get("playground-a").is_some());
        let resumed = store
            .set_user_has_control("playground-a", false)
            .expect("resume");
        assert!(!resumed.user_has_control);
    }

    #[test]
    fn rebind_across_surfaces_updates_surface_and_label() {
        let store = BrowserAgentGrantStore::default();
        let mut g = sample("playground-a", "aa", BrowserAgentMode::Drive);
        g.surface_id = "a".into();
        store.set(g, false).unwrap();
        let next = store
            .rebind_across_surfaces("a", "b", "playground-b")
            .expect("rebind");
        assert_eq!(next.surface_id, "b");
        assert_eq!(next.webview_label, "playground-b");
        assert!(store.get("playground-a").is_none());
        assert_eq!(
            store.get("playground-b").unwrap().mode,
            BrowserAgentMode::Drive
        );
    }
}
