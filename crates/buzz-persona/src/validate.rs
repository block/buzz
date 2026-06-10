//! Pack validation (`sprout pack validate`).
//!
//! Architecture: the validator delegates all structural checks to `load_pack()`.
//! If loading succeeds, the pack is structurally valid by definition — no
//! duplicate parsing, no contract drift.
//!
//! On top of the load, advisory checks inspect raw files for things the typed
//! parsers silently drop (unknown keys, naming conventions).
//!
//! Exit semantics: errors are hard failures, warnings are advisory.

use std::collections::HashSet;
use std::path::Path;

use crate::pack;

// ── Diagnostics ──────────────────────────────────────────────────────────────

/// A single validation finding.
#[derive(Debug, Clone)]
pub enum ValidationDiagnostic {
    Error(String),
    Warning(String),
}

impl std::fmt::Display for ValidationDiagnostic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Error(msg) => write!(f, "ERROR: {msg}"),
            Self::Warning(msg) => write!(f, "WARN:  {msg}"),
        }
    }
}

/// Result of validating a pack.
#[derive(Debug, Default)]
pub struct ValidationReport {
    pub diagnostics: Vec<ValidationDiagnostic>,
}

impl ValidationReport {
    pub fn error(&mut self, msg: impl Into<String>) {
        self.diagnostics
            .push(ValidationDiagnostic::Error(msg.into()));
    }

    pub fn warn(&mut self, msg: impl Into<String>) {
        self.diagnostics
            .push(ValidationDiagnostic::Warning(msg.into()));
    }

    pub fn has_errors(&self) -> bool {
        self.diagnostics
            .iter()
            .any(|d| matches!(d, ValidationDiagnostic::Error(_)))
    }

    pub fn has_warnings(&self) -> bool {
        self.diagnostics
            .iter()
            .any(|d| matches!(d, ValidationDiagnostic::Warning(_)))
    }

    /// Exit code: 0 = clean, 1 = errors, 2 = warnings only.
    pub fn exit_code(&self) -> i32 {
        if self.has_errors() {
            1
        } else if self.has_warnings() {
            2
        } else {
            0
        }
    }
}

impl std::fmt::Display for ValidationReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.diagnostics.is_empty() {
            writeln!(f, "✓ Pack is valid.")?;
        } else {
            for d in &self.diagnostics {
                writeln!(f, "  {d}")?;
            }
            let errors = self
                .diagnostics
                .iter()
                .filter(|d| matches!(d, ValidationDiagnostic::Error(_)))
                .count();
            let warnings = self
                .diagnostics
                .iter()
                .filter(|d| matches!(d, ValidationDiagnostic::Warning(_)))
                .count();
            writeln!(f, "\n{errors} error(s), {warnings} warning(s).")?;
        }
        Ok(())
    }
}

// ── Known field sets ─────────────────────────────────────────────────────────

/// Known top-level keys in `plugin.json`.
const KNOWN_MANIFEST_KEYS: &[&str] = &[
    // OPS standard fields
    "$schema",
    "id",
    "name",
    "version",
    "description",
    "author",
    "license",
    "homepage",
    "repository",
    "keywords",
    "engines",
    // Persona pack extensions
    "personas",
    "defaults",
    "pack_instructions",
    "hooks_config",
    "mcp_config",
];

/// Valid keys in the `defaults` block and persona behavioral config.
const KNOWN_BEHAVIORAL_KEYS: &[&str] = &[
    "subscribe",
    "triggers",
    "respond_to", // legacy alias — still accepted
    "model",
    "temperature",
    "max_context_tokens",
    "thread_replies",
    "broadcast_replies",
];

/// Valid sub-keys in `respond_to`.
const KNOWN_RESPOND_TO_KEYS: &[&str] = &["mentions", "keywords", "all_messages"];

// ── Pack validation ──────────────────────────────────────────────────────────

