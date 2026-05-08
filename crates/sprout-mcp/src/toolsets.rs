//! # Toolset System
//!
//! Controls which MCP tools are exposed based on the `SPROUT_TOOLSETS` environment
//! variable.
//!
//! ## Syntax
//!
//! ```text
//! SPROUT_TOOLSETS="default,channel_admin:ro,canvas"
//! ```
//!
//! Comma-separated list of toolset names with optional `:ro` / `:rw` suffix.
//! Special keywords: `default`, `all`, `none`.
//!
//! Later entries override earlier ones, so `all:ro,default:rw` gives read-only
//! access everywhere except the default toolset which gets full write access.
//!
//! ## Toolsets
//!
//! | Name            | Tools |
//! |-----------------|-------|
//! | `default`       | 26    |
//! | `channel_admin` | 5     |
//! | `dms`           | 3     |
//! | `canvas`        | 2     |
//! | `workflow_admin`| 5     |
//! | `identity`      | 1     |
//! | `forums`        | 1     |
//! | `social`        | 5     |

use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

// ---------------------------------------------------------------------------
// Static data
// ---------------------------------------------------------------------------

/// `(tool_name, toolset_name, is_read)`
///
/// Single source of truth for every tool's toolset membership and read/write
/// classification. `is_read = true` means the tool is safe to include under
/// a `:ro` (read-only) mode restriction.
///
/// See [`DEFERRED_TOOLS`] for tools planned but not yet implemented.
pub const ALL_TOOLS: &[(&str, &str, bool)] = &[
    // ── default ─────────────────────────────────────────────────────────────
    ("send_message", "default", false),
    ("send_diff_message", "default", false),
    ("edit_message", "default", false),
    ("delete_message", "default", false),
    ("get_messages", "default", true),
    ("get_thread", "default", true),
    ("search", "default", true),
    ("get_feed", "default", true),
    ("add_reaction", "default", false),
    ("remove_reaction", "default", false),
    ("get_reactions", "default", true),
    ("list_channels", "default", true),
    ("get_channel", "default", true),
    ("join_channel", "default", false),
    ("leave_channel", "default", false),
    ("update_channel", "default", false),
    ("set_channel_topic", "default", false),
    ("set_channel_purpose", "default", false),
    ("open_dm", "default", false),
    ("get_users", "default", true),
    ("set_profile", "default", false),
    ("get_presence", "default", true),
    ("set_presence", "default", false),
    ("trigger_workflow", "default", false),
    ("approve_step", "default", false),
    ("list_channel_members", "default", true),
    // ── channel_admin ────────────────────────────────────────────────────────
    ("create_channel", "channel_admin", false),
    ("archive_channel", "channel_admin", false),
    ("unarchive_channel", "channel_admin", false),
    ("add_channel_member", "channel_admin", false),
    ("remove_channel_member", "channel_admin", false),
    // ── dms ──────────────────────────────────────────────────────────────────
    ("add_dm_member", "dms", false),
    ("hide_dm", "dms", false),
    ("list_dms", "dms", true),
    // ── canvas ───────────────────────────────────────────────────────────────
    ("get_canvas", "canvas", true),
    ("set_canvas", "canvas", false),
    // ── workflow_admin ────────────────────────────────────────────────────────
    ("list_workflows", "workflow_admin", true),
    ("create_workflow", "workflow_admin", false),
    ("update_workflow", "workflow_admin", false),
    ("delete_workflow", "workflow_admin", false),
    ("get_workflow_runs", "workflow_admin", true),
    // ── identity ──────────────────────────────────────────────────────────────
    ("set_channel_add_policy", "identity", false),
    // ── forums ───────────────────────────────────────────────────────────────
    ("vote_on_post", "forums", false),
    // ── social ───────────────────────────────────────────────────────────────
    // Social tools for NIP-01/NIP-02 (text notes + contact lists).
    // `get_event` returns global events (kind:0/1/3/30023) with scope checks
    // and channel events with membership verification. Unknown global kinds
    // return 404 (closed-default allowlist in events.rs).
    ("publish_note", "social", false),
    ("set_contact_list", "social", false),
    ("get_event", "social", true),
    ("get_user_notes", "social", true),
    ("get_contact_list", "social", true),
    // ── media ────────────────────────────────────────────────────────────────
    ("upload_file", "media", false),
];

/// Tools planned but not yet implemented. These will be added to ALL_TOOLS
/// when their #[tool] handlers are created in server.rs.
pub const DEFERRED_TOOLS: &[(&str, &str, bool)] = &[
    ("subscribe", "realtime", true),
    ("unsubscribe", "realtime", false),
];

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Access mode for a toolset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// All tools in the toolset (read + write).
    ReadWrite,
    /// Read-only tools only.
    ReadOnly,
}

/// Metadata about a toolset.
#[derive(Debug, Clone)]
pub struct ToolsetDef {
    /// Toolset name, e.g. `"channel_admin"`.
    pub name: &'static str,
    /// All tools belonging to this toolset.
    pub tools: &'static [ToolDef],
}

