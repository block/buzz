//! NIP-CL channel labels: bounded values, command grammar and trusted snapshots.
//!
//! This module has no I/O. Authorization, serialization and capability admission
//! belong to the relay; these types do not confer authority.

mod command;
mod outcome;
mod snapshot;

pub use command::LabelCommand;
pub use outcome::CommandOutcome;
pub use snapshot::verify_snapshot;

use std::collections::BTreeSet;

/// NIP-32 namespace reserved for the channel label projection.
pub const NAMESPACE: &str = "nip-cl";
/// Maximum number of distinct labels stored on one channel.
pub const MAX_LABELS: usize = 32;
/// Maximum ASCII bytes per label.
pub const MAX_LABEL_BYTES: usize = 64;
/// Maximum raw operation tags, before deduplication.
pub const MAX_OPERATION_TAGS: usize = 64;

/// A violation of the NIP-CL wire contract. Details never contain stored state.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct LabelError(pub &'static str);

/// A canonical, ASCII-sorted, duplicate-free bounded set of channel labels.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LabelSet(Vec<String>);

impl LabelSet {
    /// Validate values without normalization, deduplicate and sort them.
    pub fn new(values: impl IntoIterator<Item = String>) -> Result<Self, LabelError> {
        let mut labels = BTreeSet::new();
        for value in values {
            validate_value(&value)?;
            labels.insert(value);
            if labels.len() > MAX_LABELS {
                return Err(LabelError("too many stored labels"));
            }
        }
        Ok(Self(labels.into_iter().collect()))
    }

    /// The complete canonical values; unknown valid values must be preserved.
    pub fn values(&self) -> &[String] {
        &self.0
    }

    /// Whether the set is empty (empty snapshots omit both `L` and `l`).
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Encode the single-namespace NIP-32 projection, in canonical order.
    pub fn snapshot_tags(&self) -> Vec<Vec<String>> {
        if self.is_empty() {
            return Vec::new();
        }
        let mut tags = vec![vec!["L".into(), NAMESPACE.into()]];
        tags.extend(
            self.0
                .iter()
                .map(|value| vec!["l".into(), value.clone(), NAMESPACE.into()]),
        );
        tags
    }

    /// Apply a validated incremental command, checking the final (not interim) size.
    pub fn apply(&self, command: &LabelCommand) -> Result<Self, LabelError> {
        match command {
            LabelCommand::Create { labels, .. } => Ok(labels.clone()),
            LabelCommand::Mutate { add, remove, .. } => Self::new(
                self.0
                    .iter()
                    .chain(add)
                    .filter(|value| !remove.contains(*value))
                    .cloned(),
            ),
        }
    }
}

pub(super) fn validate_value(value: &str) -> Result<(), LabelError> {
    let bytes = value.as_bytes();
    let alphanumeric = |byte: &u8| byte.is_ascii_lowercase() || byte.is_ascii_digit();
    if bytes.is_empty()
        || bytes.len() > MAX_LABEL_BYTES
        || !alphanumeric(&bytes[0])
        || !bytes
            .iter()
            .all(|byte| alphanumeric(byte) || b"._:/-".contains(byte))
    {
        return Err(LabelError("invalid label value"));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
