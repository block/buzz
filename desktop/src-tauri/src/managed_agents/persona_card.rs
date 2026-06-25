use base64::{engine::general_purpose::STANDARD, Engine as _};
use png::Decoder;
use serde::Serialize;
use serde_json::Value;
use std::io::{Cursor, Read};

#[derive(Debug, Clone, Serialize)]
pub struct ParsedPersonaPreview {
    pub display_name: String,
    pub system_prompt: String,
    pub avatar_data_url: Option<String>,
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

const MAX_ZIP_ENTRIES: usize = 50;
const MAX_ZIP_DECOMPRESSED: usize = 100 * 1024 * 1024;

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

pub fn parse_json_persona(json_bytes: &[u8]) -> Result<ParsedPersonaPreview, String> {
    let v: Value = serde_json::from_slice(json_bytes).map_err(|e| format!("Invalid JSON: {e}"))?;
    let fields = extract_buzz_fields(&v)?;

    Ok(ParsedPersonaPreview {
        display_name: fields.display_name,
        system_prompt: fields.system_prompt,
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

/// Parse a `.persona.md` file into a `ParsedPersonaPreview`.
pub fn parse_md_persona(md_bytes: &[u8]) -> Result<ParsedPersonaPreview, String> {
    let content =
        std::str::from_utf8(md_bytes).map_err(|e| format!("Invalid UTF-8 in .persona.md: {e}"))?;
    let config = buzz_persona_pkg::persona::parse_persona_md(content)
        .map_err(|e| format!("Failed to parse .persona.md: {e}"))?;

    // Split "provider:model" into separate fields for the preview.
    let model = match config.model.as_deref() {
        Some(s) if !s.is_empty() => {
            let (_prov, id) = buzz_persona_pkg::persona::split_model(s);
            Some(id.to_owned())
        }
        _ => None,
    };

    Ok(ParsedPersonaPreview {
        display_name: config.display_name,
        system_prompt: config.prompt,
        avatar_data_url: None, // .persona.md avatars are paths, not data URIs
        runtime: config.runtime,
        model,
        provider: None, // .persona.md format does not carry llmProvider
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
        let is_md = lower.ends_with(".persona.md");

        if !is_png && !is_json && !is_md {
            skipped.push(SkippedFile {
                source_file: raw_name,
                reason: "Not a .png, .json, or .persona.md file".to_string(),
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
        return Err("No persona files found (expected .png, .json, or .persona.md).".to_string());
    }

    Ok(ParsePersonaFilesResult { personas, skipped })
}

#[cfg(test)]
mod tests {
    use super::*;
    use png::{BitDepth, ColorType, Encoder};
    use std::io::Write;
    use zip::write::{SimpleFileOptions, ZipWriter};

    /// Helper: build a minimal valid PNG with a custom tEXt chunk.
    fn make_png_with_text(keyword: &str, text: &str) -> Vec<u8> {
        let mut buf = Vec::new();
        {
            let mut enc = Encoder::new(Cursor::new(&mut buf), 1, 1);
            enc.set_color(ColorType::Rgba);
            enc.set_depth(BitDepth::Eight);
            enc.add_text_chunk(keyword.to_string(), text.to_string())
                .unwrap();
            let mut w = enc.write_header().unwrap();
            w.write_image_data(&[0, 0, 0, 255]).unwrap();
        }
        buf
    }

    /// Helper: build a PNG with a buzz_persona_pkg tEXt chunk for the given name/prompt.
    fn make_test_persona_png(name: &str, prompt: &str) -> Vec<u8> {
        let payload = serde_json::json!({
            "version": 1,
            "displayName": name,
            "systemPrompt": prompt,
        });
        let b64 = STANDARD.encode(payload.to_string().as_bytes());
        make_png_with_text("buzz_persona_pkg", &b64)
    }

    /// Helper: build a plain PNG with no metadata.
    fn make_plain_png() -> Vec<u8> {
        let mut buf = Vec::new();
        {
            let mut enc = Encoder::new(Cursor::new(&mut buf), 1, 1);
            enc.set_color(ColorType::Rgba);
            enc.set_depth(BitDepth::Eight);
            let mut w = enc.write_header().unwrap();
            w.write_image_data(&[0, 0, 0, 255]).unwrap();
        }
        buf
    }

    /// Helper: create a ZIP from name→data pairs.
    fn make_test_zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut buf = Cursor::new(Vec::new());
        let mut zip = ZipWriter::new(&mut buf);
        let options = SimpleFileOptions::default();
        for (name, data) in entries {
            zip.start_file(*name, options).unwrap();
            zip.write_all(data).unwrap();
        }
        zip.finish().unwrap();
        buf.into_inner()
    }

    #[test]
    fn parse_png_round_trip() {
        let png = make_test_persona_png("George Costanza", "You are George.");
        let result = parse_png_persona(&png).unwrap();
        assert_eq!(result.display_name, "George Costanza");
        assert_eq!(result.system_prompt, "You are George.");
        assert!(result
            .avatar_data_url
            .unwrap()
            .starts_with("data:image/png;base64,"));
    }

    #[test]
    fn parse_png_no_metadata() {
        let png = make_plain_png();
        let err = parse_png_persona(&png).unwrap_err();
        assert!(err.contains("doesn't contain persona data"));
    }

    #[test]
    fn parse_png_unknown_version() {
        let payload = serde_json::json!({"version": 99, "displayName": "X", "systemPrompt": "Y"});
        let b64 = STANDARD.encode(payload.to_string().as_bytes());
        let png = make_png_with_text("buzz_persona_pkg", &b64);
        let err = parse_png_persona(&png).unwrap_err();
        assert!(err.contains("Unsupported persona version"));
    }

    #[test]
    fn parse_png_malformed_base64() {
        let png = make_png_with_text("buzz_persona_pkg", "!!!not-base64!!!");
        let err = parse_png_persona(&png).unwrap_err();
        assert!(err.contains("Invalid base64"));
    }

    #[test]
    fn parse_png_malformed_json() {
        let b64 = STANDARD.encode(b"not json at all");
        let png = make_png_with_text("buzz_persona_pkg", &b64);
        let err = parse_png_persona(&png).unwrap_err();
        assert!(err.contains("Invalid JSON"));
    }

    #[test]
    fn parse_png_empty_fields() {
        let payload = serde_json::json!({"version": 1, "displayName": "", "systemPrompt": "Y"});
        let b64 = STANDARD.encode(payload.to_string().as_bytes());
        let png = make_png_with_text("buzz_persona_pkg", &b64);
        let err = parse_png_persona(&png).unwrap_err();
        assert!(err.contains("displayName is empty"));
    }

    #[test]
    fn parse_png_chara_fallback() {
        let chara = serde_json::json!({
            "spec": "chara_card_v2",
            "spec_version": "2.0",
            "data": {
                "name": "Kramer",
                "system_prompt": "You are Kramer.",
                "description": ""
            }
        });
        let b64 = STANDARD.encode(chara.to_string().as_bytes());
        let png = make_png_with_text("chara", &b64);
        let result = parse_png_persona(&png).unwrap();
        assert_eq!(result.display_name, "Kramer");
        assert_eq!(result.system_prompt, "You are Kramer.");
    }

    #[test]
    fn parse_png_chara_ignored_when_buzz_present() {
        // Build a PNG with both buzz_persona_pkg and chara chunks.
        let buzz = serde_json::json!({"version": 1, "displayName": "Buzz Name", "systemPrompt": "Buzz prompt"});
        let chara = serde_json::json!({
            "spec": "chara_card_v2", "spec_version": "2.0",
            "data": {"name": "Chara Name", "system_prompt": "Chara prompt", "description": ""}
        });
        let buzz_b64 = STANDARD.encode(buzz.to_string().as_bytes());
        let chara_b64 = STANDARD.encode(chara.to_string().as_bytes());

        let mut buf = Vec::new();
        {
            let mut enc = Encoder::new(Cursor::new(&mut buf), 1, 1);
            enc.set_color(ColorType::Rgba);
            enc.set_depth(BitDepth::Eight);
            enc.add_text_chunk("buzz_persona_pkg".to_string(), buzz_b64)
                .unwrap();
            enc.add_text_chunk("chara".to_string(), chara_b64).unwrap();
            let mut w = enc.write_header().unwrap();
            w.write_image_data(&[0, 0, 0, 255]).unwrap();
        }

        let result = parse_png_persona(&buf).unwrap();
        assert_eq!(result.display_name, "Buzz Name");
        assert_eq!(result.system_prompt, "Buzz prompt");
    }

    #[test]
    fn parse_zip_valid_pack() {
        let p1 = make_test_persona_png("Alice", "Prompt A");
        let p2 = make_test_persona_png("Bob", "Prompt B");
        let p3 = make_test_persona_png("Carol", "Prompt C");
        let zip = make_test_zip(&[("alice.png", &p1), ("bob.png", &p2), ("carol.png", &p3)]);
        let result = parse_zip_personas(&zip).unwrap();
        assert_eq!(result.personas.len(), 3);
        assert!(result.skipped.is_empty());
        assert_eq!(result.personas[0].source_file, "alice.png");
    }

    #[test]
    fn parse_zip_mixed() {
        let valid1 = make_test_persona_png("Alice", "Prompt A");
        let valid2 = make_test_persona_png("Bob", "Prompt B");
        let bad_png = make_plain_png(); // no metadata
        let zip = make_test_zip(&[
            ("alice.png", &valid1),
            ("bob.png", &valid2),
            ("bad.png", &bad_png),
            ("readme.txt", b"hello"),
        ]);
        let result = parse_zip_personas(&zip).unwrap();
        assert_eq!(result.personas.len(), 2);
        assert_eq!(result.skipped.len(), 2);
    }

    #[test]
    fn parse_zip_no_pngs() {
        let zip = make_test_zip(&[("readme.txt", b"hello"), ("data.csv", b"a,b")]);
        let err = parse_zip_personas(&zip).unwrap_err();
        assert!(err.contains("No persona files found"));
    }

    #[test]
    fn parse_zip_exceeds_entry_limit() {
        let png = make_test_persona_png("X", "Y");
        let entries: Vec<(String, &[u8])> = (0..51)
            .map(|i| (format!("{i}.png"), png.as_slice()))
            .collect();
        let refs: Vec<(&str, &[u8])> = entries.iter().map(|(n, d)| (n.as_str(), *d)).collect();
        let zip = make_test_zip(&refs);
        let err = parse_zip_personas(&zip).unwrap_err();
        assert!(err.contains("too many entries"));
    }

    #[test]
    fn parse_zip_path_traversal() {
        let valid = make_test_persona_png("Safe", "Prompt");
        let evil = make_test_persona_png("Evil", "Prompt");
        let zip = make_test_zip(&[("safe.png", &valid), ("../evil.png", &evil)]);
        let result = parse_zip_personas(&zip).unwrap();
        assert_eq!(result.personas.len(), 1);
        assert_eq!(result.skipped.len(), 1);
        assert!(result.skipped[0].reason.contains("Path traversal"));
    }

    #[test]
    fn parse_png_duplicate_chunks() {
        // Two buzz_persona_pkg chunks — should use the first and ignore the second.
        let payload1 =
            serde_json::json!({"version": 1, "displayName": "First", "systemPrompt": "Prompt 1"});
        let payload2 =
            serde_json::json!({"version": 1, "displayName": "Second", "systemPrompt": "Prompt 2"});
        let b64_1 = STANDARD.encode(payload1.to_string().as_bytes());
        let b64_2 = STANDARD.encode(payload2.to_string().as_bytes());

        let mut buf = Vec::new();
        {
            let mut enc = Encoder::new(Cursor::new(&mut buf), 1, 1);
            enc.set_color(ColorType::Rgba);
            enc.set_depth(BitDepth::Eight);
            enc.add_text_chunk("buzz_persona_pkg".to_string(), b64_1)
                .unwrap();
            enc.add_text_chunk("buzz_persona_pkg".to_string(), b64_2)
                .unwrap();
            let mut w = enc.write_header().unwrap();
            w.write_image_data(&[0, 0, 0, 255]).unwrap();
        }

        let result = parse_png_persona(&buf).unwrap();
        assert_eq!(result.display_name, "First");
        assert_eq!(result.system_prompt, "Prompt 1");
    }

    #[test]
    fn parse_zip_exceeds_size_limit() {
        // Create a ZIP with entries whose cumulative decompressed size exceeds 100MB.
        let mut zip_buf = Cursor::new(Vec::new());
        {
            let mut zip = ZipWriter::new(&mut zip_buf);
            let options = SimpleFileOptions::default();
            zip.start_file("big.png", options).unwrap();
            let chunk = vec![0u8; 1024 * 1024]; // 1 MB
            for _ in 0..101 {
                zip.write_all(&chunk).unwrap();
            }
            zip.finish().unwrap();
        }
        let zip_bytes = zip_buf.into_inner();
        let err = parse_zip_personas(&zip_bytes).unwrap_err();
        assert!(err.contains("exceeds 100MB"));
    }

    #[test]
    fn parse_json_round_trip() {
        let bytes = encode_persona_json(
            "Ada Lovelace",
            "You are Ada.",
            Some("https://example.com/ada.png"),
            None,
            None,
            None,
            &[],
        )
        .unwrap();
        let result = parse_json_persona(&bytes).unwrap();
        assert_eq!(result.display_name, "Ada Lovelace");
        assert_eq!(result.system_prompt, "You are Ada.");
        assert_eq!(
            result.avatar_data_url.as_deref(),
            Some("https://example.com/ada.png")
        );
        assert!(result.source_file.is_empty());
    }

    #[test]
    fn parse_json_round_trip_no_avatar() {
        let bytes =
            encode_persona_json("Bob", "You are Bob.", None, None, None, None, &[]).unwrap();
        let result = parse_json_persona(&bytes).unwrap();
        assert_eq!(result.display_name, "Bob");
        assert_eq!(result.system_prompt, "You are Bob.");
        assert!(result.avatar_data_url.is_none());
    }

    #[test]
    fn parse_json_round_trip_data_uri_avatar() {
        let data_uri = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUg==";
        let bytes = encode_persona_json(
            "Carol",
            "You are Carol.",
            Some(data_uri),
            None,
            None,
            None,
            &[],
        )
        .unwrap();
        let result = parse_json_persona(&bytes).unwrap();
        assert_eq!(result.display_name, "Carol");
        assert_eq!(result.avatar_data_url.as_deref(), Some(data_uri));
    }

    #[test]
    fn parse_json_round_trip_with_runtime_and_model() {
        let bytes = encode_persona_json(
            "Agent Smith",
            "You are an agent.",
            None,
            Some("goose"),
            Some("claude-sonnet-4"),
            None,
            &[],
        )
        .unwrap();
        let result = parse_json_persona(&bytes).unwrap();
        assert_eq!(result.display_name, "Agent Smith");
        assert_eq!(result.system_prompt, "You are an agent.");
        assert!(result.avatar_data_url.is_none());
        assert_eq!(result.runtime.as_deref(), Some("goose"));
        assert_eq!(result.model.as_deref(), Some("claude-sonnet-4"));
    }

    #[test]
    fn parse_json_round_trip_without_runtime_and_model() {
        let bytes =
            encode_persona_json("Bob", "You are Bob.", None, None, None, None, &[]).unwrap();
        let result = parse_json_persona(&bytes).unwrap();
        assert_eq!(result.display_name, "Bob");
        assert!(result.runtime.is_none());
        assert!(result.model.is_none());
    }

    #[test]
    fn parse_json_backward_compat_no_runtime_model_fields() {
        // Simulate a legacy persona JSON without runtime/model fields
        let json = serde_json::json!({
            "version": 1,
            "displayName": "Legacy Persona",
            "systemPrompt": "Old school prompt"
        });
        let bytes = serde_json::to_vec(&json).unwrap();
        let result = parse_json_persona(&bytes).unwrap();
        assert_eq!(result.display_name, "Legacy Persona");
        assert_eq!(result.system_prompt, "Old school prompt");
        assert!(result.runtime.is_none());
        assert!(result.model.is_none());
    }

    #[test]
    fn parse_json_backward_compat_legacy_provider_key() {
        // A JSON card written with the old "provider" key should still parse.
        let json = serde_json::json!({
            "version": 1,
            "displayName": "Legacy Agent",
            "systemPrompt": "Old prompt",
            "provider": "goose"
        });
        let bytes = serde_json::to_vec(&json).unwrap();
        let result = parse_json_persona(&bytes).unwrap();
        assert_eq!(result.runtime.as_deref(), Some("goose"));
    }

    #[test]
    fn parse_json_invalid_version() {
        let json = serde_json::json!({
            "version": 99,
            "displayName": "X",
            "systemPrompt": "Y"
        });
        let bytes = serde_json::to_vec(&json).unwrap();
        let err = parse_json_persona(&bytes).unwrap_err();
        assert!(err.contains("Unsupported persona version"));
    }

    #[test]
    fn parse_json_empty_fields() {
        let json_empty_name = serde_json::json!({
            "version": 1,
            "displayName": "",
            "systemPrompt": "Y"
        });
        let err = parse_json_persona(&serde_json::to_vec(&json_empty_name).unwrap()).unwrap_err();
        assert!(err.contains("displayName is empty"));

        let json_empty_prompt = serde_json::json!({
            "version": 1,
            "displayName": "X",
            "systemPrompt": ""
        });
        let err = parse_json_persona(&serde_json::to_vec(&json_empty_prompt).unwrap()).unwrap_err();
        assert!(err.contains("systemPrompt is empty"));
    }

    #[test]
    fn parse_json_malformed() {
        let err = parse_json_persona(b"not json at all").unwrap_err();
        assert!(err.contains("Invalid JSON"));
    }

    #[test]
    fn parse_zip_with_json() {
        let j1 = encode_persona_json("Alice", "Prompt A", None, None, None, None, &[]).unwrap();
        let j2 = encode_persona_json("Bob", "Prompt B", None, None, None, None, &[]).unwrap();
        let zip = make_test_zip(&[("alice.persona.json", &j1), ("bob.persona.json", &j2)]);
        let result = parse_zip_personas(&zip).unwrap();
        assert_eq!(result.personas.len(), 2);
        assert!(result.skipped.is_empty());
        assert_eq!(result.personas[0].display_name, "Alice");
        assert_eq!(result.personas[1].display_name, "Bob");
    }

    #[test]
    fn parse_zip_mixed_png_and_json() {
        let png = make_test_persona_png("PngPersona", "PNG prompt");
        let json =
            encode_persona_json("JsonPersona", "JSON prompt", None, None, None, None, &[]).unwrap();
        let zip = make_test_zip(&[
            ("persona.png", &png),
            ("persona.json", &json),
            ("readme.txt", b"hello"),
        ]);
        let result = parse_zip_personas(&zip).unwrap();
        assert_eq!(result.personas.len(), 2);
        // readme.txt should be skipped
        assert_eq!(result.skipped.len(), 1);
        assert!(result.skipped[0]
            .reason
            .contains("Not a .png, .json, or .persona.md file"));
    }

    #[test]
    fn parse_zip_ignores_macos_resource_forks() {
        let j1 =
            encode_persona_json("Frank", "You are Frank.", None, None, None, None, &[]).unwrap();
        let j2 =
            encode_persona_json("Jackie", "You are Jackie.", None, None, None, None, &[]).unwrap();
        let zip = make_test_zip(&[
            ("frank-costanza.persona.json", &j1),
            ("jackie-chiles.persona.json", &j2),
            ("__MACOSX/._frank-costanza.persona.json", b"\x00\x05\x16"),
            ("__MACOSX/._jackie-chiles.persona.json", b"\x00\x05\x16"),
        ]);
        let result = parse_zip_personas(&zip).unwrap();
        assert_eq!(result.personas.len(), 2);
        // macOS resource forks should be silently ignored, not skipped with errors
        assert!(result.skipped.is_empty());
    }
}