/// Validate a persona pack directory.
///
/// Step 1: delegate all structural validation to `load_pack()`. If loading
/// fails, the pack is broken — report the error and return.
///
/// Step 2: run advisory checks on the loaded pack. These inspect raw files
/// for naming drift and unknown manifest keys. Unknown manifest keys are
/// reported as warnings (likely typos); naming mismatches are also warnings.
pub fn validate_pack(pack_dir: &Path) -> ValidationReport {
    let mut report = ValidationReport::default();

    // Step 1: structural validation via the loader.
    let loaded = match pack::load_pack(pack_dir) {
        Ok(pack) => pack,
        Err(e) => {
            report.error(format!("pack failed to load: {e}"));
            return report;
        }
    };

    // Step 2: semantic checks on the loaded pack.
    semantic_check_personas(&loaded, &mut report);

    // Step 3: advisory checks on raw files.
    advisory_check_manifest_keys(pack_dir, &mut report);
    advisory_check_respond_to_types(pack_dir, &mut report);
    advisory_check_skill_names(pack_dir, &loaded, &mut report);

    report
}

// ── Semantic: persona-level checks ───────────────────────────────────────────

/// Validate a persona `name` field: `[a-zA-Z0-9_-]+`, max 64 chars.
fn validate_persona_name(name: &str, report: &mut ValidationReport) {
    const MAX_NAME_LEN: usize = 64;
    if name.len() > MAX_NAME_LEN {
        report.error(format!(
            "persona name \"{name}\" exceeds {MAX_NAME_LEN} characters (got {})",
            name.len()
        ));
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        report.error(format!(
            "persona name \"{name}\" contains invalid characters (allowed: [a-zA-Z0-9_-])"
        ));
    }
}

/// Check for zero-persona packs, duplicate persona names, and name validity.
/// These are hard errors — the pack is logically broken.
fn semantic_check_personas(loaded: &pack::LoadedPack, report: &mut ValidationReport) {
    // Zero personas: a pack with no personas is useless.
    if loaded.personas.is_empty() {
        report.error("pack contains zero personas");
        return; // no point checking duplicates or names
    }

    // Duplicate persona names: names must be unique within a pack.
    let mut seen = HashSet::new();
    for persona in &loaded.personas {
        if !seen.insert(&persona.name) {
            report.error(format!("duplicate persona name \"{}\"", persona.name));
        }
        // Name character and length validation.
        validate_persona_name(&persona.name, report);
    }
}

// ── Advisory: respond_to type validation ────────────────────────────────────

/// Check `defaults.respond_to` sub-key types in the raw `plugin.json`.
///
/// The typed parser (serde) already rejects wrong types in persona frontmatter,
/// but the defaults block deserializes more permissively. This advisory check
/// inspects the raw JSON to catch type mismatches early with clear messages.
fn advisory_check_respond_to_types(pack_dir: &Path, report: &mut ValidationReport) {
    let manifest_path = pack_dir.join(".plugin").join("plugin.json");
    let content = match std::fs::read_to_string(&manifest_path) {
        Ok(c) => c,
        Err(_) => return,
    };
    let json: serde_json::Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(_) => return,
    };

    // Check defaults.triggers (or legacy alias defaults.respond_to)
    if let Some(defaults) = json.get("defaults").and_then(|v| v.as_object()) {
        if let Some(rt) = defaults
            .get("triggers")
            .or_else(|| defaults.get("respond_to"))
        {
            check_respond_to_value(rt, "defaults.triggers", report);
        }
    }
}

/// Validate respond_to sub-key types.
/// - `mentions` must be bool
/// - `keywords` must be array of strings
/// - `all_messages` must be bool
fn check_respond_to_value(rt: &serde_json::Value, context: &str, report: &mut ValidationReport) {
    let obj = match rt.as_object() {
        Some(o) => o,
        None => {
            report.error(format!(
                "{context}: expected object, got {}",
                value_type_name(rt)
            ));
            return;
        }
    };

    if let Some(v) = obj.get("mentions") {
        if !v.is_boolean() {
            report.error(format!(
                "{context}.mentions: expected bool, got {}",
                value_type_name(v)
            ));
        }
    }

    if let Some(v) = obj.get("keywords") {
        match v.as_array() {
            Some(arr) => {
                for (i, item) in arr.iter().enumerate() {
                    if !item.is_string() {
                        report.error(format!(
                            "{context}.keywords[{i}]: expected string, got {}",
                            value_type_name(item)
                        ));
                    }
                }
            }
            None => {
                report.error(format!(
                    "{context}.keywords: expected array, got {}",
                    value_type_name(v)
                ));
            }
        }
    }

    if let Some(v) = obj.get("all_messages") {
        if !v.is_boolean() {
            report.error(format!(
                "{context}.all_messages: expected bool, got {}",
                value_type_name(v)
            ));
        }
    }
}

