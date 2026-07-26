//! On-disk state for the spawner.
//!
//! Holds the one thing that cannot be reconstructed from the relay: each
//! agent's secret key, minted here and never transmitted. Losing this file
//! orphans every running agent — their pubkeys stay in the owner's channels and
//! in relay membership, but nothing can sign as them again. It therefore belongs
//! on a persistent volume, and is written mode 0600 with an atomic
//! write-then-rename so a crash mid-write cannot truncate it.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// Everything the spawner knows about one agent it manages.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentRecord {
    /// The spec slug this agent reconciles, unique per owner.
    pub slug: String,
    /// Owner pubkey, hex — the author of the spec.
    pub owner_pubkey: String,
    /// Agent pubkey, hex.
    pub agent_pubkey: String,
    /// Agent secret key, bech32 nsec. Never leaves this host.
    pub private_key_nsec: String,
    /// NIP-OA auth tag, as a JSON array string. Absent until the owner attests.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_tag: Option<String>,
    /// Nonce of the attestation round currently in flight, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_nonce: Option<String>,
    /// Unix seconds when the pending attestation request was sent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attestation_sent_at: Option<i64>,
    /// Hash of the spec content the running container was created from, so
    /// drift can be detected without re-reading the container's env.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spec_hash: Option<String>,
    /// Prompt material the owner delivered over the encrypted handshake.
    ///
    /// Present when the spawner cannot read the referenced kind:30175 persona —
    /// which is the normal case, since personas are author-only unless shared
    /// and the spawner is not the author. Held here rather than fetched because
    /// there is nowhere to fetch it from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<buzz_sdk::spawner::PromptMaterial>,
    /// Consecutive failed start attempts, driving backoff.
    #[serde(default)]
    pub restart_count: u32,
    /// Unix seconds of the last failed start, driving backoff.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_failure_at: Option<i64>,
}

impl AgentRecord {
    /// Whether this agent can be started.
    ///
    /// Needs both halves: an owner attestation, and a secret key. A relocated
    /// agent is recorded from its spec before its key arrives, so the key check
    /// is not redundant — without it the spawner would launch a container with
    /// an empty `BUZZ_PRIVATE_KEY` and the harness would fail obscurely.
    pub fn is_attested(&self) -> bool {
        self.auth_tag.is_some() && !self.private_key_nsec.is_empty()
    }

    /// The container name for this agent.
    ///
    /// Keyed on the owner pubkey prefix as well as the slug: slugs are chosen by
    /// clients and are only unique *per owner*, so two owners both naming an
    /// agent `fizz` must not collide into one container on a shared host.
    pub fn container_name(&self) -> String {
        format!(
            "buzz-agent-{}-{}",
            &self.owner_pubkey[..12.min(self.owner_pubkey.len())],
            self.slug
        )
    }
}

/// The serialized state file.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct StateFile {
    /// Keyed by `"<owner_pubkey>/<slug>"`.
    #[serde(default)]
    agents: HashMap<String, AgentRecord>,
}

/// Persistent store over a single JSON file.
pub struct Store {
    path: PathBuf,
    state: StateFile,
}

/// Compose the map key for an agent.
pub fn agent_key(owner_pubkey: &str, slug: &str) -> String {
    format!("{owner_pubkey}/{slug}")
}

impl Store {
    /// Open the store at `state_dir/agents.json`, creating the directory and an
    /// empty state file if they do not exist.
    pub fn open(state_dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(state_dir)
            .with_context(|| format!("failed to create state dir {}", state_dir.display()))?;
        let path = state_dir.join("agents.json");

        let state = match std::fs::read_to_string(&path) {
            Ok(raw) => serde_json::from_str(&raw)
                .with_context(|| format!("failed to parse state file {}", path.display()))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => StateFile::default(),
            Err(e) => return Err(e).with_context(|| format!("failed to read {}", path.display())),
        };

        Ok(Self { path, state })
    }

    /// Every agent the spawner is managing.
    pub fn agents(&self) -> impl Iterator<Item = &AgentRecord> {
        self.state.agents.values()
    }

    /// Look up one agent.
    pub fn get(&self, owner_pubkey: &str, slug: &str) -> Option<&AgentRecord> {
        self.state.agents.get(&agent_key(owner_pubkey, slug))
    }

