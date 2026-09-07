//! Durable scope is owner + community, independent of the global agent identity.
use super::*;

/// Only the scoped sets are persisted. The resolved launch is an immutable native projection.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeConfigurationStore {
    scopes: BTreeMap<String, BTreeMap<String, RuntimeConfigurations>>,
    #[serde(skip)]
    pub(super) launch: Option<RuntimeConfiguration>,
}

impl RuntimeConfigurationStore {
    pub(crate) fn get(&self, owner: &str, community: &str) -> RuntimeConfigurations {
        self.scopes
            .get(owner)
            .and_then(|communities| communities.get(community))
            .cloned()
            .unwrap_or_default()
    }

    /// Replace one authorized scope atomically; callers persist the whole agent once.
    pub(crate) fn replace(
        &mut self,
        owner: &str,
        community: &str,
        host: &str,
        mut next: RuntimeConfigurations,
    ) -> Result<(), String> {
        next.validate()?;
        let previous = self.get(owner, community);
        // A local editor may preserve, but neither invent, modify nor delete another host's entries.
        for entry in previous.entries.iter().filter(|entry| entry.host != host) {
            if !next.entries.contains(entry) {
                return Err("Another Desktop's configuration cannot be changed here".into());
            }
        }
        for entry in &mut next.entries {
            let old = previous.entries.iter().find(|old| old.id == entry.id);
            if entry.host != host && old != Some(entry) {
                return Err("Another Desktop's configuration cannot be changed here".into());
            }
            if old.is_some_and(|old| old.host != entry.host) {
                return Err("A configuration's Desktop cannot be changed".into());
            }
            if old != Some(entry) {
                entry.revision = uuid::Uuid::new_v4().to_string();
            }
        }
        if next.selected()?.is_some_and(|entry| entry.host != host) {
            return Err("Select a configuration on this Desktop".into());
        }
        self.scopes
            .entry(owner.into())
            .or_default()
            .insert(community.into(), next);
        Ok(())
    }
}

/// Capture selection at the actual launch pair, never the record's creation community.
pub(crate) fn selected_reference(
    record: &ManagedAgentRecord,
    owner: Option<&str>,
    community: &str,
) -> Result<Option<RuntimeConfigurationRef>, String> {
    let Some(owner) = owner else {
        return Ok(None);
    };
    record
        .runtime_configurations
        .get(owner, community)
        .selected()
        .map(|entry| entry.map(RuntimeConfiguration::reference))
}
