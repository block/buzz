use base64::{engine::general_purpose::STANDARD, Engine as _};
use png::Decoder;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::{Cursor, Read};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct ParsedPersonaPreview {
    pub display_name: String,
    pub system_prompt: String,
    pub avatar_data_url: Option<String>,
    pub avatar_ref: Option<String>,
    pub runtime: Option<String>,
    pub model: Option<String>,
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub name_pool: Vec<String>,
    pub source_file: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ParsePersonaFilesResult {
    pub personas: Vec<ParsedPersonaPreview>,
    pub skipped: Vec<SkippedFile>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SkippedFile {
    pub source_file: String,
    pub reason: String,
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const MAX_ZIP_ENTRIES: usize = 50;
const MAX_ZIP_DECOMPRESSED: usize = 100 * 1024 * 1024;

// ---------------------------------------------------------------------------
// PNG persona parsing
// ---------------------------------------------------------------------------

pub fn parse_png_persona(png_bytes: &[u8]) -> Result<ParsedPersonaPreview, String> {
    let decoder = Decoder::new(Cursor::new(png_bytes));
    let reader = decoder
        .read_info()
        .map_err(|e| format!("Invalid PNG: {e}"))?;
    let info = reader.info();

    let mut buzz_text: Option<&str> = None;
    let mut chara_text: Option<&str> = None;

    for chunk in &info.uncompressed_latin1_text {
        match chunk.keyword.as_str() {
            "buzz_persona_pkg" if buzz_text.is_none() => buzz_text = Some(&chunk.text),
            "chara" | "ccv3" if chara_text.is_none() => chara_text = Some(&chunk.text),
            _ => {}
        }
    }

    let fields = if let Some(text) = buzz_text {
        parse_buzz_payload(text)?
    } else if let Some(text) = chara_text {
        parse_chara_payload(text)?
    } else {
        return Err("This image doesn't contain persona data.".to_string());
    };

    // For PNG persona cards, the avatar is the image itself — override
    // whatever avatarUrl the embedded JSON metadata might contain.
    let avatar_data_url = Some(format!(
        "data:image/png;base64,{}",
        STANDARD.encode(png_bytes)
    ));

    Ok(ParsedPersonaPreview {
        display_name: fields.display_name,
        system_prompt: fields.system_prompt,
        avatar_data_url,
        avatar_ref: fields.avatar_url,
        runtime: fields.runtime,
        model: fields.model,
        provider: fields.provider,
        name_pool: fields.name_pool,
        source_file: String::new(),
    })
}

fn decode_b64_json(b64: &str) -> Result<Value, String> {
    let bytes = STANDARD
        .decode(b64.trim())
        .map_err(|e| format!("Invalid base64: {e}"))?;
    serde_json::from_slice(&bytes).map_err(|e| format!("Invalid JSON: {e}"))
}

/// Extracted fields from a Buzz persona JSON payload.
struct BuzzPersonaFields {
    display_name: String,
    system_prompt: String,
    avatar_url: Option<String>,
    runtime: Option<String>,
    model: Option<String>,
    provider: Option<String>,
    name_pool: Vec<String>,
}

/// Extract and validate fields from a Buzz persona JSON value
/// (shared by both the PNG tEXt-chunk path and the standalone JSON path).
fn extract_buzz_fields(v: &Value) -> Result<BuzzPersonaFields, String> {
    let version = v.get("version").and_then(|v| v.as_u64()).unwrap_or(0);
    if version != 1 {
        return Err(format!("Unsupported persona version: {version}"));
    }
    let name = v
        .get("displayName")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    let prompt = v
        .get("systemPrompt")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if name.is_empty() {
        return Err("displayName is empty".to_string());
    }
    if prompt.is_empty() {
        return Err("systemPrompt is empty".to_string());
    }
    let avatar_url = v
        .get("avatarUrl")
        .and_then(|v| v.as_str())
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    // Read "runtime" with backward-compat fallback to legacy "provider" key.
    let runtime = v
        .get("runtime")
        .or_else(|| v.get("provider"))
        .and_then(|v| v.as_str())
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    let model = v
        .get("model")
        .and_then(|v| v.as_str())
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    // "llmProvider" is the LLM inference provider (e.g. "databricks", "anthropic").
    // Distinct from "runtime" (the ACP harness) and from the legacy "provider" key
    // (which mapped to runtime for backward compat).
    let provider = v
        .get("llmProvider")
        .and_then(|v| v.as_str())
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    let name_pool = v
        .get("namePool")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|item| item.as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default();
    Ok(BuzzPersonaFields {
        display_name: name,
        system_prompt: prompt,
        avatar_url,
        runtime,
        model,
        provider,
        name_pool,
    })
}

fn parse_buzz_payload(b64: &str) -> Result<BuzzPersonaFields, String> {
    let v = decode_b64_json(b64)?;
    extract_buzz_fields(&v)
}

fn parse_chara_payload(b64: &str) -> Result<BuzzPersonaFields, String> {
    let v = decode_b64_json(b64)?;
    let data = v.get("data").ok_or("Missing 'data' in chara payload")?;
    let name = data
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    let mut prompt = data
        .get("system_prompt")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if prompt.is_empty() {
        prompt = data
            .get("description")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
    }
    if name.is_empty() {
        return Err("Chara card has no name".to_string());
    }
    if prompt.is_empty() {
        return Err("Chara card has no system_prompt or description".to_string());
    }
    Ok(BuzzPersonaFields {
        display_name: name,
        system_prompt: prompt,
        avatar_url: None,
        runtime: None,
        model: None,
        provider: None,
        name_pool: Vec::new(),
    })
}

// ---------------------------------------------------------------------------
// JSON persona parsing / encoding
// ---------------------------------------------------------------------------

pub fn parse_json_persona(json_bytes: &[u8]) -> Result<ParsedPersonaPreview, String> {
    let v: Value = serde_json::from_slice(json_bytes).map_err(|e| format!("Invalid JSON: {e}"))?;
    let fields = extract_buzz_fields(&v)?;

    Ok(ParsedPersonaPreview {
        display_name: fields.display_name,
        system_prompt: fields.system_prompt,
        avatar_ref: fields.avatar_url.clone(),
        avatar_data_url: fields.avatar_url,
        runtime: fields.runtime,
        model: fields.model,
        provider: fields.provider,
        name_pool: fields.name_pool,
        source_file: String::new(),
    })
}

pub fn encode_persona_json(
    display_name: &str,
    system_prompt: &str,
    avatar_url: Option<&str>,
    runtime: Option<&str>,
    model: Option<&str>,
    provider: Option<&str>,
    name_pool: &[String],
) -> Result<Vec<u8>, String> {
    let mut map = serde_json::Map::new();
    map.insert("version".to_string(), serde_json::json!(1));
    map.insert("displayName".to_string(), serde_json::json!(display_name));
    map.insert("systemPrompt".to_string(), serde_json::json!(system_prompt));
    if let Some(url) = avatar_url {
        map.insert("avatarUrl".to_string(), serde_json::json!(url));
    }
    if let Some(r) = runtime {
        map.insert("runtime".to_string(), serde_json::json!(r));
    }
    if let Some(m) = model {
        map.insert("model".to_string(), serde_json::json!(m));
    }
    if let Some(p) = provider {
        map.insert("llmProvider".to_string(), serde_json::json!(p));
    }
    if !name_pool.is_empty() {
        map.insert("namePool".to_string(), serde_json::json!(name_pool));
    }

    serde_json::to_vec_pretty(&map).map_err(|e| format!("Failed to serialize JSON: {e}"))
}

// ---------------------------------------------------------------------------
// Persona markdown parsing
// ---------------------------------------------------------------------------

/// Parse a persona Markdown file into a `ParsedPersonaPreview`.
pub fn parse_md_persona(md_bytes: &[u8]) -> Result<ParsedPersonaPreview, String> {
    let content =
        std::str::from_utf8(md_bytes).map_err(|e| format!("Invalid UTF-8 in Markdown: {e}"))?;
    match buzz_persona_pkg::persona::parse_persona_md(content) {
        Ok(config) => Ok(parsed_preview_from_md_config(config)),
        Err(strict_err) => parse_lenient_md_persona(content)
            .map_err(|_| format!("Failed to parse persona Markdown: {strict_err}")),
    }
}

fn parsed_preview_from_md_config(
    config: buzz_persona_pkg::persona::PersonaConfig,
) -> ParsedPersonaPreview {
    // Split "provider:model" into separate fields for the preview.
    let model = match config.model.as_deref() {
        Some(s) if !s.is_empty() => {
            let (_prov, id) = buzz_persona_pkg::persona::split_model(s);
            Some(id.to_owned())
        }
        _ => None,
    };

    ParsedPersonaPreview {
        display_name: config.display_name,
        system_prompt: config.prompt,
        avatar_data_url: None, // Markdown avatars are paths, not data URIs
        avatar_ref: config.avatar,
        runtime: config.runtime,
        model,
        provider: None, // Markdown persona format does not carry llmProvider
        name_pool: Vec::new(),
        source_file: String::new(),
    }
}

#[derive(Debug, Deserialize)]
struct LenientMdFrontmatter {
    name: Option<String>,
    display_name: Option<String>,
    avatar: Option<String>,
    runtime: Option<String>,
    model: Option<String>,
}

fn parse_lenient_md_persona(content: &str) -> Result<ParsedPersonaPreview, String> {
    let (frontmatter, body) = buzz_persona_pkg::persona::split_frontmatter(content)
        .map_err(|e| format!("Missing frontmatter: {e}"))?;
    let fields: LenientMdFrontmatter =
        serde_yaml::from_str(frontmatter).map_err(|e| format!("Invalid YAML frontmatter: {e}"))?;
    let display_name = fields
        .display_name
        .or(fields.name)
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "Missing display name".to_string())?;
    // Split "provider:model" into separate fields for the preview.
    let model = match fields.model.as_deref() {
        Some(s) if !s.is_empty() => {
            let (_prov, id) = buzz_persona_pkg::persona::split_model(s);
            Some(id.to_owned())
        }
        _ => None,
    };

    Ok(ParsedPersonaPreview {
        display_name,
        system_prompt: body.to_string(),
        avatar_data_url: None, // Markdown avatars are paths, not data URIs
        avatar_ref: fields.avatar,
        runtime: fields.runtime,
        model,
        provider: None, // Markdown persona format does not carry llmProvider
        name_pool: Vec::new(),
        source_file: String::new(),
    })
}

/// Detect whether a ZIP archive is a persona pack (has `.plugin/plugin.json`).
/// If so, resolve it and return previews for all personas in the pack.
/// Find `.plugin/plugin.json` in a directory. Returns the parent of `.plugin/`.
/// Checks root and root/* only (matches pack detection scope in parse_zip_personas).
pub fn find_plugin_json(root: &std::path::Path) -> Option<std::path::PathBuf> {
    // Root level: .plugin/plugin.json
    if root.join(".plugin").join("plugin.json").exists() {
        return Some(root.to_path_buf());
    }
    // One folder deep: <folder>/.plugin/plugin.json (common zip layout)
    for entry in std::fs::read_dir(root).ok()?.flatten() {
        if entry.file_type().ok()?.is_dir() {
            let child = entry.path();
            if child.join(".plugin").join("plugin.json").exists() {
                return Some(child);
            }
        }
    }
    None
}

pub fn parse_zip_pack(zip_bytes: &[u8]) -> Result<ParsePersonaFilesResult, String> {
    // Extract to a temp directory, resolve the pack, convert to previews.
    let tmp = tempfile::tempdir().map_err(|e| format!("Failed to create temp dir: {e}"))?;
    let cursor = Cursor::new(zip_bytes);
    let mut archive =
        zip::ZipArchive::new(cursor).map_err(|e| format!("Invalid ZIP archive: {e}"))?;

    // Extract all files with safe path handling.
    let mut total_decompressed: usize = 0;
    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| format!("Failed to read ZIP entry: {e}"))?;

        // enclosed_name() returns None for paths with traversal components
        // (.., absolute paths, Windows drive prefixes). This is the canonical
        // safe extraction check from the zip crate.
        let safe_name = match entry.enclosed_name() {
            Some(name) => name.to_path_buf(),
            None => continue, // path traversal — skip
        };
        let name_str = safe_name.to_string_lossy();
        if name_str.starts_with("__MACOSX/") || name_str.contains("/._") {
            continue; // macOS metadata
        }

        let out_path = tmp.path().join(&safe_name);
        if entry.is_dir() {
            std::fs::create_dir_all(&out_path).map_err(|e| format!("Failed to create dir: {e}"))?;
        } else {
            if let Some(parent) = out_path.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("Failed to create parent dir: {e}"))?;
            }
            let mut data = Vec::new();
            loop {
                let mut chunk = [0u8; 8192];
                let n = entry
                    .read(&mut chunk)
                    .map_err(|e| format!("Read error: {e}"))?;
                if n == 0 {
                    break;
                }
                total_decompressed += n;
                if total_decompressed > MAX_ZIP_DECOMPRESSED {
                    return Err("ZIP decompressed content exceeds 100MB limit".to_string());
                }
                data.extend_from_slice(&chunk[..n]);
            }
            std::fs::write(&out_path, &data)
                .map_err(|e| format!("Failed to write {}: {e}", name_str))?;
        }
    }

    // Find the pack root by locating .plugin/plugin.json in the extracted tree.
    // Handles: root-level (.plugin/plugin.json), single folder (my-pack/.plugin/...),
    // or deeper nesting (foo/bar/.plugin/...).
    let pack_root = find_plugin_json(tmp.path()).ok_or_else(|| {
        "ZIP detected as pack but .plugin/plugin.json not found after extraction".to_string()
    })?;

    // Resolve the pack from the extracted directory.
    let resolved = buzz_persona_pkg::resolve::resolve_pack(&pack_root)
        .map_err(|e| format!("Pack validation failed: {e}"))?;

    let personas: Vec<ParsedPersonaPreview> = resolved
        .personas
        .iter()
        .map(|p| ParsedPersonaPreview {
            display_name: p.display_name.clone(),
            system_prompt: p.system_prompt.clone(),
            avatar_data_url: None,
            avatar_ref: p.avatar.clone(),
            runtime: p.runtime.clone(),
            model: p.model.clone(),
            provider: None, // persona packs do not carry llmProvider
            name_pool: Vec::new(),
            source_file: format!("{} ({})", p.name, resolved.name),
        })
        .collect();

    Ok(ParsePersonaFilesResult {
        personas,
        skipped: vec![],
    })
}

