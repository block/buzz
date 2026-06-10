/// Pack directory loader.
///
/// Reads a pack directory and produces a fully loaded [`LoadedPack`].
///
/// Directory layout expected:
/// ```text
/// <pack_root>/
///   .plugin/
///     plugin.json          ← manifest
///   personas/
///     <name>.persona.md    ← one file per persona
///   instructions.md        ← optional pack-level instructions
///   .mcp.json              ← optional shared MCP config
///   skills/                ← optional skills directory
/// ```
use std::{
    collections::HashMap,
    path::{Component, Path, PathBuf},
};

use crate::manifest::{self, ManifestError};
use crate::merge::{resolve_persona_config, HooksData, TriggersData};
use crate::persona::{self, PersonaConfig};

// ── Error ─────────────────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum PackError {
    #[error("manifest not found at {0}")]
    ManifestNotFound(PathBuf),

    #[error("failed to read {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to parse manifest: {0}")]
    ManifestParse(String),

    #[error("persona file not found: {0}")]
    PersonaNotFound(PathBuf),

    #[error("invalid file {path}: {reason}")]
    FileParse { path: PathBuf, reason: String },

    #[error("path traversal rejected: {0}")]
    PathTraversal(String),

    #[error("path escapes pack root: {0}")]
    PathEscape(PathBuf),

    #[error("failed to parse .mcp.json at {path}: {reason}")]
    McpConfigParse { path: PathBuf, reason: String },
}

impl From<ManifestError> for PackError {
    fn from(e: ManifestError) -> Self {
        PackError::ManifestParse(e.to_string())
    }
}

// ── Public types ──────────────────────────────────────────────────────────────

/// A fully loaded persona pack.
#[derive(Debug)]
pub struct LoadedPack {
    pub manifest: PackManifestData,
    pub personas: Vec<LoadedPersona>,
    /// Content of instructions.md, if present.
    pub pack_instructions: Option<String>,
    /// Raw .mcp.json content, if present.
    pub shared_mcp_config: Option<serde_json::Value>,
    /// Path to the skills/ directory, if it exists.
    pub skills_dir: Option<PathBuf>,
}

/// A persona with its resolved effective config.
#[derive(Debug)]
pub struct LoadedPersona {
    pub source_path: PathBuf,
    pub name: String,
    pub display_name: String,
    pub description: String,
    pub avatar: Option<String>,
    pub model: Option<String>,
    /// Preferred ACP runtime ID from the persona config (e.g., 'goose', 'claude').
    pub runtime: Option<String>,
    pub temperature: Option<f64>,
    pub max_context_tokens: Option<u64>,
    pub subscribe: Vec<String>,
    pub triggers: Option<TriggersData>,
    pub thread_replies: bool,
    pub broadcast_replies: bool,
    pub skills: Vec<String>,
    /// Raw MCP server configs.
    pub mcp_servers: Vec<serde_json::Value>,
    pub hooks: Option<HooksData>,
    /// The markdown body (system prompt).
    pub prompt: String,
}

/// Minimal manifest data needed by the pack loader.
#[derive(Debug)]
pub struct PackManifestData {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: Option<String>,
    /// Relative paths to .persona.md files.
    pub personas: Vec<String>,
    pub pack_instructions: Option<String>,
    pub mcp_config: Option<String>,
    // hooks_config is intentionally omitted: hooks are a runtime concern loaded
    // separately by sprout-acp, not a pack-parsing concern.
    /// Raw defaults block.
    pub defaults: Option<serde_json::Value>,
}

// ── Entry point ───────────────────────────────────────────────────────────────

