//! Device-local agent execution policy. This file is never relay-synchronized.

use serde::{Deserialize, Serialize};
use std::{io::Read, path::Path, sync::OnceLock};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
/// Preferences belonging to one Desktop installation, never an agent definition.
pub struct DeviceAgentPolicy {
    /// Prevent this installation from minting or executing agent identities.
    pub client_only: bool,
    /// Allow local hosting with distinct names while retaining remote definitions locally.
    #[serde(default)]
    pub unique_names: bool,
    /// Exact existing identities preferred in discovery for a given owner/name.
    #[serde(default)]
    pub preferred_agents: Vec<PreferredAgent>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
/// A discovery preference, not an identity alias or an authorization grant.
pub struct PreferredAgent {
    /// Canonical HTTP community URL.
    pub relay_url: String,
    /// Verified owner of the identity.
    pub owner_pubkey: String,
    /// Display name whose older instances are omitted from discovery.
    pub name: String,
    /// Exact preferred public key. Explicit historical links remain exact.
    pub pubkey: String,
    /// Stable definition hosted elsewhere, even if its display name changes.
    #[serde(default)]
    pub persona_id: Option<String>,
}

impl DeviceAgentPolicy {
    /// Guard both identities on every edit; consult the directory only for a
    /// new name. Unchanged-name local configuration edits remain usable offline.
    pub async fn check_name_update<F, Fut>(
        &self,
        current_name: &str,
        requested_name: &str,
        pubkey: Option<&str>,
        persona_id: Option<&str>,
        lookup: F,
    ) -> Result<(), String>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<(), String>>,
    {
        self.require_local_agent(current_name, pubkey, persona_id)?;
        self.require_local_agent(requested_name, pubkey, persona_id)?;
        if self.unique_names
            && !current_name
                .trim()
                .eq_ignore_ascii_case(requested_name.trim())
        {
            lookup().await?;
        }
        Ok(())
    }

    /// Validate local management of one named identity.
    pub fn require_local_agent(
        &self,
        name: &str,
        pubkey: Option<&str>,
        persona_id: Option<&str>,
    ) -> Result<(), String> {
        self.require_hosting()?;
        if self.unique_names
            && self.preferred_agents.iter().any(|agent| {
                agent.name.trim().eq_ignore_ascii_case(name.trim())
                    || pubkey.is_some_and(|key| key.eq_ignore_ascii_case(&agent.pubkey))
                    || persona_id.is_some_and(|id| agent.persona_id.as_deref() == Some(id))
            })
        {
            return Err(format!("{name} belongs to an agent hosted on another device. Use that existing identity, or choose a different name for a new local agent."));
        }
        Ok(())
    }

    /// Refuse before generating keys, writing records, or spawning processes.
    pub fn require_hosting(&self) -> Result<(), String> {
        if self.client_only {
            return Err("This device is in client-only mode. Use an existing agent from the relay, or change Agent hosting in Settings and restart Buzz.".into());
        }
        Ok(())
    }

    /// Narrow discovery only within the explicitly configured owner/community.
    pub fn allows_identity(
        &self,
        relay_url: &str,
        owner: Option<&str>,
        name: &str,
        pubkey: &str,
    ) -> bool {
        self.preferred_agents.iter().all(|preferred| {
            preferred.relay_url.trim_end_matches('/') != relay_url.trim_end_matches('/')
                || owner != Some(preferred.owner_pubkey.as_str())
                || !preferred.name.trim().eq_ignore_ascii_case(name.trim())
                || preferred.pubkey.eq_ignore_ascii_case(pubkey)
        })
    }
}

/// Read a bounded policy file. Only a missing file inherits the historical default.
pub fn load_policy(path: &Path) -> Result<DeviceAgentPolicy, String> {
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(DeviceAgentPolicy::default())
        }
        Err(error) => return Err(format!("Cannot read agent device policy: {error}")),
    };
    let mut bytes = Vec::new();
    file.take(65_537)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("Cannot read agent device policy: {error}"))?;
    if bytes.len() > 65_536 {
        return Err("Agent device policy exceeds 64 KiB".into());
    }
    serde_json::from_slice(&bytes).map_err(|error| format!("Invalid agent device policy: {error}"))
}