// ---------------------------------------------------------------------------
// ZIP parsing
// ---------------------------------------------------------------------------

pub fn parse_zip_personas(zip_bytes: &[u8]) -> Result<ParsePersonaFilesResult, String> {
    let cursor = Cursor::new(zip_bytes);
    let mut archive =
        zip::ZipArchive::new(cursor).map_err(|e| format!("Invalid ZIP archive: {e}"))?;

    // Detect persona pack BEFORE entry limit — packs may have many files.
    // Only match root-level or one-folder-deep (matches find_plugin_json scope).
    let is_pack = (0..archive.len()).any(|i| {
        archive
            .by_index(i)
            .ok()
            .map(|e| {
                let name = e.name().trim_start_matches('/');
                // Root: ".plugin/plugin.json"
                // One folder deep: "my-pack/.plugin/plugin.json"
                name == ".plugin/plugin.json"
                    || name
                        .strip_suffix("/.plugin/plugin.json")
                        .map(|prefix| !prefix.contains('/'))
                        .unwrap_or(false)
            })
            .unwrap_or(false)
    });
    if is_pack {
        return parse_zip_pack(zip_bytes);
    }

    // Entry limit only applies to loose-persona zips (not packs).
    if archive.len() > MAX_ZIP_ENTRIES {
        return Err(format!(
            "ZIP contains too many entries ({}, max {MAX_ZIP_ENTRIES})",
            archive.len()
        ));
    }

    let mut personas = Vec::new();
    let mut skipped = Vec::new();
    let mut total_decompressed: usize = 0;
    let mut has_valid_file = false;

    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| format!("Failed to read ZIP entry: {e}"))?;

        let raw_name = entry.name().to_string();

        // Sanitize path
        let name = raw_name.trim_start_matches('/');
        if name.contains("..") {
            skipped.push(SkippedFile {
                source_file: raw_name.clone(),
                reason: "Path traversal detected".to_string(),
            });
            continue;
        }

        // Skip macOS resource fork metadata (e.g. __MACOSX/._file.json)
        if name.starts_with("__MACOSX/") || name.contains("/._") || name.starts_with("._") {
            continue;
        }

        let lower = name.to_ascii_lowercase();
        let is_png = lower.ends_with(".png");
        let is_json = lower.ends_with(".json");
        let is_md = lower.ends_with(".md");

        if !is_png && !is_json && !is_md {
            skipped.push(SkippedFile {
                source_file: raw_name,
                reason: "Not a .png, .json, or .md file".to_string(),
            });
            continue;
        }

        has_valid_file = true;

        // Read with cumulative size limit
        let mut data = Vec::new();
        loop {
            let mut chunk = [0u8; 8192];
            let n = entry
                .read(&mut chunk)
                .map_err(|e| format!("Read error: {e}"))?;
            if n == 0 {
                break;
            }
            total_decompressed += n;
            if total_decompressed > MAX_ZIP_DECOMPRESSED {
                return Err("ZIP decompressed content exceeds 100MB limit".to_string());
            }
            data.extend_from_slice(&chunk[..n]);
        }

        let parse_result = if is_md {
            parse_md_persona(&data)
        } else if is_json {
            parse_json_persona(&data)
        } else {
            parse_png_persona(&data)
        };

        match parse_result {
            Ok(mut preview) => {
                preview.source_file = raw_name;
                personas.push(preview);
            }
            Err(reason) => {
                skipped.push(SkippedFile {
                    source_file: raw_name,
                    reason,
                });
            }
        }
    }

    if !has_valid_file {
        return Err("No persona files found (expected .png, .json, or .md).".to_string());
    }

    Ok(ParsePersonaFilesResult { personas, skipped })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "persona_card_tests.rs"]
mod tests;