/// Load a persona pack from a directory.
///
/// 1. Read and parse `.plugin/plugin.json`
/// 2. For each persona path in the manifest, read the `.persona.md` file
/// 3. Apply pack defaults to each persona (via merge logic)
/// 4. Read `pack_instructions` if present
/// 5. Read `.mcp.json` if present
/// 6. Validate all paths resolve within `pack_root` (no path traversal)
pub fn load_pack(pack_dir: &Path) -> Result<LoadedPack, PackError> {
    let pack_root = pack_dir.canonicalize().map_err(|e| PackError::Io {
        path: pack_dir.to_path_buf(),
        source: e,
    })?;

    // 1. Manifest
    let manifest_path = pack_root.join(".plugin").join("plugin.json");
    if !manifest_path.exists() {
        return Err(PackError::ManifestNotFound(manifest_path));
    }
    let manifest_raw = read_file(&manifest_path)?;
    let pm = manifest::parse_manifest(&manifest_raw)?;
    let manifest = PackManifestData {
        id: pm.id,
        name: pm.name,
        version: pm.version,
        description: pm.description,
        personas: pm.personas,
        pack_instructions: pm.pack_instructions,
        mcp_config: pm.mcp_config,
        defaults: pm
            .defaults
            .map(serde_json::to_value)
            .transpose()
            .map_err(|e| PackError::ManifestParse(format!("failed to serialize defaults: {e}")))?,
    };

    let pack_defaults = manifest.defaults.clone();

    // 2 & 3. Personas
    let persona_size_limit =
        (persona::MAX_FRONTMATTER_BYTES + persona::MAX_BODY_BYTES + 200) as u64;
    let mut personas = Vec::with_capacity(manifest.personas.len());
    for rel_path in &manifest.personas {
        let abs_path = safe_resolve(&pack_root, rel_path)?;
        if !abs_path.exists() {
            return Err(PackError::PersonaNotFound(abs_path));
        }
        let content = read_bounded_file(&abs_path, persona_size_limit)?;
        let persona = parse_persona_file(&abs_path, &content, pack_defaults.as_ref())?;
        personas.push(persona);
    }

    // 4. Pack instructions
    let text_size_limit =
        (crate::persona::MAX_FRONTMATTER_BYTES + crate::persona::MAX_BODY_BYTES + 200) as u64;

    let pack_instructions = match &manifest.pack_instructions {
        Some(rel) => {
            let abs = safe_resolve(&pack_root, rel)?;
            if !abs.exists() {
                return Err(PackError::FileParse {
                    path: abs,
                    reason: format!("pack_instructions file not found: {rel}"),
                });
            }
            Some(read_bounded_file(&abs, text_size_limit)?)
        }
        None => {
            let path = pack_root.join("instructions.md");
            if path.exists() {
                Some(read_bounded_file(&path, text_size_limit)?)
            } else {
                None
            }
        }
    };

    // 5. Shared MCP config
    let parse_mcp = |raw: String, path: &Path| -> Result<serde_json::Value, PackError> {
        serde_json::from_str(&raw).map_err(|e| PackError::McpConfigParse {
            path: path.to_path_buf(),
            reason: e.to_string(),
        })
    };
    let shared_mcp_config = match &manifest.mcp_config {
        Some(rel) => {
            let abs = safe_resolve(&pack_root, rel)?;
            if !abs.exists() {
                return Err(PackError::FileParse {
                    path: abs,
                    reason: format!("mcp_config file not found: {rel}"),
                });
            }
            let raw = read_bounded_file(&abs, text_size_limit)?;
            Some(parse_mcp(raw, &abs)?)
        }
        None => {
            let path = pack_root.join(".mcp.json");
            if path.exists() {
                let raw = read_bounded_file(&path, text_size_limit)?;
                Some(parse_mcp(raw, &path)?)
            } else {
                None
            }
        }
    };

    // 6. Skills directory
    let skills_dir = {
        let path = pack_root.join("skills");
        if path.is_dir() {
            Some(path)
        } else {
            None
        }
    };

    Ok(LoadedPack {
        manifest,
        personas,
        pack_instructions,
        shared_mcp_config,
        skills_dir,
    })
}

// ── Skill resolution ──────────────────────────────────────────────────────────