/// Metadata about a single tool.
#[derive(Debug, Clone, Copy)]
pub struct ToolDef {
    /// Tool name, e.g. `"get_messages"`.
    pub name: &'static str,
    /// Whether the tool is safe under `:ro` mode.
    pub is_read: bool,
}

/// Parsed toolset configuration.
///
/// Construct via [`ToolsetConfig::parse`] or [`ToolsetConfig::from_env`].
#[derive(Debug, Clone)]
pub struct ToolsetConfig {
    /// `toolset_name → Mode`. Only explicitly enabled toolsets appear here.
    enabled: HashMap<&'static str, Mode>,
}

// ---------------------------------------------------------------------------
// Known toolset names (compile-time set for validation)
// ---------------------------------------------------------------------------

const KNOWN_TOOLSETS: &[&str] = &[
    "default",
    "channel_admin",
    "dms",
    "canvas",
    "workflow_admin",
    "media",
    "realtime",
    "identity",
    "forums",
    "social",
];

// ---------------------------------------------------------------------------
// Lazy static toolset definitions (built from ALL_TOOLS)
// ---------------------------------------------------------------------------

static TOOLSET_DEFS: LazyLock<Vec<ToolsetDef>> = LazyLock::new(|| {
    let mut map: std::collections::BTreeMap<&'static str, Vec<ToolDef>> =
        std::collections::BTreeMap::new();
    for &(tool, ts, is_read) in ALL_TOOLS {
        map.entry(ts).or_default().push(ToolDef {
            name: tool,
            is_read,
        });
    }
    map.into_iter()
        .map(|(name, tools)| ToolsetDef {
            name,
            tools: Box::leak(tools.into_boxed_slice()),
        })
        .collect()
});

/// Returns all toolset definitions, built once from [`ALL_TOOLS`].
pub fn all_toolsets() -> &'static [ToolsetDef] {
    &TOOLSET_DEFS
}

/// Returns the tools belonging to `name`, or `None` if the toolset is unknown.
pub fn tools_in_toolset(name: &str) -> Option<Vec<ToolDef>> {
    let tools: Vec<ToolDef> = ALL_TOOLS
        .iter()
        .filter(|&&(_, ts, _)| ts == name)
        .map(|&(tool, _, is_read)| ToolDef {
            name: tool,
            is_read,
        })
        .collect();
    if tools.is_empty() {
        None
    } else {
        Some(tools)
    }
}

// ---------------------------------------------------------------------------
// ToolsetConfig implementation
// ---------------------------------------------------------------------------

impl ToolsetConfig {
    /// Parse a comma-separated toolset string.
    ///
    /// # Keywords
    /// - `default`  — enables the `default` toolset
    /// - `all`      — enables every toolset
    /// - `none`     — clears all enabled toolsets
    ///
    /// # Mode suffixes
    /// - `:ro`  — read-only (only tools with `is_read = true`)
    /// - `:rw`  — read-write (default)
    ///
    /// Later entries override earlier ones.
    pub fn parse(input: &str) -> Self {
        let mut enabled: HashMap<&'static str, Mode> = HashMap::new();

        for token in input.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            let (name, mode) = if let Some(n) = token.strip_suffix(":ro") {
                (n, Mode::ReadOnly)
            } else if let Some(n) = token.strip_suffix(":rw") {
                (n, Mode::ReadWrite)
            } else {
                (token, Mode::ReadWrite)
            };

            match name {
                "none" => {
                    enabled.clear();
                }
                "all" => {
                    for &ts in KNOWN_TOOLSETS {
                        enabled.insert(ts, mode);
                    }
                }
                "default" => {
                    enabled.insert("default", mode);
                }
                other => {
                    // Intern to &'static str if known; warn and skip if not.
                    if let Some(&known) = KNOWN_TOOLSETS.iter().find(|&&k| k == other) {
                        enabled.insert(known, mode);
                    } else {
                        eprintln!("sprout-mcp: unknown toolset {:?} — skipping", other);
                    }
                }
            }
        }

