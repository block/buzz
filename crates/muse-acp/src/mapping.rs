//! Channel -> Muse side-chat mapping, with discovery and creation.
//!
//! A Buzz channel maps 1:1 to a Muse side chat. That mapping must be
//! deterministic and idempotent: inviting the bridge to a channel it has
//! already seen must never create a second side chat, rename the first
//! one, or change the title it already uses.
//!
//! How discovery works, in order, for a channel and the list of side
//! chat titles the browser actually shows (the candidates returned by
//! the selection script in [`crate::selectors`]):
//!
//! 1. If a persisted mapping exists for the channel and its title is
//!    among the candidates, use it (`Found`, source `Mapped`).
//! 2. Else if a default title (`<channel> (buzz)`, then the bare channel
//!    name) is among the candidates, use it and persist it
//!    (`Found`, source `Discovered`).
//! 3. Else no side chat for the channel exists in the UI yet: record
//!    the deterministic default title and return `Created`. The caller
//!    then runs the new-chat step, which is the UI creation — the
//!    bridge cannot rename a Muse web chat through the DOM, so the
//!    created mapping is the *intended* title, and the operator may
//!    need to rename the chat or set `MUSE_ACP_SIDE_CHAT_MAP` if the
//!    web app auto-titles the new chat differently. That limitation is
//!    stated here and in the README, not hidden behind a success.
//!
//! The mapping file is a plain JSON object, `channel -> title`, written
//! atomically (temp file + rename) so a crash mid-write cannot leave a
//! half-written mapping for a restart to misread. Keys are normalized
//! (trimmed, lower-cased) for lookup, so `MASKY-CMO` and `masky-cmo`
//! are the same channel; the stored title keeps its original casing.
//!
//! This module is pure except for the file store: it never touches the
//! browser, the ACP layer, or the handoff gate. Tests drive it with
//! candidate lists, not a DOM.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::BridgeError;

/// Environment variable naming the mapping file. When unset, the driver
/// does not persist mappings to disk (it still resolves in memory from
/// `MUSE_ACP_SIDE_CHAT_MAP` and the default titles); when set, the
/// driver loads and saves this file on every resolve.
pub const MAP_FILE_ENV: &str = "MUSE_ACP_SIDE_CHAT_MAP_FILE";

/// The deterministic title for a channel that has no side chat yet.
pub fn default_title(channel: &str) -> String {
    format!("{} (buzz)", channel.trim())
}

/// Candidate titles for a channel, in discovery order: the persisted
/// mapping (if any), then the default title, then the bare channel name.
pub fn candidate_titles(channel: &str, mapped: Option<&str>) -> Vec<String> {
    let mut titles = Vec::new();
    if let Some(title) = mapped.filter(|t| !t.trim().is_empty()) {
        titles.push(title.to_string());
    }
    let default = default_title(channel);
    if !titles.iter().any(|t| t.eq_ignore_ascii_case(&default)) {
        titles.push(default);
    }
    if !titles
        .iter()
        .any(|t| t.eq_ignore_ascii_case(channel.trim()))
    {
        titles.push(channel.trim().to_string());
    }
    titles
}

fn normalize(channel: &str) -> String {
    channel.trim().to_ascii_lowercase()
}

/// How a channel's side chat was resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SideChatResolution {
    /// A side chat already exists in the UI. `title` is the title that
    /// matched, `created` is always false.
    Found {
        title: String,
        /// `Mapped` when the title came from a persisted mapping,
        /// `Discovered` when it came from a default title found in the
        /// candidate list.
        source: ResolutionSource,
    },
    /// No side chat for the channel exists in the UI yet. `title` is
    /// the deterministic title recorded for it; the caller must create
    /// the UI chat (new-chat step) for this to become real.
    Created { title: String },
}

/// Where a `Found` resolution came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolutionSource {
    Mapped,
    Discovered,
}

impl SideChatResolution {
    /// The title to select (or to create) for this channel.
    pub fn title(&self) -> &str {
        match self {
            SideChatResolution::Found { title, .. } => title,
            SideChatResolution::Created { title } => title,
        }
    }

    /// True only for the creation path.
    pub fn is_created(&self) -> bool {
        matches!(self, SideChatResolution::Created { .. })
    }
}

