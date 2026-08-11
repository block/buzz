use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Owner-reviewed filesystem boundary for a local managed agent.
///
/// The profile is instance-local and is never published with a persona. Each
/// start creates a new run root; additional roots are read-only and must be
/// explicitly declared by the owner. The shared Buzz nest is denied even when
/// a configured root would otherwise overlap it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum FilesystemIsolationProfile {
    Ephemeral {
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        read_only_roots: Vec<PathBuf>,
    },
}