/// Freeze execution policy for this application lifetime so a preference change
/// cannot race a suspended create/deploy operation. Changes require a restart.
pub fn active_policy(
    cache: &OnceLock<Result<DeviceAgentPolicy, String>>,
    loader: impl FnOnce() -> Result<DeviceAgentPolicy, String>,
) -> Result<&DeviceAgentPolicy, String> {
    cache.get_or_init(loader).as_ref().map_err(Clone::clone)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn local_configuration_edit_does_not_depend_on_an_online_name_directory() {
        let policy = DeviceAgentPolicy {
            unique_names: true,
            ..Default::default()
        };
        for requested in ["Notebook", " notebook "] {
            policy
                .check_name_update("Notebook", requested, Some("local-key"), None, || async {
                    Err("relay is offline".into())
                })
                .await
                .unwrap();
        }
        assert_eq!(
            policy
                .check_name_update("Notebook", "Different", Some("local-key"), None, || async {
                    Err("relay is offline".into())
                })
                .await,
            Err("relay is offline".into())
        );
    }

    #[tokio::test]
    async fn unchanged_names_still_enforce_protected_identity_guards() {
        let mut policy = DeviceAgentPolicy {
            unique_names: true,
            ..Default::default()
        };
        policy.preferred_agents.push(PreferredAgent {
            relay_url: "https://relay.example".into(),
            owner_pubkey: "owner".into(),
            name: "Scout".into(),
            pubkey: "remote-key".into(),
            persona_id: Some("remote-definition".into()),
        });
        for (before, after, key, persona) in [
            ("Scout", "Scout", "other-key", None),
            ("Local", "Local", "remote-key", None),
            ("Local", "Local", "other-key", Some("remote-definition")),
            ("Scout", "Renamed", "other-key", None),
            ("Local", "Scout", "other-key", None),
        ] {
            assert!(policy
                .check_name_update(before, after, Some(key), persona, || async { Ok(()) })
                .await
                .is_err());
        }
    }

    #[test]
    fn unique_names_allows_new_agents_but_protects_remote_names_keys_and_definitions() {
        let policy: DeviceAgentPolicy = serde_json::from_str(
            r#"{
            "client_only":false,"unique_names":true,
            "preferred_agents":[{"relay_url":"https://relay.example","owner_pubkey":"owner",
                "name":"Scout","pubkey":"remote-key","persona_id":"remote-definition"}]
        }"#,
        )
        .unwrap();
        assert!(policy.require_local_agent("Notebook", None, None).is_ok());
        assert!(policy.require_local_agent(" sCoUt ", None, None).is_err());
        assert!(policy
            .require_local_agent("Renamed", Some("remote-key"), None)
            .is_err());
        assert!(policy
            .require_local_agent("Renamed", None, Some("remote-definition"))
            .is_err());
        assert!(policy
            .require_local_agent("Notebook", Some("local-key"), Some("local-definition"))
            .is_ok());
    }

    #[test]
    fn settings_changes_and_read_errors_stay_frozen_until_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("policy.json");
        std::fs::write(&path, r#"{"client_only":true}"#).unwrap();
        let cache = OnceLock::new();
        assert!(
            active_policy(&cache, || load_policy(&path))
                .unwrap()
                .client_only
        );
        std::fs::write(&path, r#"{"client_only":false}"#).unwrap();
        assert!(
            active_policy(&cache, || load_policy(&path))
                .unwrap()
                .client_only
        );
        assert!(
            !active_policy(&OnceLock::new(), || load_policy(&path))
                .unwrap()
                .client_only
        );
        let failed_cache = OnceLock::new();
        assert!(active_policy(&failed_cache, || Err("read failed".into())).is_err());
        assert!(active_policy(&failed_cache, || load_policy(&path)).is_err());
    }

    #[test]
    fn client_only_refuses_execution_even_without_presence() {
        let policy = DeviceAgentPolicy {
            client_only: true,
            ..Default::default()
        };
        assert!(policy.require_hosting().is_err());
    }

    #[test]
    fn saved_client_only_policy_survives_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("agent-device-policy.json");
        std::fs::write(&path, r#"{"client_only":true}"#).unwrap();
        assert!(load_policy(&path).unwrap().require_hosting().is_err());
        assert!(load_policy(&path).unwrap().require_hosting().is_err());
    }

    #[test]
    fn unreadable_or_invalid_policy_never_enables_hosting() {
        let dir = tempfile::tempdir().unwrap();
        assert!(load_policy(dir.path()).is_err());
        let path = dir.path().join("policy.json");
        for bytes in ["broken", "{}", r#"{"client_only":"true"}"#] {
            std::fs::write(&path, bytes).unwrap();
            assert!(load_policy(&path).is_err(), "accepted {bytes}");
        }
    }

    #[test]
    fn unconfigured_devices_keep_existing_hosting_behavior() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("absent.json");
        assert!(load_policy(&path).unwrap().require_hosting().is_ok());
        std::fs::write(&path, r#"{"client_only":false}"#).unwrap();
        assert!(load_policy(&path).unwrap().require_hosting().is_ok());
    }

    #[test]
    fn preferred_identity_is_stable_and_scoped_to_owner_and_community() {
        let policy = DeviceAgentPolicy {
            client_only: true,
            unique_names: false,
            preferred_agents: vec![PreferredAgent {
                relay_url: "https://relay.example".into(),
                owner_pubkey: "a".repeat(64),
                name: "Scout".into(),
                pubkey: "b".repeat(64),
                persona_id: None,
            }],
        };
        let owner = "a".repeat(64);
        // There is deliberately no presence input: offline does not mean replaceable.
        assert!(policy.allows_identity(
            "https://relay.example",
            Some(&owner),
            "Scout",
            &"b".repeat(64)
        ));
        assert!(!policy.allows_identity(
            "https://relay.example",
            Some(&owner),
            "Scout",
            &"c".repeat(64)
        ));
        assert!(policy.allows_identity(
            "https://other.example",
            Some(&owner),
            "Scout",
            &"c".repeat(64)
        ));
        assert!(policy.allows_identity(
            "https://relay.example",
            Some(&"d".repeat(64)),
            "Scout",
            &"c".repeat(64)
        ));
        assert!(policy.allows_identity("https://relay.example", None, "Scout", &"c".repeat(64)));
        assert!(policy.allows_identity(
            "https://relay.example",
            Some(&owner),
            "Unrelated",
            &"c".repeat(64)
        ));
    }
}