    /// Find the agent awaiting attestation for `agent_pubkey`, if any.
    ///
    /// The handshake response identifies itself by agent pubkey rather than by
    /// slug, because that is what the owner actually signed over.
    pub fn find_by_agent_pubkey(&self, agent_pubkey: &str) -> Option<&AgentRecord> {
        self.state
            .agents
            .values()
            .find(|r| r.agent_pubkey == agent_pubkey)
    }

    /// Insert or replace an agent and persist.
    pub fn put(&mut self, record: AgentRecord) -> Result<()> {
        let key = agent_key(&record.owner_pubkey, &record.slug);
        self.state.agents.insert(key, record);
        self.flush()
    }

    /// Mutate an agent in place and persist. Does nothing if it is absent.
    pub fn update(
        &mut self,
        owner_pubkey: &str,
        slug: &str,
        f: impl FnOnce(&mut AgentRecord),
    ) -> Result<()> {
        if let Some(record) = self.state.agents.get_mut(&agent_key(owner_pubkey, slug)) {
            f(record);
            self.flush()?;
        }
        Ok(())
    }

    /// Remove an agent and persist, returning the removed record.
    pub fn remove(&mut self, owner_pubkey: &str, slug: &str) -> Result<Option<AgentRecord>> {
        let removed = self.state.agents.remove(&agent_key(owner_pubkey, slug));
        if removed.is_some() {
            self.flush()?;
        }
        Ok(removed)
    }

    /// Write the state file atomically with owner-only permissions.
    fn flush(&self) -> Result<()> {
        let json = serde_json::to_string_pretty(&self.state)
            .context("failed to serialize spawner state")?;

        // Write-then-rename: a crash leaves either the old complete file or the
        // new complete file, never a truncated one holding half the agent keys.
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, &json)
            .with_context(|| format!("failed to write {}", tmp.display()))?;
        restrict_permissions(&tmp)?;
        std::fs::rename(&tmp, &self.path)
            .with_context(|| format!("failed to rename into {}", self.path.display()))?;
        Ok(())
    }
}

#[cfg(unix)]
fn restrict_permissions(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
        .with_context(|| format!("failed to chmod {}", path.display()))
}

#[cfg(not(unix))]
fn restrict_permissions(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(owner: &str, slug: &str) -> AgentRecord {
        AgentRecord {
            slug: slug.into(),
            owner_pubkey: owner.into(),
            agent_pubkey: "a".repeat(64),
            private_key_nsec: "nsec1test".into(),
            auth_tag: None,
            pending_nonce: None,
            attestation_sent_at: None,
            spec_hash: None,
            prompt: None,
            restart_count: 0,
            last_failure_at: None,
        }
    }

    #[test]
    fn round_trips_through_the_file() {
        let dir = std::env::temp_dir().join(format!("buzz-spawner-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);

        let owner = "b".repeat(64);
        {
            let mut store = Store::open(&dir).unwrap();
            store.put(record(&owner, "fizz")).unwrap();
        }

        let store = Store::open(&dir).unwrap();
        assert_eq!(store.get(&owner, "fizz").unwrap().slug, "fizz");
        assert!(store.get(&owner, "missing").is_none());

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn an_adopted_agent_is_not_startable_until_its_key_arrives() {
        // A relocated agent is recorded from its spec before the owner delivers
        // the key. Starting then would launch a container with an empty
        // BUZZ_PRIVATE_KEY and fail deep inside the harness.
        let mut rec = record(&"b".repeat(64), "fizz");
        rec.private_key_nsec = String::new();
        rec.auth_tag = Some(r#"["auth","o","","s"]"#.into());
        assert!(!rec.is_attested());

        rec.private_key_nsec = "nsec1real".into();
        assert!(rec.is_attested());
    }

    #[test]
    fn container_names_are_scoped_per_owner() {
        // Slugs are client-chosen and only unique per owner. Two owners each
        // naming an agent "fizz" must not fight over one container.
        let a = record(&"a".repeat(64), "fizz");
        let b = record(&"c".repeat(64), "fizz");
        assert_ne!(a.container_name(), b.container_name());
    }

    #[test]
    fn state_file_is_owner_only() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let dir =
                std::env::temp_dir().join(format!("buzz-spawner-perm-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);

            let mut store = Store::open(&dir).unwrap();
            store.put(record(&"d".repeat(64), "fizz")).unwrap();

            let mode = std::fs::metadata(dir.join("agents.json"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600, "state file holds agent secret keys");

            std::fs::remove_dir_all(&dir).unwrap();
        }
    }
}
