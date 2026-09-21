//! Profile-variant expansion for a custom harness definition.
//!
//! Kept in its own module so `custom_harnesses.rs` stays within the
//! repository's file-size ratchet.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::{validate_harness_definition, HarnessDefinition};

/// Profile-variant expansion for a custom harness definition.
///
/// One definition file then covers a whole family of harnesses that differ only
/// by directory: a Hermes profile root, a per-project agent config dir, and so
/// on. Each immediate subdirectory of `dir` that carries `marker` becomes one
/// catalog entry, with `{name}` and `{dir}` available in the templates below.
///
/// A definition with this block is a TEMPLATE. The template itself stays in the
/// catalog as the harness's own default entry (it carries none of the variant
/// `env`, so it resolves exactly like the bare command), and it is also the
/// entry the user edits — the generated variants are read-only.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HarnessVariants {
    /// Directory whose immediate subdirectories are the profiles. A leading
    /// `~/` resolves against the user's home directory.
    pub dir: String,
    /// File that must exist inside a subdirectory for it to count as a profile.
    /// Empty means every immediate subdirectory counts.
    #[serde(default)]
    pub marker: String,
    /// Optional cap on expanded entries. Values above
    /// [`MAX_HARNESS_VARIANTS`] are clamped to it.
    #[serde(default)]
    pub max: Option<usize>,
    /// Template for each variant's id. `{id}` is this definition's id, `{slug}`
    /// the id-safe form of the profile directory name. Defaults to
    /// `{id}-{slug}`.
    #[serde(default)]
    pub id_template: Option<String>,
    /// Template for each variant's label. `{label}` is this definition's label,
    /// `{name}` the profile directory name as it appears on disk, `{meta}` the
    /// value read by [`HarnessVariants::label_from`]. Defaults to
    /// `{label} ({name})`, or to `{label} [{meta}] ({name})` when `label_from`
    /// is declared and resolves.
    #[serde(default)]
    pub label_template: Option<String>,
    /// Optional labelled metadata read from each profile directory, used to
    /// fill the `{meta}` placeholder (a Hermes persona title, say). When the
    /// declared `label_template` uses `{meta}` and the value cannot be read, the
    /// label falls back to the `{label} ({name})` default rather than rendering
    /// an empty bracket.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label_from: Option<HarnessVariantLabel>,
    /// Extra env vars for each variant. Placeholders: `{name}` (profile
    /// directory name), `{slug}` (its id-safe form), `{dir}` (the profile
    /// directory), `{root}` (the scanned directory).
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    /// Extra args appended to each variant, same placeholders as `env`.
    #[serde(default)]
    pub args: Vec<String>,
}

/// One value to read out of a file inside each profile directory.
///
/// The reading is presentation-only and fully optional: the referenced file
/// lives inside a directory the definition already names, and the definition
/// can already name the command Buzz spawns, so this adds no reach a harness
/// file did not have. Failures degrade to the directory-name label.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HarnessVariantLabel {
    /// File inside the profile directory, e.g. `profile.yaml`.
    pub file: String,
    /// Dotted path to the value, e.g. `ui_meta.hermes-bots.title`.
    pub key: String,
}

/// Maximum size of a `label_from` file read during expansion.
///
/// The value is a display label; a profile directory must not be able to make
/// discovery read an arbitrarily large file.
pub(crate) const MAX_VARIANT_LABEL_FILE_BYTES: u64 = 64 * 1024;

/// Maximum number of catalog entries a single `variants` block may expand to.
///
/// A template pointed at a large or unexpected directory must not flood the
/// runtime dropdown or the readiness registry.
pub(crate) const MAX_HARNESS_VARIANTS: usize = 64;