/// Human-readable JSON value type name.
fn value_type_name(v: &serde_json::Value) -> &'static str {
    match v {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "bool",
        serde_json::Value::Number(_) => "number",
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    }
}

// ── Advisory: unknown manifest keys ─────────────────────────────────────────

/// Check `plugin.json` for unknown top-level keys and unknown keys in
/// `defaults` / `defaults.respond_to`. Emits warnings (likely typos).
fn advisory_check_manifest_keys(pack_dir: &Path, report: &mut ValidationReport) {
    let manifest_path = pack_dir.join(".plugin").join("plugin.json");
    let content = match std::fs::read_to_string(&manifest_path) {
        Ok(c) => c,
        Err(_) => return, // load_pack already caught this
    };
    let json: serde_json::Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(_) => return, // load_pack already caught this
    };
    let obj = match json.as_object() {
        Some(o) => o,
        None => return,
    };

    // Top-level unknown keys.
    let known_manifest: HashSet<&str> = KNOWN_MANIFEST_KEYS.iter().copied().collect();
    for key in obj.keys() {
        if !known_manifest.contains(key.as_str()) {
            report.warn(format!("plugin.json unknown key \"{key}\""));
        }
    }

    // Unknown keys in `defaults`.
    if let Some(defaults) = obj.get("defaults").and_then(|v| v.as_object()) {
        let known_behavioral: HashSet<&str> = KNOWN_BEHAVIORAL_KEYS.iter().copied().collect();
        for key in defaults.keys() {
            if !known_behavioral.contains(key.as_str()) {
                report.warn(format!("plugin.json defaults: unknown key \"{key}\""));
            }
        }

        // Unknown keys in `defaults.triggers` (or legacy `defaults.respond_to`).
        let triggers_obj = defaults
            .get("triggers")
            .or_else(|| defaults.get("respond_to"))
            .and_then(|v| v.as_object());
        if let Some(rt) = triggers_obj {
            let known_rt: HashSet<&str> = KNOWN_RESPOND_TO_KEYS.iter().copied().collect();
            for key in rt.keys() {
                if !known_rt.contains(key.as_str()) {
                    report.warn(format!(
                        "plugin.json defaults.triggers: unknown key \"{key}\""
                    ));
                }
            }
        }
    }
}

// ── Advisory: skill naming conventions ──────────────────────────────────────