/// Determine which skills go to which persona.
///
/// - Skills listed in a persona's `skills:` array → only that persona
/// - Skills not listed in any persona's `skills:` → all personas (shared)
///
/// Returns a map of `persona_name → Vec<skill_name>`.
pub fn resolve_skills(pack_dir: &Path, personas: &[LoadedPersona]) -> HashMap<String, Vec<String>> {
    // Normalize a persona skill path (e.g. `"./skills/security-review/"`,
    // `"skills/search"`, `"web-search"`) to just the final path component
    // so it can be compared against bare `read_dir` entry names.
    fn normalize_skill_name(path: &str) -> String {
        std::path::Path::new(path.trim_end_matches('/'))
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(path)
            .to_owned()
    }

    // Collect all *normalized* skill names claimed by at least one persona.
    let mut claimed: std::collections::HashSet<String> = std::collections::HashSet::new();
    for p in personas {
        for s in &p.skills {
            claimed.insert(normalize_skill_name(s));
        }
    }

    // Enumerate skills directory — directories only, skip dotfiles.
    let skills_path = pack_dir.join("skills");
    let all_skills: Vec<String> = if skills_path.is_dir() {
        std::fs::read_dir(&skills_path)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|entry| {
                let name = entry.file_name().to_string_lossy().into_owned();
                // Skip dotfiles
                if name.starts_with('.') {
                    return None;
                }
                // Directories only (skip plain files and unresolvable entries)
                if entry.file_type().ok()?.is_dir() {
                    Some(name)
                } else {
                    None
                }
            })
            .collect()
    } else {
        vec![]
    };

    // Shared skills = those not claimed by any persona.
    let shared: Vec<String> = all_skills
        .iter()
        .filter(|s| !claimed.contains(*s))
        .cloned()
        .collect();

    let mut result: HashMap<String, Vec<String>> = HashMap::new();
    for p in personas {
        // Normalize claimed skill paths to bare directory names so the output
        // format is consistent with shared skills (which come from read_dir).
        let mut skills: Vec<String> = p.skills.iter().map(|s| normalize_skill_name(s)).collect();
        for s in &shared {
            if !skills.contains(s) {
                skills.push(s.clone());
            }
        }
        result.insert(p.name.clone(), skills);
    }

    result
}

// ── Path safety ───────────────────────────────────────────────────────────────

/// Verify a path resolves within the pack root.
///
/// Defense-in-depth:
/// 1. Reject any `..` path component before canonicalization.
/// 2. Canonicalize the joined path.
/// 3. Verify the result has `pack_root` as a prefix.
fn safe_resolve(pack_root: &Path, relative: &str) -> Result<PathBuf, PackError> {
    // Step 0: reject absolute paths (Unix `/` prefix or Windows drive letters).
    if relative.starts_with('/') {
        return Err(PackError::PathTraversal(relative.to_owned()));
    }
    #[cfg(windows)]
    if relative.len() >= 2 && relative.as_bytes()[1] == b':' {
        return Err(PackError::PathTraversal(relative.to_owned()));
    }

    // Step 1: reject `..` components eagerly.
    let rel = Path::new(relative);
    for component in rel.components() {
        if component == Component::ParentDir {
            return Err(PackError::PathTraversal(relative.to_owned()));
        }
    }

    let joined = pack_root.join(rel);

    // Step 2: canonicalize (resolves symlinks).
    // If the path doesn't exist yet we can't canonicalize — return the
    // un-canonicalized path so callers can produce a proper "not found" error.
    // We still verify the non-canonical form doesn't escape (belt-and-suspenders).
    if !joined.exists() {
        // Normalize without canonicalize: just check the lexical prefix.
        // The `..` check above already guards against traversal.
        return Ok(joined);
    }

    let canonical = joined.canonicalize().map_err(|e| PackError::Io {
        path: joined.clone(),
        source: e,
    })?;

    // Step 3: must be inside pack_root.
    if !canonical.starts_with(pack_root) {
        return Err(PackError::PathEscape(canonical));
    }

    Ok(canonical)
}

// ── Parsing helpers ───────────────────────────────────────────────────────────

fn read_file(path: &Path) -> Result<String, PackError> {
    std::fs::read_to_string(path).map_err(|e| PackError::Io {
        path: path.to_path_buf(),
        source: e,
    })
}