        Self { enabled }
    }

    /// Parse from `SPROUT_TOOLSETS`, falling back to `"default"`.
    ///
    /// An empty string (e.g. `SPROUT_TOOLSETS=""`) is treated the same as unset.
    pub fn from_env() -> Self {
        let raw = std::env::var("SPROUT_TOOLSETS")
            .ok()
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| "default".to_string());
        Self::parse(&raw)
    }

    /// Returns the set of tool names that should be **removed** from the router.
    ///
    /// Callers pass each name to `ToolRouter::remove_route()`.
    pub fn tools_to_remove(&self) -> HashSet<&'static str> {
        ALL_TOOLS
            .iter()
            .filter(|&&(_tool, ts, is_read)| {
                match self.enabled.get(ts) {
                    None => true,                     // toolset not enabled → remove
                    Some(Mode::ReadWrite) => false,   // fully enabled → keep
                    Some(Mode::ReadOnly) => !is_read, // ro → remove write tools
                }
            })
            .map(|&(tool, _, _)| tool)
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn enabled_tools(input: &str) -> HashSet<&'static str> {
        let cfg = ToolsetConfig::parse(input);
        let remove = cfg.tools_to_remove();
        ALL_TOOLS
            .iter()
            .map(|&(t, _, _)| t)
            .filter(|t| !remove.contains(t))
            .collect()
    }

    #[test]
    fn default_includes_26_tools() {
        let tools = enabled_tools("default");
        assert_eq!(tools.len(), 26);
        assert!(tools.contains("send_message"));
        assert!(tools.contains("approve_step"));
        assert!(!tools.contains("create_channel"));
    }

    #[test]
    fn none_removes_all_tools() {
        assert!(enabled_tools("none").is_empty());
    }

    #[test]
    fn all_includes_all_tools() {
        assert_eq!(enabled_tools("all").len(), ALL_TOOLS.len());
    }

    #[test]
    fn ro_keeps_only_read_tools() {
        let tools = enabled_tools("default:ro");
        // Every enabled tool must be a read tool
        for t in &tools {
            let is_read = ALL_TOOLS.iter().find(|&&(n, _, _)| n == *t).unwrap().2;
            assert!(is_read, "{t} should not be present in :ro mode");
        }
        assert!(tools.contains("get_messages"));
        assert!(!tools.contains("send_message"));
    }

    #[test]
    fn later_entry_overrides_earlier() {
        // all:ro then default:rw → default tools are rw, rest are ro
        let cfg = ToolsetConfig::parse("all:ro,default:rw");
        let remove = cfg.tools_to_remove();
        // send_message is default+write → should be kept (rw)
        assert!(!remove.contains("send_message"));
        // create_channel is channel_admin+write → should be removed (ro)
        assert!(remove.contains("create_channel"));
        // list_channel_members is default+read → should be kept (rw)
        assert!(!remove.contains("list_channel_members"));
    }

    #[test]
    fn unknown_toolset_is_skipped_gracefully() {
        // Should not panic; unknown toolset is silently ignored
        let tools = enabled_tools("default,nonexistent_toolset");
        assert_eq!(tools.len(), 26); // only default
    }

    #[test]
    fn empty_input_enables_nothing() {
        assert!(enabled_tools("").is_empty());
    }

    #[test]
    fn none_after_all_clears() {
        assert!(enabled_tools("all,none").is_empty());
    }

    #[test]
    fn rw_suffix_is_same_as_bare() {
        assert_eq!(enabled_tools("default:rw"), enabled_tools("default"));
    }

    #[test]
    fn all_tools_count_is_49() {
        assert_eq!(ALL_TOOLS.len(), 49);
    }

    #[test]
    fn deferred_tools_count_is_2() {
        assert_eq!(DEFERRED_TOOLS.len(), 2);
    }

    #[test]
    fn tools_in_toolset_returns_correct_tools() {
        let tools = tools_in_toolset("canvas").unwrap();
        assert_eq!(tools.len(), 2);
        let names: Vec<_> = tools.iter().map(|t| t.name).collect();
        assert!(names.contains(&"get_canvas"));
        assert!(names.contains(&"set_canvas"));
    }

    #[test]
    fn tools_in_toolset_unknown_returns_none() {
        assert!(tools_in_toolset("bogus").is_none());
    }

    #[test]
    fn all_toolsets_returns_correct_count() {
        // ALL_TOOLS covers: default, channel_admin, dms, canvas, workflow_admin, identity, forums, social, media
        // (realtime has no implemented tools yet)
        let defs = all_toolsets();
        assert_eq!(defs.len(), 9);
        let names: Vec<_> = defs.iter().map(|d| d.name).collect();
        assert!(names.contains(&"default"));
        assert!(names.contains(&"canvas"));
        assert!(names.contains(&"forums"));
        assert!(names.contains(&"social"));
        assert!(names.contains(&"media"));
    }

    // ── Cross-check: ALL_TOOLS integrity ────────────────────────────────────

    #[test]
    fn all_tools_has_no_duplicates() {
        let mut seen = std::collections::HashSet::new();
        for &(name, _, _) in ALL_TOOLS {
            assert!(
                seen.insert(name),
                "duplicate tool name in ALL_TOOLS: {name}"
            );
        }
    }

    #[test]
    fn all_tools_names_are_valid_identifiers() {
        for &(name, _, _) in ALL_TOOLS {
            assert!(!name.is_empty(), "empty tool name in ALL_TOOLS");
            assert!(
                name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'),
                "invalid tool name in ALL_TOOLS: {name}"
            );
        }
    }

    // ── from_env empty-string fallback ──────────────────────────────────────

    #[test]
    fn parse_empty_string_enables_nothing() {
        // parse("") is the raw parser — empty input → no toolsets enabled.
        // from_env() adds the fallback before calling parse, so agents always
        // get at least the default toolset even when SPROUT_TOOLSETS="".
        assert!(enabled_tools("").is_empty());
    }
}