/// For each skill directory referenced by a loaded persona, check that the
/// SKILL.md `name:` field matches the directory name. Emits warnings.
fn advisory_check_skill_names(
    pack_dir: &Path,
    loaded: &pack::LoadedPack,
    report: &mut ValidationReport,
) {
    // Collect all skill paths referenced by any persona.
    let mut skill_paths: Vec<std::path::PathBuf> = Vec::new();
    for persona in &loaded.personas {
        for skill_ref in &persona.skills {
            // Normalize: strip trailing slash, take final component.
            let skill_name = std::path::Path::new(skill_ref.trim_end_matches('/'))
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(skill_ref.as_str())
                .to_owned();
            let candidate = pack_dir.join("skills").join(&skill_name);
            if candidate.is_dir() && !skill_paths.contains(&candidate) {
                skill_paths.push(candidate);
            }
        }
    }

    // Also check skills dir for any directories not claimed by personas.
    let skills_dir = pack_dir.join("skills");
    if skills_dir.is_dir() {
        if let Ok(entries) = std::fs::read_dir(&skills_dir) {
            for entry in entries.flatten() {
                if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                    let p = entry.path();
                    if !skill_paths.contains(&p) {
                        skill_paths.push(p);
                    }
                }
            }
        }
    }

    for skill_dir in &skill_paths {
        let skill_md = skill_dir.join("SKILL.md");
        if !skill_md.exists() {
            continue; // load_pack handles missing SKILL.md if it's required
        }

        let content = match std::fs::read_to_string(&skill_md) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let (fm_str, _) = match crate::persona::split_frontmatter(&content).ok() {
            Some(parts) => parts,
            None => continue,
        };

        let yaml_val: serde_yaml::Value = match serde_yaml::from_str(fm_str) {
            Ok(v) => v,
            Err(_) => continue,
        };

        let mapping = match yaml_val.as_mapping() {
            Some(m) => m,
            None => continue,
        };

        if let Some(name_val) = mapping.get(serde_yaml::Value::String("name".into())) {
            if let Some(name_str) = name_val.as_str() {
                if let Some(dir_name) = skill_dir.file_name().and_then(|n| n.to_str()) {
                    if name_str != dir_name {
                        let label = skill_dir
                            .strip_prefix(pack_dir)
                            .unwrap_or(skill_dir)
                            .display()
                            .to_string();
                        report.warn(format!(
                            "skill {label}: name \"{name_str}\" differs from directory name \"{dir_name}\""
                        ));
                    }
                }
            }
        }
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── ValidationReport unit tests ──────────────────────────────────────────

    #[test]
    fn exit_code_clean() {
        let report = ValidationReport::default();
        assert_eq!(report.exit_code(), 0);
    }

    #[test]
    fn exit_code_errors() {
        let mut report = ValidationReport::default();
        report.error("bad");
        assert_eq!(report.exit_code(), 1);
    }

    #[test]
    fn exit_code_warnings_only() {
        let mut report = ValidationReport::default();
        report.warn("meh");
        assert_eq!(report.exit_code(), 2);
    }

    // ── Filesystem integration tests ─────────────────────────────────────────

    /// Minimal valid pack: load succeeds, no advisory issues.
    #[test]
    fn validate_pack_minimal_valid() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().to_path_buf();
        std::fs::create_dir_all(dir.join(".plugin")).unwrap();
        std::fs::create_dir_all(dir.join("agents")).unwrap();

        std::fs::write(
            dir.join(".plugin/plugin.json"),
            r#"{
                "id": "com.test.minimal",
                "name": "Minimal Pack",
                "version": "0.1.0",
                "personas": ["agents/test.persona.md"]
            }"#,
        )
        .unwrap();

        std::fs::write(
            dir.join("agents/test.persona.md"),
            "---\nname: \"test\"\ndisplay_name: \"Test Agent\"\ndescription: \"A test agent\"\n---\n\nYou are a test agent.\n",
        )
        .unwrap();

        let report = validate_pack(&dir);
        assert!(
            !report.has_errors(),
            "expected clean validation, got: {report}"
        );
        assert_eq!(report.exit_code(), 0);
    }

    /// Missing plugin.json: load_pack fails → error reported.
    #[test]
    fn validate_pack_missing_manifest() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().to_path_buf();

        let report = validate_pack(&dir);
        assert!(report.has_errors());
        let msg = format!("{report}");
        assert!(
            msg.contains("plugin.json") || msg.contains("load"),
            "got: {msg}"
        );
    }

    /// Path traversal in personas list: load_pack rejects it → error.
    #[test]
    fn validate_pack_persona_path_traversal() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().to_path_buf();
        std::fs::create_dir_all(dir.join(".plugin")).unwrap();

        std::fs::write(
            dir.join(".plugin/plugin.json"),
            r#"{
                "id": "com.test.evil",
                "name": "Evil Pack",
                "version": "0.1.0",
                "personas": ["../../etc/passwd"]
            }"#,
        )
        .unwrap();

        let report = validate_pack(&dir);
        assert!(report.has_errors());
    }

    /// Unknown key in defaults: advisory check emits warning.
    #[test]
    fn validate_pack_unknown_defaults_key() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().to_path_buf();
        std::fs::create_dir_all(dir.join(".plugin")).unwrap();
        std::fs::create_dir_all(dir.join("agents")).unwrap();

        std::fs::write(
            dir.join(".plugin/plugin.json"),
            r#"{
                "id": "com.test.typo",
                "name": "Typo Pack",
                "version": "0.1.0",
                "personas": ["agents/t.persona.md"],
                "defaults": { "temprature": 0.5 }
            }"#,
        )
        .unwrap();
        std::fs::write(
            dir.join("agents/t.persona.md"),
            "---\nname: t\ndisplay_name: T\ndescription: T.\n---\n",
        )
        .unwrap();

        let report = validate_pack(&dir);
        assert!(!report.has_errors(), "advisory checks should not be errors");
        assert!(report.has_warnings(), "expected warning for unknown key");
        let msg = format!("{report}");
        assert!(msg.contains("temprature"), "got: {msg}");
    }

    /// OPS standard fields should NOT trigger unknown key errors.
    #[test]
    fn validate_pack_ops_fields_accepted() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().to_path_buf();
        std::fs::create_dir_all(dir.join(".plugin")).unwrap();
        std::fs::create_dir_all(dir.join("agents")).unwrap();

        std::fs::write(
            dir.join(".plugin/plugin.json"),
            r#"{
                "$schema": "https://open-plugin-spec.org/schema/v1/plugin.json",
                "id": "com.test.ops",
                "name": "OPS Fields Pack",
                "version": "0.1.0",
                "personas": ["agents/t.persona.md"],
                "license": "MIT",
                "homepage": "https://example.com",
                "repository": "https://github.com/example/pack",
                "keywords": ["test"],
                "engines": { "sprout": ">=0.9.0" }
            }"#,
        )
        .unwrap();
        std::fs::write(
            dir.join("agents/t.persona.md"),
            "---\nname: t\ndisplay_name: T\ndescription: T.\n---\n",
        )
        .unwrap();

        let report = validate_pack(&dir);
        let key_errors: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|d| {
                if let ValidationDiagnostic::Error(msg) = d {
                    msg.contains("unknown key")
                } else {
                    false
                }
            })
            .collect();
        assert!(
            key_errors.is_empty(),
            "OPS fields should not trigger unknown key errors: {key_errors:?}"
        );
    }

    /// Unknown top-level key in plugin.json: advisory check emits warning.
    #[test]
    fn validate_pack_unknown_manifest_key() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().to_path_buf();
        std::fs::create_dir_all(dir.join(".plugin")).unwrap();
        std::fs::create_dir_all(dir.join("agents")).unwrap();

        std::fs::write(
            dir.join(".plugin/plugin.json"),
            r#"{
                "id": "com.test.bogus",
                "name": "Bogus Key Pack",
                "version": "0.1.0",
                "personas": ["agents/t.persona.md"],
                "totally_made_up": true
            }"#,
        )
        .unwrap();
        std::fs::write(
            dir.join("agents/t.persona.md"),
            "---\nname: t\ndisplay_name: T\ndescription: T.\n---\n",
        )
        .unwrap();

        let report = validate_pack(&dir);
        assert!(!report.has_errors(), "advisory checks should not be errors");
        assert!(report.has_warnings(), "expected warning for unknown key");
        let msg = format!("{report}");
        assert!(
            msg.contains("totally_made_up"),
            "expected unknown key warning for 'totally_made_up', got: {msg}"
        );
    }

    /// Persona missing required fields: load_pack fails → error.
    #[test]
    fn validate_pack_persona_missing_required_fields() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().to_path_buf();
        std::fs::create_dir_all(dir.join(".plugin")).unwrap();
        std::fs::create_dir_all(dir.join("agents")).unwrap();

        std::fs::write(
            dir.join(".plugin/plugin.json"),
            r#"{"id":"t","name":"T","version":"0.1.0","personas":["agents/t.persona.md"]}"#,
        )
        .unwrap();
        // Missing display_name and description.
        std::fs::write(
            dir.join("agents/t.persona.md"),
            "---\nname: \"bad\"\n---\nNo display_name or description.\n",
        )
        .unwrap();

        let report = validate_pack(&dir);
        assert!(report.has_errors());
    }

    /// Persona with no frontmatter: load_pack fails → error.
    #[test]
    fn validate_pack_persona_no_frontmatter() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().to_path_buf();
        std::fs::create_dir_all(dir.join(".plugin")).unwrap();
        std::fs::create_dir_all(dir.join("agents")).unwrap();

        std::fs::write(
            dir.join(".plugin/plugin.json"),
            r#"{
                "id": "com.test.bad-fm",
                "name": "Bad Frontmatter Pack",
                "version": "0.1.0",
                "personas": ["agents/broken.persona.md"]
            }"#,
        )
        .unwrap();

        std::fs::write(
            dir.join("agents/broken.persona.md"),
            "This file has no frontmatter at all.\nJust plain markdown.\n",
        )
        .unwrap();

        let report = validate_pack(&dir);
        assert!(report.has_errors());
    }

    /// Persona with leading whitespace before ---: load_pack fails → error.
    #[test]
    fn validate_pack_persona_leading_whitespace() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().to_path_buf();
        std::fs::create_dir_all(dir.join(".plugin")).unwrap();
        std::fs::create_dir_all(dir.join("agents")).unwrap();

        std::fs::write(
            dir.join(".plugin/plugin.json"),
            r#"{
                "id": "com.test.ws",
                "name": "Whitespace Pack",
                "version": "0.1.0",
                "personas": ["agents/ws.persona.md"]
            }"#,
        )
        .unwrap();

        std::fs::write(
            dir.join("agents/ws.persona.md"),
            "\n---\nname: \"test\"\ndisplay_name: \"Test\"\ndescription: \"A test\"\n---\n\nPrompt.\n",
        )
        .unwrap();

        let report = validate_pack(&dir);
        assert!(
            report.has_errors(),
            "expected error for leading whitespace before ---, got clean validation"
        );
    }

    /// Unknown frontmatter key in persona: advisory check emits error.
    #[test]
    fn validate_pack_persona_unknown_key() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().to_path_buf();
        std::fs::create_dir_all(dir.join(".plugin")).unwrap();
        std::fs::create_dir_all(dir.join("agents")).unwrap();

        std::fs::write(
            dir.join(".plugin/plugin.json"),
            r#"{"id":"t","name":"T","version":"0.1.0","personas":["agents/t.persona.md"]}"#,
        )
        .unwrap();
        std::fs::write(
            dir.join("agents/t.persona.md"),
            "---\nname: t\ndisplay_name: T\ndescription: T.\nzomg_unknown: true\n---\n",
        )
        .unwrap();

        let report = validate_pack(&dir);
        assert!(report.has_errors());
        let msg = format!("{report}");
        assert!(msg.contains("zomg_unknown"), "got: {msg}");
    }

    /// Skill name mismatch: advisory check emits warning.
    #[test]
    fn validate_pack_skill_name_mismatch_warns() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().to_path_buf();
        std::fs::create_dir_all(dir.join(".plugin")).unwrap();
        std::fs::create_dir_all(dir.join("agents")).unwrap();
        std::fs::create_dir_all(dir.join("skills/code-review")).unwrap();

        std::fs::write(
            dir.join(".plugin/plugin.json"),
            r#"{"id":"t","name":"T","version":"0.1.0","personas":["agents/t.persona.md"]}"#,
        )
        .unwrap();
        std::fs::write(
            dir.join("agents/t.persona.md"),
            "---\nname: t\ndisplay_name: T\ndescription: T.\nskills:\n  - skills/code-review\n---\n",
        )
        .unwrap();
        // SKILL.md name doesn't match directory name "code-review".
        std::fs::write(
            dir.join("skills/code-review/SKILL.md"),
            "---\nname: code_review\ndescription: Reviews code.\n---\nDoes code review.\n",
        )
        .unwrap();

        let report = validate_pack(&dir);
        assert!(!report.has_errors(), "should be no errors, got: {report}");
        assert!(report.has_warnings(), "expected naming mismatch warning");
        let msg = format!("{report}");
        assert!(msg.contains("code_review"), "got: {msg}");
    }

    // ── Semantic: zero-persona and duplicate-name checks ─────────────────────

    /// Zero personas in manifest → hard error.
    #[test]
    fn validate_zero_personas_error() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().to_path_buf();
        std::fs::create_dir_all(dir.join(".plugin")).unwrap();

        std::fs::write(
            dir.join(".plugin/plugin.json"),
            r#"{
                "id": "com.test.empty",
                "name": "Empty Pack",
                "version": "0.1.0",
                "personas": []
            }"#,
        )
        .unwrap();

        let report = validate_pack(&dir);
        assert!(report.has_errors(), "zero-persona pack should be an error");
        let msg = format!("{report}");
        assert!(
            msg.contains("zero personas"),
            "error should mention zero personas, got: {msg}"
        );
    }

    /// Duplicate persona names → hard error.
    #[test]
    fn validate_duplicate_persona_names_error() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().to_path_buf();
        std::fs::create_dir_all(dir.join(".plugin")).unwrap();
        std::fs::create_dir_all(dir.join("agents")).unwrap();

        std::fs::write(
            dir.join(".plugin/plugin.json"),
            r#"{
                "id": "com.test.dupes",
                "name": "Dupe Pack",
                "version": "0.1.0",
                "personas": ["agents/a.persona.md", "agents/b.persona.md"]
            }"#,
        )
        .unwrap();
        // Both personas have the same name "bot".
        std::fs::write(
            dir.join("agents/a.persona.md"),
            "---\nname: bot\ndisplay_name: Bot A\ndescription: First bot.\n---\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("agents/b.persona.md"),
            "---\nname: bot\ndisplay_name: Bot B\ndescription: Second bot.\n---\n",
        )
        .unwrap();

        let report = validate_pack(&dir);
        assert!(report.has_errors(), "duplicate names should be an error");
        let msg = format!("{report}");
        assert!(
            msg.contains("duplicate persona name") && msg.contains("bot"),
            "error should mention duplicate name 'bot', got: {msg}"
        );
    }

    /// Unique persona names → no duplicate error.
    #[test]
    fn validate_unique_persona_names_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().to_path_buf();
        std::fs::create_dir_all(dir.join(".plugin")).unwrap();
        std::fs::create_dir_all(dir.join("agents")).unwrap();

        std::fs::write(
            dir.join(".plugin/plugin.json"),
            r#"{
                "id": "com.test.unique",
                "name": "Unique Pack",
                "version": "0.1.0",
                "personas": ["agents/a.persona.md", "agents/b.persona.md"]
            }"#,
        )
        .unwrap();
        std::fs::write(
            dir.join("agents/a.persona.md"),
            "---\nname: alpha\ndisplay_name: Alpha\ndescription: First.\n---\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("agents/b.persona.md"),
            "---\nname: beta\ndisplay_name: Beta\ndescription: Second.\n---\n",
        )
        .unwrap();

        let report = validate_pack(&dir);
        assert!(
            !report.has_errors(),
            "unique names should not cause errors, got: {report}"
        );
    }

    // ── Semantic: name character and length validation ────────────────────────

    /// Persona name with spaces or slashes → hard error.
    #[test]
    fn validate_name_invalid_chars() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().to_path_buf();
        std::fs::create_dir_all(dir.join(".plugin")).unwrap();
        std::fs::create_dir_all(dir.join("agents")).unwrap();

        std::fs::write(
            dir.join(".plugin/plugin.json"),
            r#"{"id":"t","name":"T","version":"0.1.0","personas":["agents/t.persona.md"]}"#,
        )
        .unwrap();
        // Name contains a space — invalid.
        std::fs::write(
            dir.join("agents/t.persona.md"),
            "---\nname: \"my bot\"\ndisplay_name: T\ndescription: T.\n---\n",
        )
        .unwrap();

        let report = validate_pack(&dir);
        assert!(report.has_errors(), "name with spaces should be an error");
        let msg = format!("{report}");
        assert!(
            msg.contains("invalid characters"),
            "error should mention invalid characters, got: {msg}"
        );
    }

    /// Persona name with 65+ characters → hard error.
    #[test]
    fn validate_name_too_long() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().to_path_buf();
        std::fs::create_dir_all(dir.join(".plugin")).unwrap();
        std::fs::create_dir_all(dir.join("agents")).unwrap();

        std::fs::write(
            dir.join(".plugin/plugin.json"),
            r#"{"id":"t","name":"T","version":"0.1.0","personas":["agents/t.persona.md"]}"#,
        )
        .unwrap();
        // 65-character name — one over the limit.
        let long_name = "a".repeat(65);
        let persona_content =
            format!("---\nname: \"{long_name}\"\ndisplay_name: T\ndescription: T.\n---\n");
        std::fs::write(dir.join("agents/t.persona.md"), persona_content).unwrap();

        let report = validate_pack(&dir);
        assert!(report.has_errors(), "65-char name should be an error");
        let msg = format!("{report}");
        assert!(
            msg.contains("exceeds") && msg.contains("64"),
            "error should mention 64-char limit, got: {msg}"
        );
    }

    // ── Advisory: respond_to type validation ─────────────────────────────────

    /// respond_to with wrong types in defaults → caught by typed parser.
    /// The manifest's BehavioralDefaults uses typed RespondTo, so serde_json
    /// rejects wrong types during load_pack(). This surfaces as a load error.
    #[test]
    fn validate_respond_to_bad_types() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().to_path_buf();
        std::fs::create_dir_all(dir.join(".plugin")).unwrap();
        std::fs::create_dir_all(dir.join("agents")).unwrap();

        // mentions should be bool, not string — serde catches this at parse time.
        std::fs::write(
            dir.join(".plugin/plugin.json"),
            r#"{
                "id": "com.test.bad-rt",
                "name": "Bad RT Pack",
                "version": "0.1.0",
                "personas": ["agents/t.persona.md"],
                "defaults": {
                    "respond_to": {
                        "mentions": "yes",
                        "keywords": ["security"],
                        "all_messages": false
                    }
                }
            }"#,
        )
        .unwrap();
        std::fs::write(
            dir.join("agents/t.persona.md"),
            "---\nname: t\ndisplay_name: T\ndescription: T.\n---\n",
        )
        .unwrap();

        let report = validate_pack(&dir);
        assert!(report.has_errors(), "bad respond_to types should be errors");
        let msg = format!("{report}");
        // Serde catches the type mismatch during manifest parsing.
        assert!(
            msg.contains("mentions") || msg.contains("invalid type"),
            "should flag mentions type error, got: {msg}"
        );
    }

    /// respond_to with correct types → no type errors.
    #[test]
    fn validate_respond_to_correct_types_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().to_path_buf();
        std::fs::create_dir_all(dir.join(".plugin")).unwrap();
        std::fs::create_dir_all(dir.join("agents")).unwrap();

        std::fs::write(
            dir.join(".plugin/plugin.json"),
            r#"{
                "id": "com.test.good-rt",
                "name": "Good RT Pack",
                "version": "0.1.0",
                "personas": ["agents/t.persona.md"],
                "defaults": {
                    "respond_to": {
                        "mentions": true,
                        "keywords": ["security", "CVE"],
                        "all_messages": false
                    }
                }
            }"#,
        )
        .unwrap();
        std::fs::write(
            dir.join("agents/t.persona.md"),
            "---\nname: t\ndisplay_name: T\ndescription: T.\n---\n",
        )
        .unwrap();

        let report = validate_pack(&dir);
        assert!(
            !report.has_errors(),
            "correct respond_to types should not cause errors, got: {report}"
        );
    }

    /// respond_to with non-string items in keywords array → caught by serde.
    #[test]
    fn validate_respond_to_keywords_non_string_items() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().to_path_buf();
        std::fs::create_dir_all(dir.join(".plugin")).unwrap();
        std::fs::create_dir_all(dir.join("agents")).unwrap();

        std::fs::write(
            dir.join(".plugin/plugin.json"),
            r#"{
                "id": "com.test.bad-kw",
                "name": "Bad KW Pack",
                "version": "0.1.0",
                "personas": ["agents/t.persona.md"],
                "defaults": {
                    "respond_to": {
                        "keywords": ["valid", 42, true]
                    }
                }
            }"#,
        )
        .unwrap();
        std::fs::write(
            dir.join("agents/t.persona.md"),
            "---\nname: t\ndisplay_name: T\ndescription: T.\n---\n",
        )
        .unwrap();

        let report = validate_pack(&dir);
        assert!(
            report.has_errors(),
            "non-string keyword items should be errors"
        );
        let msg = format!("{report}");
        // Serde catches the type mismatch in the keywords array.
        assert!(
            msg.contains("keywords") || msg.contains("invalid type"),
            "should flag keywords type error, got: {msg}"
        );
    }
}