/// Resolve a channel against the side-chat titles the UI shows.
///
/// Pure: no file or browser access. The caller decides whether to
/// persist a `Discovered` or `Created` result via
/// [`ChannelSideChatMapping`].
pub fn resolve_side_chat(
    channel: &str,
    mapped: Option<&str>,
    candidates: &[String],
) -> SideChatResolution {
    // Mapped title wins, but only if the UI actually shows it. A
    // mapped title that is not in the candidate list means the chat was
    // deleted or renamed in the UI, so fall through to discovery /
    // creation rather than clicking a title that is not there.
    if let Some(title) = mapped.filter(|t| !t.trim().is_empty()) {
        if candidates.iter().any(|c| c.eq_ignore_ascii_case(title)) {
            return SideChatResolution::Found {
                title: title.to_string(),
                source: ResolutionSource::Mapped,
            };
        }
    }
    for title in candidate_titles(channel, None) {
        if candidates.iter().any(|c| c.eq_ignore_ascii_case(&title)) {
            return SideChatResolution::Found {
                title,
                source: ResolutionSource::Discovered,
            };
        }
    }
    SideChatResolution::Created {
        title: default_title(channel),
    }
}

/// A file-backed channel -> side-chat-title store.
#[derive(Debug, Default)]
pub struct ChannelSideChatMapping {
    /// Normalized channel -> title.
    entries: HashMap<String, String>,
    /// Original channel casing, for reporting (first seen wins).
    originals: HashMap<String, String>,
}

impl ChannelSideChatMapping {
    /// An empty, in-memory mapping.
    pub fn new() -> Self {
        Self::default()
    }

