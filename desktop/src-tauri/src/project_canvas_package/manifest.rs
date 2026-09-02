use std::{collections::BTreeMap, path::Path};

use serde::Deserialize;

pub(super) const MAX_MANIFEST_BYTES: usize = 64 * 1024;
pub(super) const MAX_DATA_BYTES: usize = 256 * 1024;
pub(super) const MAX_TEXT_BYTES: usize = 2 * 1024 * 1024;
pub(super) const MAX_FILE_BYTES: usize = 8 * 1024 * 1024;
pub(super) const MAX_PACKAGE_BYTES: usize = 32 * 1024 * 1024;
pub(super) const MAX_PACKAGE_FILES: usize = 512;
const MAX_JSON_DEPTH: usize = 32;
const MAX_JSON_NODES: usize = 10_000;

const FORMAT: &str = "buzz-project-canvas";
const PROTOCOL_VERSION: u32 = 1;
// Must stay in sync with `capabilitySchema` in
// desktop/src/features/projects/ui/project-canvas/projectCanvasProtocol.ts.
const ALLOWED_CAPABILITIES: &[&str] = &[
    "project.metadata.read",
    "project.channels.read",
    "project.reviews.read",
    "project.tasks.read",
    "project.people.read",
    "project.tasks.write",
    "app.open",
    "app.dm.send",
];