/// Expand a definition into the catalog entries it stands for.
///
/// Definitions without a `variants` block return themselves as a single entry,
/// so callers have one code path for both shapes. A definition WITH a block
/// returns itself first (the file-backed template entry, which is also the
/// harness's own default entry because it carries none of the variant `env`),
/// followed by one generated entry per profile directory (see
/// [`HarnessVariants`]).
///
/// A profile is an immediate subdirectory of `variants.dir` that contains
/// `variants.marker` (when a marker is declared). An unreadable directory, a
/// directory whose slugified name is empty, and a variant that fails validation
/// are logged and skipped so one bad profile never blocks the rest.
pub(crate) fn expand_variant_definitions(def: &HarnessDefinition) -> Vec<HarnessDefinition> {
    // `generated` is provenance, never authored — the template is file-backed.
    let base = HarnessDefinition {
        generated: false,
        generated_from: None,
        ..def.clone()
    };
    let Some(variants) = def.variants.as_ref() else {
        return vec![base];
    };

    let root = expand_tilde(&variants.dir);
    let entries = match std::fs::read_dir(&root) {
        Ok(entries) => entries,
        Err(err) => {
            tracing::warn!(
                "custom_harnesses: cannot read variants dir {} for {:?}: {err}",
                root.display(),
                def.id
            );
            return vec![base];
        }
    };

    let marker = variants.marker.trim().to_string();
    let mut profiles: Vec<(String, PathBuf)> = entries
        .flatten()
        .filter(|entry| entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false))
        .filter_map(|entry| {
            let path = entry.path();
            let name = path.file_name()?.to_str()?.to_string();
            if !marker.is_empty() && !path.join(&marker).is_file() {
                return None;
            }
            Some((name, path))
        })
        .collect();
    profiles.sort_by(|a, b| a.0.cmp(&b.0));
    let cap = variants.max.unwrap_or(MAX_HARNESS_VARIANTS);
    profiles.truncate(cap.min(MAX_HARNESS_VARIANTS));

    let id_template = variants.id_template.as_deref().unwrap_or("{id}-{slug}");
    let default_label_template = "{label} ({name})";
    let meta_label_template = "{label} [{meta}] ({name})";
    let declared_label_template = variants.label_template.as_deref();

    let mut expanded = Vec::with_capacity(profiles.len() + 1);
    expanded.push(base);
    for (name, dir) in profiles {
        let slug = slugify_harness_id_fragment(&name);
        if slug.is_empty() {
            tracing::warn!(
                "custom_harnesses: skipping profile {:?} under {} for {:?}: name has no usable id characters",
                name,
                root.display(),
                def.id
            );
            continue;
        }

        let meta = variants
            .label_from
            .as_ref()
            .and_then(|source| read_variant_label_meta(&dir, source));
        // A template that asks for `{meta}` on a profile with no readable value
        // would render an empty bracket, so fall back to the plain default.
        let label_template = match (declared_label_template, meta.as_deref()) {
            (Some(template), None) if template.contains("{meta}") => {
                tracing::debug!(
                    "custom_harnesses: profile {:?} has no readable labelFrom value; using {:?}",
                    name,
                    default_label_template
                );
                default_label_template
            }
            (Some(template), _) => template,
            (None, Some(_)) => meta_label_template,
            (None, None) => default_label_template,
        };

        let root_str = root.to_string_lossy().to_string();
        let dir_str = dir.to_string_lossy().to_string();
        let meta_str = meta.as_deref().unwrap_or("");
        let replacements = [
            ("{name}", name.as_str()),
            ("{dir}", dir_str.as_str()),
            ("{root}", root_str.as_str()),
            ("{id}", def.id.as_str()),
            ("{label}", def.label.as_str()),
            ("{slug}", slug.as_str()),
            ("{meta}", meta_str),
        ];
        let render = |template: &str| {
            let mut out = template.to_string();
            for (needle, value) in replacements {
                out = out.replace(needle, value);
            }
            out
        };

        let mut variant = def.clone();
        variant.id = render(id_template);
        variant.label = render(label_template);
        variant.args = def
            .args
            .iter()
            .chain(variants.args.iter())
            .map(|arg| render(arg))
            .collect();
        for (key, value) in &variants.env {
            variant.env.insert(key.clone(), render(value));
        }
        // The template's own block must not survive onto its variants: a
        // materialized entry is a real harness, not another template.
        variant.variants = None;
        variant.generated = true;
        variant.generated_from = Some(def.id.clone());

        if let Err(reason) = validate_harness_definition(&variant) {
            tracing::warn!(
                "custom_harnesses: skipping variant {:?} (profile {:?}): {reason}",
                variant.id,
                name
            );
            continue;
        }

        expanded.push(variant);
    }

    expanded
}