/// Read a file with a size limit. Returns an error if the file exceeds `max_bytes`.
fn read_bounded_file(path: &Path, max_bytes: u64) -> Result<String, PackError> {
    let meta = std::fs::metadata(path).map_err(|e| PackError::FileParse {
        path: path.to_path_buf(),
        reason: format!("cannot stat file: {e}"),
    })?;
    if meta.len() > max_bytes {
        return Err(PackError::FileParse {
            path: path.to_path_buf(),
            reason: format!("file too large: {} bytes (max {max_bytes})", meta.len()),
        });
    }
    read_file(path)
}

/// Parse a `.persona.md` file.
///
/// Delegates identity and prompt parsing to [`persona::parse_persona_md`],
/// then applies pack-level behavioral defaults via the merge layer.
fn parse_persona_file(
    path: &Path,
    content: &str,
    pack_defaults: Option<&serde_json::Value>,
) -> Result<LoadedPersona, PackError> {
    let pc: PersonaConfig =
        persona::parse_persona_md(content).map_err(|e| PackError::FileParse {
            path: path.to_path_buf(),
            reason: e.to_string(),
        })?;

    // Serialize the behavioral fields of PersonaConfig to JSON so the
    // existing merge logic can do precedence resolution unchanged.
    let fm_json = serde_json::to_value(&pc).map_err(|e| PackError::FileParse {
        path: path.to_path_buf(),
        reason: e.to_string(),
    })?;

    let resolved = resolve_persona_config(&fm_json, pack_defaults);

    // Convert typed MCP server configs back to raw JSON values.
    let mcp_servers: Vec<serde_json::Value> = pc
        .mcp_servers
        .iter()
        .filter_map(|s| serde_json::to_value(s).ok())
        .collect();

    // Convert typed Hooks to HooksData.
    let hooks = pc.hooks.map(|h| HooksData {
        on_start: h.on_start,
        on_stop: h.on_stop,
        on_message: h.on_message,
    });

    Ok(LoadedPersona {
        source_path: path.to_path_buf(),
        name: pc.name,
        display_name: pc.display_name,
        description: pc.description,
        avatar: pc.avatar,
        model: resolved.model,
        runtime: pc.runtime.clone(),
        temperature: resolved.temperature,
        max_context_tokens: resolved.max_context_tokens,
        subscribe: resolved.subscribe.unwrap_or_default(),
        triggers: resolved.triggers,
        thread_replies: resolved.thread_replies,
        broadcast_replies: resolved.broadcast_replies,
        skills: pc.skills,
        mcp_servers,
        hooks,
        prompt: pc.prompt,
    })
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    // ── Fixture helpers ───────────────────────────────────────────────────────

    fn make_pack(dir: &TempDir, personas: &[(&str, &str)]) -> PathBuf {
        let root = dir.path();

        // .plugin/plugin.json
        fs::create_dir_all(root.join(".plugin")).unwrap();
        let persona_paths: Vec<String> = personas
            .iter()
            .map(|(name, _)| format!("personas/{name}.persona.md"))
            .collect();
        let manifest = serde_json::json!({
            "id": "test-pack",
            "name": "Test Pack",
            "version": "0.1.0",
            "personas": persona_paths,
        });
        fs::write(
            root.join(".plugin/plugin.json"),
            serde_json::to_string_pretty(&manifest).unwrap(),
        )
        .unwrap();

        // personas/
        fs::create_dir_all(root.join("personas")).unwrap();
        for (name, content) in personas {
            fs::write(root.join(format!("personas/{name}.persona.md")), content).unwrap();
        }

        root.to_path_buf()
    }

    const SIMPLE_PERSONA: &str = r#"---
name: berry
display_name: Berry
description: A fast worker
---
You are Berry, a fast and direct worker.
"#;

    // ── load_pack: happy path ─────────────────────────────────────────────────

    #[test]
    fn load_valid_pack() {
        let dir = TempDir::new().unwrap();
        let root = make_pack(&dir, &[("berry", SIMPLE_PERSONA)]);
        let pack = load_pack(&root).unwrap();

        assert_eq!(pack.manifest.id, "test-pack");
        assert_eq!(pack.personas.len(), 1);
        assert_eq!(pack.personas[0].name, "berry");
        assert_eq!(pack.personas[0].display_name, "Berry");
        assert!(pack.personas[0].prompt.contains("fast and direct"));
        // Built-in defaults
        assert!(pack.personas[0].thread_replies);
        assert!(!pack.personas[0].broadcast_replies);
    }

    #[test]
    fn load_pack_with_instructions_and_mcp() {
        let dir = TempDir::new().unwrap();
        let root = make_pack(&dir, &[("berry", SIMPLE_PERSONA)]);

        fs::write(root.join("instructions.md"), "Pack-level instructions.").unwrap();
        fs::write(root.join(".mcp.json"), r#"{"mcpServers": {}}"#).unwrap();

        let pack = load_pack(&root).unwrap();
        assert_eq!(
            pack.pack_instructions.as_deref(),
            Some("Pack-level instructions.")
        );
        assert!(pack.shared_mcp_config.is_some());
    }

    #[test]
    fn load_pack_skills_dir_detected() {
        let dir = TempDir::new().unwrap();
        let root = make_pack(&dir, &[("berry", SIMPLE_PERSONA)]);
        fs::create_dir_all(root.join("skills")).unwrap();

        let pack = load_pack(&root).unwrap();
        assert!(pack.skills_dir.is_some());
    }

    // ── load_pack: error cases ────────────────────────────────────────────────

    #[test]
    fn missing_plugin_json_returns_error() {
        let dir = TempDir::new().unwrap();
        let err = load_pack(dir.path()).unwrap_err();
        assert!(matches!(err, PackError::ManifestNotFound(_)));
    }

    #[test]
    fn missing_persona_file_returns_error() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();

        fs::create_dir_all(root.join(".plugin")).unwrap();
        let manifest = serde_json::json!({
            "id": "x", "name": "X", "version": "0.1.0",
            "personas": ["personas/ghost.persona.md"],
        });
        fs::write(
            root.join(".plugin/plugin.json"),
            serde_json::to_string(&manifest).unwrap(),
        )
        .unwrap();
        fs::create_dir_all(root.join("personas")).unwrap();

        let err = load_pack(root).unwrap_err();
        assert!(matches!(err, PackError::PersonaNotFound(_)));
    }

    // ── Path safety ───────────────────────────────────────────────────────────

    #[test]
    fn dotdot_component_rejected() {
        let dir = TempDir::new().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let err = safe_resolve(&root, "../../etc/passwd").unwrap_err();
        assert!(matches!(err, PackError::PathTraversal(_)));
    }

    #[test]
    fn dotdot_in_middle_rejected() {
        let dir = TempDir::new().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let err = safe_resolve(&root, "personas/../../../etc/passwd").unwrap_err();
        assert!(matches!(err, PackError::PathTraversal(_)));
    }

    #[test]
    fn normal_path_resolves_ok() {
        let dir = TempDir::new().unwrap();
        let root = dir.path().canonicalize().unwrap();
        fs::create_dir_all(root.join("personas")).unwrap();
        let target = root.join("personas/berry.persona.md");
        fs::write(&target, "hello").unwrap();
        let resolved = safe_resolve(&root, "personas/berry.persona.md").unwrap();
        assert_eq!(resolved, target.canonicalize().unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn symlink_escape_rejected() {
        use std::os::unix::fs::symlink;

        let dir = TempDir::new().unwrap();
        let root = dir.path().canonicalize().unwrap();
        fs::create_dir_all(root.join("personas")).unwrap();

        // Create a symlink inside the pack that points outside.
        let outside = TempDir::new().unwrap();
        let outside_file = outside.path().join("secret.txt");
        fs::write(&outside_file, "secret").unwrap();

        symlink(&outside_file, root.join("personas/escape.persona.md")).unwrap();

        let err = safe_resolve(&root, "personas/escape.persona.md").unwrap_err();
        assert!(matches!(err, PackError::PathEscape(_)));
    }

    // ── Skill resolution ──────────────────────────────────────────────────────

    fn make_loaded_persona(name: &str, skills: Vec<&str>) -> LoadedPersona {
        LoadedPersona {
            source_path: PathBuf::from(format!("{name}.persona.md")),
            name: name.to_owned(),
            display_name: name.to_owned(),
            description: String::new(),
            avatar: None,
            model: None,
            runtime: None,
            temperature: None,
            max_context_tokens: None,
            subscribe: vec![],
            triggers: None,
            thread_replies: true,
            broadcast_replies: false,
            skills: skills.into_iter().map(str::to_owned).collect(),
            mcp_servers: vec![],
            hooks: None,
            prompt: String::new(),
        }
    }

    #[test]
    fn listed_skills_go_to_specific_persona() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        let skills_dir = root.join("skills");
        fs::create_dir_all(&skills_dir).unwrap();
        fs::create_dir_all(skills_dir.join("web-search")).unwrap();
        fs::create_dir_all(skills_dir.join("code-review")).unwrap();

        let personas = vec![
            make_loaded_persona("alpha", vec!["web-search"]),
            make_loaded_persona("beta", vec![]),
        ];

        let map = resolve_skills(root, &personas);

        // alpha claimed "web-search" → only alpha gets it
        assert!(map["alpha"].contains(&"web-search".to_owned()));
        // "code-review" is unclaimed → both get it
        assert!(map["alpha"].contains(&"code-review".to_owned()));
        assert!(map["beta"].contains(&"code-review".to_owned()));
        // beta did NOT claim "web-search" and it was claimed → beta doesn't get it
        assert!(!map["beta"].contains(&"web-search".to_owned()));
    }

    #[test]
    fn unclaimed_skills_go_to_all_personas() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        let skills_dir = root.join("skills");
        fs::create_dir_all(&skills_dir).unwrap();
        fs::create_dir_all(skills_dir.join("shared-skill")).unwrap();

        let personas = vec![
            make_loaded_persona("alpha", vec![]),
            make_loaded_persona("beta", vec![]),
        ];

        let map = resolve_skills(root, &personas);
        assert!(map["alpha"].contains(&"shared-skill".to_owned()));
        assert!(map["beta"].contains(&"shared-skill".to_owned()));
    }

    #[test]
    fn no_skills_dir_returns_empty() {
        let dir = TempDir::new().unwrap();
        let personas = vec![make_loaded_persona("alpha", vec![])];
        let map = resolve_skills(dir.path(), &personas);
        assert!(map["alpha"].is_empty());
    }

    #[test]
    fn claimed_skill_paths_normalized_to_bare_names() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        let skills_dir = root.join("skills");
        fs::create_dir_all(&skills_dir).unwrap();
        fs::create_dir_all(skills_dir.join("web-search")).unwrap();

        // Persona claims skill via a path with prefix and trailing slash.
        let personas = vec![make_loaded_persona("alpha", vec!["./skills/web-search/"])];

        let map = resolve_skills(root, &personas);

        // Output must be the bare name, not the raw path.
        assert!(map["alpha"].contains(&"web-search".to_owned()));
        assert!(!map["alpha"].iter().any(|s| s.contains('/')));
    }

    // ── Pack defaults ─────────────────────────────────────────────────────────

    #[test]
    fn pack_defaults_applied_to_persona() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();

        fs::create_dir_all(root.join(".plugin")).unwrap();
        let manifest = serde_json::json!({
            "id": "test-pack",
            "name": "Test Pack",
            "version": "0.1.0",
            "personas": ["personas/berry.persona.md"],
            "defaults": {
                "model": "claude-3-sonnet",
                "thread_replies": false,
            }
        });
        fs::write(
            root.join(".plugin/plugin.json"),
            serde_json::to_string_pretty(&manifest).unwrap(),
        )
        .unwrap();

        fs::create_dir_all(root.join("personas")).unwrap();
        // Persona does NOT set model or thread_replies → should inherit defaults
        fs::write(
            root.join("personas/berry.persona.md"),
            "---\nname: berry\ndisplay_name: Berry\ndescription: Fast\n---\nYou are Berry.\n",
        )
        .unwrap();

        let pack = load_pack(root).unwrap();
        let p = &pack.personas[0];
        assert_eq!(p.model.as_deref(), Some("claude-3-sonnet"));
        assert!(!p.thread_replies);
    }
}