#[derive(Clone, Debug)]
pub(super) struct ValidatedManifest {
    pub(super) scripts: Vec<String>,
    pub(super) styles: Vec<String>,
    pub(super) capabilities: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Manifest {
    format: String,
    protocol_version: u32,
    scripts: Vec<String>,
    styles: Vec<String>,
    data: String,
    capabilities: Vec<String>,
}

pub(super) fn validate_manifest(
    files: &BTreeMap<String, Vec<u8>>,
) -> Result<(ValidatedManifest, serde_json::Value), String> {
    let raw = files
        .get("manifest.json")
        .ok_or_else(|| "project canvas package is missing manifest.json".to_string())?;
    if raw.len() > MAX_MANIFEST_BYTES {
        return Err("project canvas manifest exceeds 64 KiB".to_string());
    }
    let text = std::str::from_utf8(raw)
        .map_err(|_| "project canvas manifest must be UTF-8".to_string())?;
    let manifest: Manifest = serde_json::from_str(text)
        .map_err(|error| format!("invalid project canvas manifest: {error}"))?;

    if manifest.format != FORMAT {
        return Err(format!(
            "unsupported project canvas format: {}",
            manifest.format
        ));
    }
    if manifest.protocol_version != PROTOCOL_VERSION {
        return Err(format!(
            "unsupported project canvas protocol version: {}",
            manifest.protocol_version
        ));
    }

    if manifest.scripts.is_empty() || manifest.scripts.len() > 64 {
        return Err("project canvas manifest must declare 1 to 64 scripts".to_string());
    }
    let mut scripts = Vec::with_capacity(manifest.scripts.len());
    for raw_script in manifest.scripts {
        let script = validate_relative_path(&raw_script)?;
        let is_canvas_entry = script == "canvas.js";
        let is_widget = script.starts_with("widgets/") && extension(&script) == Some("js");
        if !is_canvas_entry && !is_widget {
            return Err(
                "project canvas scripts must be canvas.js or .js files below widgets/".to_string(),
            );
        }
        if !files.contains_key(&script) {
            return Err(format!("project canvas script does not exist: {script}"));
        }
        if scripts.contains(&script) {
            return Err(format!("duplicate project canvas script: {script}"));
        }
        scripts.push(script);
    }
    if scripts.last().map(String::as_str) != Some("canvas.js") {
        return Err("project canvas scripts must load canvas.js last".to_string());
    }

    if manifest.styles.is_empty() || manifest.styles.len() > 8 {
        return Err("project canvas manifest must declare 1 to 8 styles".to_string());
    }
    let mut styles = Vec::with_capacity(manifest.styles.len());
    for raw_style in manifest.styles {
        let style = validate_relative_path(&raw_style)?;
        if !style.starts_with("styles/") || extension(&style) != Some("css") {
            return Err("project canvas styles must be .css files below styles/".to_string());
        }
        if !files.contains_key(&style) {
            return Err(format!("project canvas style does not exist: {style}"));
        }
        if styles.contains(&style) {
            return Err(format!("duplicate project canvas style: {style}"));
        }
        styles.push(style);
    }

    let data_path = validate_relative_path(&manifest.data)?;
    if !data_path.starts_with("data/") || extension(&data_path) != Some("json") {
        return Err("project canvas data must be a .json file below data/".to_string());
    }
    let data_bytes = files
        .get(&data_path)
        .ok_or_else(|| format!("project canvas data does not exist: {data_path}"))?;
    if data_bytes.len() > MAX_DATA_BYTES {
        return Err("project canvas data exceeds 256 KiB".to_string());
    }
    let data_text = std::str::from_utf8(data_bytes)
        .map_err(|_| "project canvas data must be UTF-8".to_string())?;
    let data = serde_json::from_str(data_text)
        .map_err(|error| format!("invalid project canvas data: {error}"))?;
    let mut nodes = 0;
    validate_json_shape(&data, 0, &mut nodes)?;

    if manifest.capabilities.len() > ALLOWED_CAPABILITIES.len() {
        return Err("project canvas requests unsupported capabilities".to_string());
    }
    let mut capabilities = Vec::with_capacity(manifest.capabilities.len());
    for capability in manifest.capabilities {
        if !ALLOWED_CAPABILITIES.contains(&capability.as_str()) {
            return Err(format!(
                "unsupported project canvas capability: {capability}"
            ));
        }
        if capabilities.contains(&capability) {
            return Err(format!("duplicate project canvas capability: {capability}"));
        }
        capabilities.push(capability);
    }

    for path in files.keys() {
        validate_declared_file(path, &scripts, &styles, &data_path)?;
    }

    Ok((
        ValidatedManifest {
            scripts,
            styles,
            capabilities,
        },
        data,
    ))
}

fn validate_json_shape(
    value: &serde_json::Value,
    depth: usize,
    nodes: &mut usize,
) -> Result<(), String> {
    *nodes += 1;
    if depth > MAX_JSON_DEPTH || *nodes > MAX_JSON_NODES {
        return Err("project canvas data exceeds the JSON structure limit".to_string());
    }
    match value {
        serde_json::Value::Array(values) => {
            for value in values {
                validate_json_shape(value, depth + 1, nodes)?;
            }
        }
        serde_json::Value::Object(values) => {
            for value in values.values() {
                validate_json_shape(value, depth + 1, nodes)?;
            }
        }
        _ => {}
    }
    Ok(())
}

pub(super) fn validate_relative_path(raw: &str) -> Result<String, String> {
    if raw.is_empty() || raw.len() > 240 || raw.contains('\\') || raw.contains('\0') {
        return Err("invalid project canvas package path".to_string());
    }

    let path = Path::new(raw);
    if path.is_absolute() {
        return Err("project canvas package paths must be relative".to_string());
    }
    for component in path.components() {
        let std::path::Component::Normal(segment) = component else {
            return Err(format!("invalid project canvas package path: {raw}"));
        };
        let segment = segment
            .to_str()
            .ok_or_else(|| "project canvas package paths must be UTF-8".to_string())?;
        if segment.starts_with('.') || segment.is_empty() {
            return Err(format!("hidden project canvas package path: {raw}"));
        }
    }
    Ok(raw.to_string())
}

pub(super) fn mime_type(path: &str) -> Option<&'static str> {
    match extension(path)? {
        "js" | "mjs" => Some("text/javascript; charset=utf-8"),
        "css" => Some("text/css; charset=utf-8"),
        "json" => Some("application/json; charset=utf-8"),
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "gif" => Some("image/gif"),
        "webp" => Some("image/webp"),
        "avif" => Some("image/avif"),
        "woff" => Some("font/woff"),
        "woff2" => Some("font/woff2"),
        "ttf" => Some("font/ttf"),
        "mp4" => Some("video/mp4"),
        "webm" => Some("video/webm"),
        "ogg" => Some("audio/ogg"),
        "mp3" => Some("audio/mpeg"),
        "wav" => Some("audio/wav"),
        _ => None,
    }
}

pub(super) fn is_text(path: &str) -> bool {
    matches!(extension(path), Some("js" | "mjs" | "css" | "json"))
}

fn validate_declared_file(
    path: &str,
    scripts: &[String],
    styles: &[String],
    data_path: &str,
) -> Result<(), String> {
    if path == "manifest.json"
        || scripts.iter().any(|script| script == path)
        || styles.iter().any(|style| style == path)
    {
        return Ok(());
    }
    if path == data_path || path.starts_with("data/") && extension(path) == Some("json") {
        return Ok(());
    }
    if path.starts_with("assets/") && mime_type(path).is_some() {
        return Ok(());
    }
    Err(format!(
        "project canvas package contains an undeclared file: {path}"
    ))
}

fn extension(path: &str) -> Option<&str> {
    Path::new(path).extension()?.to_str()
}
