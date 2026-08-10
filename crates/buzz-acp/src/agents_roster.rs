//! Live reload of the Buzz-managed block in `AGENTS.md` for long-running agents.
//!
//! Desktop regenerates the block when agents or the active community relay change.
//! The harness injects updates into turn prompts when the managed section changes.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

const BEGIN_MARKER: &str = "<!-- BEGIN BUZZ MANAGED";
const END_MARKER: &str = "<!-- END BUZZ MANAGED -->";

static ROSTER_CACHE: Mutex<Option<HashMap<PathBuf, RosterSnapshot>>> = Mutex::new(None);

#[derive(Clone, PartialEq, Eq)]
struct RosterSnapshot {
    content_hash: u64,
}

fn hash_bytes(bytes: &[u8]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut hasher);
    hasher.finish()
}

/// Extract the managed section body (including markers) from `AGENTS.md` text.
pub fn extract_managed_section(content: &str) -> Option<String> {
    let begin = content.find(BEGIN_MARKER)?;
    let end = content[begin..]
        .find(END_MARKER)
        .map(|p| p + begin + END_MARKER.len())?;
    Some(content[begin..end].to_string())
}

fn agents_md_path(nest_dir: &Path) -> PathBuf {
    nest_dir.join("AGENTS.md")
}

/// If the managed roster in `nest_dir/AGENTS.md` changed since the last poll for
/// this agent process, return the fresh section for injection into the prompt.
pub fn poll_roster_update(nest_dir: &Path) -> Option<String> {
    let path = agents_md_path(nest_dir);
    let content = std::fs::read_to_string(&path).ok()?;
    let section = extract_managed_section(&content)?;
    let digest = hash_bytes(section.as_bytes());

    let mut guard = ROSTER_CACHE.lock().ok()?;
    let map = guard.get_or_insert_with(HashMap::new);
    let prev = map.get(&path).cloned();
    if prev.as_ref().is_some_and(|s| s.content_hash == digest) {
        return None;
    }
    map.insert(
        path,
        RosterSnapshot {
            content_hash: digest,
        },
    );
    prev.as_ref()?;
    Some(format!(
        "The Active Agents roster in AGENTS.md was updated. Use this authoritative list \
         (names, persona labels, instance pubkeys, relay URL for the active Desktop community):\n\n{section}"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_managed_section_finds_block() {
        let md =
            "intro\n<!-- BEGIN BUZZ MANAGED -->\n## Active Agents\n<!-- END BUZZ MANAGED -->\n";
        let block = extract_managed_section(md).expect("block");
        assert!(block.contains("Active Agents"));
    }
}