/// Read one dotted-path value out of a file inside a profile directory.
///
/// `key` is a dotted path into the document (`ui_meta.hermes-bots.title`).
/// Missing files, oversized files, unparseable YAML, absent keys, and non-scalar
/// values all return `None`: a label is presentation, so a profile whose
/// metadata cannot be read still appears in the catalog under its directory
/// name.
fn read_variant_label_meta(dir: &Path, source: &HarnessVariantLabel) -> Option<String> {
    let path = dir.join(source.file.trim());
    let metadata = match std::fs::metadata(&path) {
        Ok(metadata) => metadata,
        Err(err) => {
            tracing::debug!(
                "custom_harnesses: no label metadata at {}: {err}",
                path.display()
            );
            return None;
        }
    };
    if !metadata.is_file() {
        return None;
    }
    if metadata.len() > MAX_VARIANT_LABEL_FILE_BYTES {
        tracing::warn!(
            "custom_harnesses: label metadata {} is {} bytes, above the {} byte cap — ignoring",
            path.display(),
            metadata.len(),
            MAX_VARIANT_LABEL_FILE_BYTES
        );
        return None;
    }

    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(err) => {
            tracing::debug!(
                "custom_harnesses: cannot read label metadata {}: {err}",
                path.display()
            );
            return None;
        }
    };
    let document: serde_yaml::Value = match serde_yaml::from_str(&text) {
        Ok(document) => document,
        Err(err) => {
            tracing::debug!(
                "custom_harnesses: label metadata {} is not valid YAML: {err}",
                path.display()
            );
            return None;
        }
    };

    let mut cursor = &document;
    for segment in source.key.split('.').map(str::trim) {
        if segment.is_empty() {
            return None;
        }
        cursor = cursor.get(segment)?;
    }

    let value = match cursor {
        serde_yaml::Value::String(text) => text.clone(),
        serde_yaml::Value::Number(number) => number.to_string(),
        serde_yaml::Value::Bool(flag) => flag.to_string(),
        _ => return None,
    };
    let value = value.trim().to_string();
    (!value.is_empty()).then_some(value)
}

/// Resolve a leading `~` against the user's home directory.
///
/// Harness definition files are hand-written, so `~/.hermes/profiles` must mean
/// the same thing here as it does in the user's shell.
fn expand_tilde(dir: &str) -> PathBuf {
    let trimmed = dir.trim();
    let rest = trimmed
        .strip_prefix("~/")
        .or_else(|| trimmed.strip_prefix("~\\"))
        .or_else(|| (trimmed == "~").then_some(""));
    match rest {
        Some(rest) => match dirs::home_dir() {
            Some(home) if rest.is_empty() => home,
            Some(home) => home.join(rest),
            None => {
                tracing::warn!(
                    "custom_harnesses: no home directory available to expand {:?}",
                    trimmed
                );
                PathBuf::from(trimmed)
            }
        },
        None => PathBuf::from(trimmed),
    }
}

/// Lowercase a profile directory name into the `[a-z0-9_-]` fragment the id
/// grammar allows, collapsing every other run of characters to a single hyphen.
fn slugify_harness_id_fragment(name: &str) -> String {
    let mut out = String::new();
    let mut pending_hyphen = false;
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            if pending_hyphen && !out.is_empty() {
                out.push('-');
            }
            pending_hyphen = false;
            out.push(ch.to_ascii_lowercase());
        } else {
            pending_hyphen = true;
        }
    }
    out
}

/// Reject a `variants` block the spawn path would reject later.
///
/// A `variants` block is a template the loader expands into real entries, so
/// its dir, args, and env must satisfy the same invariants the template's own
/// fields do — otherwise the whole family vanishes at load with no signal.
pub(crate) fn validate_variants(def: &HarnessDefinition) -> Result<(), String> {
    let Some(variants) = def.variants.as_ref() else {
        return Ok(());
    };
    if variants.dir.trim().is_empty() {
        return Err("variants.dir must not be empty".into());
    }
    if let Some(arg) = variants.args.iter().find(|a| a.contains(',')) {
        return Err(format!(
            "variants.args: argument {arg:?} contains a comma — arguments are passed via a \
             comma-delimited transport and would be split at spawn time; \
             use separate argument entries instead"
        ));
    }
    crate::managed_agents::env_vars::validate_user_env_keys(&variants.env)
        .map_err(|e| format!("variants.env: {e}"))?;
    if let Some(label_from) = variants.label_from.as_ref() {
        if label_from.file.trim().is_empty() {
            return Err("variants.labelFrom.file must not be empty".into());
        }
        if label_from.key.trim().is_empty() {
            return Err("variants.labelFrom.key must not be empty".into());
        }
        // The file is opened relative to a profile directory, so a value
        // that escapes it would read arbitrary paths under a name the
        // reader does not expect. Rejected outright:
        //   * anything with a root (`/x`, `\x`, `C:\x`);
        //   * a Windows drive-relative prefix (`C:x`) — `has_root` is false
        //     for that shape, and on Linux it is not a prefix at all, so it
        //     is checked syntactically to behave the same on both;
        //   * any `..` component, wherever it appears.
        // A nested relative path (`nested/meta.yaml`) stays legal.
        let file = label_from.file.trim();
        let drive_relative = file.len() >= 2 && file.as_bytes()[1] == b':';
        let escapes = Path::new(file).has_root()
            || drive_relative
            || Path::new(file).components().any(|component| {
                matches!(
                    component,
                    std::path::Component::Prefix(_) | std::path::Component::ParentDir
                )
            });
        if escapes {
            return Err(format!(
                "variants.labelFrom.file {file:?} must be a relative path inside the profile \
                 directory"
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