    /// Load a mapping from `path`. A missing file is an empty mapping;
    /// a malformed file is an error (the caller falls back to empty and
    /// warns, rather than crashing the turn — see `load_or_empty`).
    pub fn load(path: &Path) -> Result<Self, BridgeError> {
        let text = match fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::new()),
            Err(e) => return Err(BridgeError::Io(e)),
        };
        if text.trim().is_empty() {
            return Ok(Self::new());
        }
        let value: Value = serde_json::from_str(&text)?;
        let mut mapping = Self::new();
        if let Some(obj) = value.as_object() {
            for (channel, title) in obj {
                if let Some(title) = title.as_str() {
                    if !title.trim().is_empty() {
                        mapping.insert(channel, title);
                    }
                }
            }
        }
        Ok(mapping)
    }

    /// Load, or an empty mapping on any read/parse failure. Used by the
    /// driver: a corrupt mapping file must not strand a channel's turn.
    pub fn load_or_empty(path: &Path) -> Self {
        Self::load(path).unwrap_or_default()
    }

    /// Save atomically: write a temp file in the same directory, then
    /// rename over the destination, so a crash cannot leave a
    /// half-written mapping.
    pub fn save(&self, path: &Path) -> Result<(), BridgeError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut obj = serde_json::Map::new();
        for (normalized, title) in &self.entries {
            let display = self
                .originals
                .get(normalized)
                .cloned()
                .unwrap_or_else(|| normalized.clone());
            obj.insert(display, Value::String(title.clone()));
        }
        let text = serde_json::to_string_pretty(&Value::Object(obj))?;
        let tmp = path.with_extension("tmp");
        fs::write(&tmp, text)?;
        fs::rename(&tmp, path)?;
        Ok(())
    }

    /// The title currently mapped for `channel`, if any.
    pub fn get(&self, channel: &str) -> Option<&str> {
        self.entries.get(&normalize(channel)).map(String::as_str)
    }

    /// Record a mapping, overwriting any previous title for the channel.
    pub fn insert(&mut self, channel: &str, title: &str) {
        let key = normalize(channel);
        self.originals
            .entry(key.clone())
            .or_insert_with(|| channel.trim().to_string());
        self.entries.insert(key, title.to_string());
    }

    /// Resolve a channel against the UI candidates, persisting a
    /// `Discovered` or `Created` result. Calling this twice with the
    /// same channel and candidates returns the same title and the same
    /// stored mapping — that is the idempotency guarantee.
    ///
    /// Returns the resolution and whether the store changed (so the
    /// caller knows whether to save).
    pub fn get_or_create(
        &mut self,
        channel: &str,
        candidates: &[String],
    ) -> (SideChatResolution, bool) {
        let mapped = self.get(channel).map(str::to_string);
        let resolution = resolve_side_chat(channel, mapped.as_deref(), candidates);
        match &resolution {
            SideChatResolution::Found {
                source: ResolutionSource::Discovered,
                title,
            } => {
                // Persist the discovery so the next resolve is Mapped,
                // not a fresh scan.
                if mapped.as_deref() != Some(title.as_str()) {
                    self.insert(channel, title);
                    return (resolution, true);
                }
                (resolution, false)
            }
            SideChatResolution::Created { title } => {
                if mapped.as_deref() != Some(title.as_str()) {
                    self.insert(channel, title);
                    return (resolution, true);
                }
                (resolution, false)
            }
            SideChatResolution::Found {
                source: ResolutionSource::Mapped,
                ..
            } => (resolution, false),
        }
    }

    /// Path from the environment, if set.
    pub fn path_from_env() -> Option<PathBuf> {
        std::env::var(MAP_FILE_ENV)
            .ok()
            .filter(|v| !v.trim().is_empty())
            .map(PathBuf::from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn candidates(titles: &[&str]) -> Vec<String> {
        titles.iter().map(|t| t.to_string()).collect()
    }

    #[test]
    fn discovery_prefers_mapped_title_when_present() {
        let res = resolve_side_chat(
            "MASKY-CMO",
            Some("My CMO chat"),
            &candidates(&["My CMO chat", "MASKY-CMO (buzz)"]),
        );
        assert_eq!(
            res,
            SideChatResolution::Found {
                title: "My CMO chat".to_string(),
                source: ResolutionSource::Mapped,
            }
        );
    }

    #[test]
    fn discovery_falls_back_to_default_when_mapped_title_absent() {
        // Mapped title not in the UI (deleted/renamed): do not click a
        // title that is not there; discover the default instead.
        let res = resolve_side_chat(
            "MASKY-CMO",
            Some("Old chat"),
            &candidates(&["MASKY-CMO (buzz)", "General"]),
        );
        assert_eq!(
            res,
            SideChatResolution::Found {
                title: "MASKY-CMO (buzz)".to_string(),
                source: ResolutionSource::Discovered,
            }
        );
    }

    #[test]
    fn creation_when_no_candidate_matches() {
        let res = resolve_side_chat("new-channel", None, &candidates(&["General"]));
        assert_eq!(
            res,
            SideChatResolution::Created {
                title: "new-channel (buzz)".to_string(),
            }
        );
        assert!(res.is_created());
    }

    #[test]
    fn get_or_create_is_idempotent() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("map.json");
        let mut mapping = ChannelSideChatMapping::new();
        // First resolve: no candidate, so Created, and the store changes.
        let (first, changed) = mapping.get_or_create("MASKY-CMO", &candidates(&["General"]));
        assert!(changed);
        assert_eq!(first.title(), "MASKY-CMO (buzz)");
        mapping.save(&path).unwrap();
        // Reload, then resolve again with the same candidates: the
        // mapped title is still not in the UI, so the resolution is
        // Created again with the same title, and the store does not change.
        let mut reloaded = ChannelSideChatMapping::load(&path).unwrap();
        let (second, changed) = reloaded.get_or_create("MASKY-CMO", &candidates(&["General"]));
        assert!(!changed);
        assert_eq!(second.title(), first.title());
        // Once the UI shows the title, the next resolve is Found/Mapped.
        let (third, changed) =
            reloaded.get_or_create("MASKY-CMO", &candidates(&["MASKY-CMO (buzz)"]));
        assert!(!changed);
        assert_eq!(
            third,
            SideChatResolution::Found {
                title: "MASKY-CMO (buzz)".to_string(),
                source: ResolutionSource::Mapped,
            }
        );
    }

    #[test]
    fn mapping_persists_across_restart() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("map.json");
        let mut mapping = ChannelSideChatMapping::new();
        mapping.insert("MASKY-CMO", "My CMO chat");
        mapping.save(&path).unwrap();
        let reloaded = ChannelSideChatMapping::load(&path).unwrap();
        assert_eq!(reloaded.get("masky-cmo"), Some("My CMO chat"));
        assert_eq!(reloaded.get("MASKY-CMO"), Some("My CMO chat"));
    }

    #[test]
    fn malformed_mapping_file_is_an_error_not_a_panic() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("map.json");
        fs::write(&path, "not json").unwrap();
        assert!(ChannelSideChatMapping::load(&path).is_err());
        // The driver fallback is empty, not a crash.
        assert!(ChannelSideChatMapping::load_or_empty(&path)
            .get("x")
            .is_none());
    }

    #[test]
    fn missing_mapping_file_is_empty() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("does-not-exist.json");
        let mapping = ChannelSideChatMapping::load(&path).unwrap();
        assert!(mapping.get("MASKY-CMO").is_none());
    }
}
