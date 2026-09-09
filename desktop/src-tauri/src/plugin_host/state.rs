//! On-disk registry persistence.
//!
//! The registry file is the authoritative record of every installed plugin:
//! its package location, manifest summary, enabled state, and grant. All
//! mutation (commit, `set_enabled`, uninstall) is serialized by the caller
//! under one lock ([`crate::plugin_host::PluginHost`]'s registry mutex); this
//! module only knows how to load and atomically replace the file.

use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use atomic_write_file::AtomicWriteFile;
use serde::{Deserialize, Serialize};

use crate::plugin_host::types::{ContributionSummary, InstalledPlugin, PluginError};

const REGISTRY_FILE_NAME: &str = "registry.json";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RegistryRecord {
    pub plugin_id: String,
    pub name: String,
    pub version: String,
    pub publisher: String,
    pub enabled: bool,
    pub contributions: Vec<ContributionSummary>,
    pub package_path: PathBuf,
    pub digest: String,
    /// Grants approved at install time (currently always `["browser.browse"]`).
    pub grants: Vec<String>,
}

impl RegistryRecord {
    pub fn summary(&self) -> InstalledPlugin {
        InstalledPlugin {
            plugin_id: self.plugin_id.clone(),
            name: self.name.clone(),
            version: self.version.clone(),
            publisher: self.publisher.clone(),
            enabled: self.enabled,
            contributions: self.contributions.clone(),
        }
    }
}

#[derive(Default, Serialize, Deserialize)]
pub struct RegistryFile {
    #[serde(default)]
    pub plugins: HashMap<String, RegistryRecord>,
}

/// Loads and atomically replaces the registry file. Holds no lock itself;
/// the caller must serialize load-mutate-save sequences.
pub struct RegistryStore {
    root: PathBuf,
}

impl RegistryStore {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    fn registry_path(&self) -> PathBuf {
        self.root.join(REGISTRY_FILE_NAME)
    }

    /// The plugin host's own config root, the trusted base every owned
    /// storage path must resolve within.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Directory holding every committed plugin's package directory.
    pub fn plugins_root(&self) -> PathBuf {
        self.root.join("plugins")
    }

    /// Directory holding transient staged packages awaiting commit.
    pub fn staging_root(&self) -> PathBuf {
        self.root.join("staging")
    }

    pub fn load(&self) -> Result<RegistryFile, PluginError> {
        match fs::read(self.registry_path()) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map_err(|error| PluginError::Registry(format!("parse registry: {error}"))),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Ok(RegistryFile::default())
            }
            Err(error) => Err(PluginError::Registry(format!("read registry: {error}"))),
        }
    }

    pub fn save(&self, file: &RegistryFile) -> Result<(), PluginError> {
        fs::create_dir_all(&self.root)
            .map_err(|error| PluginError::Registry(format!("create registry root: {error}")))?;
        let bytes = serde_json::to_vec_pretty(file)
            .map_err(|error| PluginError::Registry(format!("serialize registry: {error}")))?;
        let mut target = AtomicWriteFile::open(self.registry_path())
            .map_err(|error| PluginError::Registry(format!("open registry for write: {error}")))?;
        target
            .write_all(&bytes)
            .map_err(|error| PluginError::Registry(format!("write registry: {error}")))?;
        target
            .commit()
            .map_err(|error| PluginError::Registry(format!("commit registry: {error}")))?;
        Ok(())
    }
}
