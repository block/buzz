//! Runtime-neutral content model for portable Agent Skills directories.
//!
//! This module preserves exact regular-file bytes, validates the Agent Skills
//! structure, and computes stable content identities. It deliberately defines
//! no archive, publication, installation, permission, or activation behavior.

use std::collections::{BTreeMap, BTreeSet};

use serde::Deserialize;
use sha2::{Digest as _, Sha256};
use unicode_casefold::UnicodeCaseFold as _;
use unicode_normalization::UnicodeNormalization as _;

/// Current portable Skill bundle schema.
pub const SKILL_BUNDLE_SCHEMA_VERSION: u16 = 1;

const MAX_SKILLS_PER_BUNDLE: usize = 32;
const MAX_FILES_PER_SKILL: usize = 256;
/// Maximum bytes in one file accepted by the portable Skill model.
pub const MAX_PORTABLE_SKILL_FILE_BYTES: usize = 16 * 1024 * 1024;
const MAX_SKILL_BUNDLE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_SKILL_PATH_BYTES: usize = 512;

/// A validated collection of complete Agent Skills directories.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SkillBundle {
    /// Schema used to validate and hash this bundle.
    pub schema_version: u16,
    /// Complete Skills. Input order does not affect the canonical digest.
    pub skills: Vec<PortableSkill>,
}

/// One complete Agent Skills directory.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PortableSkill {
    /// Agent Skills name and directory name.
    pub name: String,
    /// Discovery description parsed from `SKILL.md`.
    pub description: String,
    /// Every regular file below the Skill root.
    pub files: Vec<PortableSkillFile>,
}

/// One exact regular file below a Skill root.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PortableSkillFile {
    /// NFC-normalized forward-slash path relative to the Skill root.
    pub path: String,
    /// Exact bytes, including binary assets and empty files.
    pub bytes: Vec<u8>,
    /// Source executable intent. Validation never executes the file.
    pub executable: bool,
}

/// Parsed metadata and exact file inventory for review UI.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SkillBundleInspection {
    /// Stable digest of the complete bundle.
    pub digest: String,
    /// Skills in canonical name order.
    pub skills: Vec<PortableSkillInspection>,
}

/// Review information for one Skill.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PortableSkillInspection {
    /// Skill name.
    pub name: String,
    /// Discovery description.
    pub description: String,
    /// License name or reference to a bundled license file.
    pub license: Option<String>,
    /// Declared runtime or product compatibility constraints.
    pub compatibility: Option<String>,
    /// Arbitrary Agent Skills metadata preserved for exact review.
    pub metadata: BTreeMap<String, String>,
    /// Experimental, untrusted Agent Skills request. This is never a Buzz
    /// permission or tool grant.
    pub requested_allowed_tools: Option<String>,
    /// Stable digest of the exact Skill directory.
    pub digest: String,
    /// Complete canonical file inventory.
    pub files: Vec<PortableSkillFileInspection>,
}

/// Review information for one exact file.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PortableSkillFileInspection {
    /// Path relative to the Skill root.
    pub path: String,
    /// Exact byte length.
    pub size: u64,
    /// SHA-256 of exact bytes.
    pub sha256: String,
    /// Source executable intent.
    pub executable: bool,
    /// Whether the complete bytes are valid UTF-8 and can be shown as text.
    pub is_utf8: bool,
}

/// Advisory review finding in UTF-8 Skill content.
///
/// An empty result never proves that a bundle is free of sensitive data.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SkillBundleWarning {
    /// Skill containing the finding.
    pub skill_name: String,
    /// File containing the finding.
    pub path: String,
    /// Review guidance suitable for import/export UI.
    pub message: String,
}

/// Validated Agent Skills frontmatter for discovery and review.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PortableSkillMetadata {
    /// Agent Skills name. It must match the containing directory.
    name: String,
    /// Discovery description.
    description: String,
    /// License name or reference to a bundled license file.
    license: Option<String>,
    /// Declared runtime or product compatibility constraints.
    compatibility: Option<String>,
    /// Arbitrary Agent Skills metadata.
    metadata: BTreeMap<String, String>,
    /// Experimental, untrusted Agent Skills request. This is never a Buzz
    /// permission or tool grant.
    requested_allowed_tools: Option<String>,
}

impl PortableSkillMetadata {
    /// Agent Skills name matching the containing directory.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Discovery description.
    pub fn description(&self) -> &str {
        &self.description
    }

    /// License name or bundled license-file reference.
    pub fn license(&self) -> Option<&str> {
        self.license.as_deref()
    }

    /// Declared runtime or product compatibility constraints.
    pub fn compatibility(&self) -> Option<&str> {
        self.compatibility.as_deref()
    }

    /// Arbitrary Agent Skills metadata.
    pub fn metadata(&self) -> &BTreeMap<String, String> {
        &self.metadata
    }

    /// Experimental, untrusted tool request. This is never a Buzz grant.
    pub fn requested_allowed_tools(&self) -> Option<&str> {
        self.requested_allowed_tools.as_deref()
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
struct AgentSkillFrontmatter {
    name: String,
    description: String,
    #[serde(default)]
    license: Option<String>,
    #[serde(default)]
    compatibility: Option<String>,
    #[serde(default)]
    metadata: BTreeMap<String, String>,
    #[serde(default)]
    allowed_tools: Option<String>,
}

/// Parse and validate one `SKILL.md` for discovery without installing or
/// activating it. `expected_name` is the containing Skill directory name.
/// Filesystem callers should bounded-read no more than
/// [`MAX_PORTABLE_SKILL_FILE_BYTES`] before constructing this byte slice.
pub fn inspect_skill_md(
    expected_name: &str,
    bytes: &[u8],
) -> Result<PortableSkillMetadata, String> {
    if bytes.len() > MAX_PORTABLE_SKILL_FILE_BYTES {
        return Err(format!(
            "Skill {expected_name:?} SKILL.md exceeds {MAX_PORTABLE_SKILL_FILE_BYTES} bytes."
        ));
    }
    validate_skill_name(expected_name)?;
    let metadata = parse_skill_frontmatter(expected_name, bytes)?;
    if metadata.name != expected_name {
        return Err(format!(
            "Skill {expected_name:?} name must match its directory."
        ));
    }
    Ok(PortableSkillMetadata {
        name: metadata.name,
        description: metadata.description,
        license: metadata.license,
        compatibility: metadata.compatibility,
        metadata: metadata.metadata,
        requested_allowed_tools: metadata.allowed_tools,
    })
}

impl PortableSkill {
    /// Validate this exact Skill directory without installing or activating it.
    ///
    /// The fields remain mutable, so callers must revalidate after any change.
    pub fn validate(&self) -> Result<(), String> {
        validate_skill_name(&self.name)?;
        validate_skill_description(&self.name, &self.description)?;
        validate_skill_files(self, &mut 0)
    }

    /// Stable SHA-256 content identity for this exact Skill directory.
    ///
    /// This digest identifies reviewed content. It is not proof of trust,
    /// permission, installation, or runtime availability.
    pub fn canonical_digest(&self) -> Result<String, String> {
        self.validate()?;
        skill_digest(self)
    }

    /// Build exact review metadata for this Skill after revalidating it.
    ///
    /// `allowed-tools` remains an untrusted request and is never a Buzz grant.
    pub fn inspection(&self) -> Result<PortableSkillInspection, String> {
        self.validate()?;
        inspect_skill(self)
    }
}

impl SkillBundle {
    /// Validate exact contents without publishing, installing, or executing.
    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != SKILL_BUNDLE_SCHEMA_VERSION {
            return Err(format!(
                "Skill bundle schema version {} is unsupported.",
                self.schema_version
            ));
        }
        if self.skills.is_empty() || self.skills.len() > MAX_SKILLS_PER_BUNDLE {
            return Err(format!(
                "A bundle must contain 1 to {MAX_SKILLS_PER_BUNDLE} Skills."
            ));
        }

        let mut names = BTreeSet::new();
        let mut total_bytes = 0u64;
        for skill in &self.skills {
            validate_skill_name(&skill.name)?;
            validate_skill_description(&skill.name, &skill.description)?;
            if !names.insert(skill.name.clone()) {
                return Err(format!("Skill {:?} appears more than once.", skill.name));
            }
            validate_skill_files(skill, &mut total_bytes)?;
        }
        Ok(())
    }

    /// Stable SHA-256 identity of every Skill, path, byte, and executable bit.
    pub fn canonical_digest(&self) -> Result<String, String> {
        self.validate()?;
        let mut skills = self.skills.iter().collect::<Vec<_>>();
        skills.sort_unstable_by(|left, right| left.name.cmp(&right.name));
        let mut hasher = Sha256::new();
        hash_field(&mut hasher, b"buzz-portable-skill-bundle-v1");
        for skill in skills {
            hash_field(&mut hasher, skill_digest(skill)?.as_bytes());
        }
        Ok(hex::encode(hasher.finalize()))
    }

    /// Build exact review metadata. `allowed-tools` remains a request and the
    /// caller must never treat it as a permission grant.
    pub fn inspection(&self) -> Result<SkillBundleInspection, String> {
        self.validate()?;
        let digest = self.canonical_digest()?;
        let mut skills = self.skills.iter().collect::<Vec<_>>();
        skills.sort_unstable_by(|left, right| left.name.cmp(&right.name));
        let skills = skills
            .into_iter()
            .map(inspect_skill)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(SkillBundleInspection { digest, skills })
    }

    /// Find review risks in UTF-8 files without claiming that the bundle is
    /// safe. Binary files remain opaque and are identified by
    /// [`PortableSkillFileInspection::is_utf8`]. Warnings never alter bytes.
    pub fn review_warnings(&self) -> Result<Vec<SkillBundleWarning>, String> {
        self.validate()?;
        let mut warnings = Vec::new();
        for skill in &self.skills {
            for file in &skill.files {
                let Ok(content) = std::str::from_utf8(&file.bytes) else {
                    continue;
                };
                if file_looks_like_it_contains_a_secret(content) {
                    warnings.push(SkillBundleWarning {
                        skill_name: skill.name.clone(),
                        path: file.path.clone(),
                        message: "This file may contain a credential. Included files can contain sensitive data and must be reviewed before sharing."
                            .to_string(),
                    });
                }
                if content.chars().any(is_suspicious_unicode) {
                    warnings.push(SkillBundleWarning {
                        skill_name: skill.name.clone(),
                        path: file.path.clone(),
                        message: "This file contains invisible or directional Unicode formatting. Review the exact content before sharing or activation."
                            .to_string(),
                    });
                }
            }
        }
        Ok(warnings)
    }
}

fn validate_skill_files(skill: &PortableSkill, total_bytes: &mut u64) -> Result<(), String> {
    if skill.files.is_empty() || skill.files.len() > MAX_FILES_PER_SKILL {
        return Err(format!(
            "Skill {:?} must include 1 to {MAX_FILES_PER_SKILL} files.",
            skill.name
        ));
    }

    let mut paths = Vec::with_capacity(skill.files.len());
    let mut skill_md = None;
    for file in &skill.files {
        validate_portable_skill_path(&file.path)?;
        if file.bytes.len() > MAX_PORTABLE_SKILL_FILE_BYTES {
            return Err(format!(
                "{} in Skill {:?} exceeds {MAX_PORTABLE_SKILL_FILE_BYTES} bytes.",
                file.path, skill.name
            ));
        }
        *total_bytes = total_bytes
            .checked_add(file.bytes.len() as u64)
            .ok_or_else(|| "Skill bundle size overflowed.".to_string())?;
        if *total_bytes > MAX_SKILL_BUNDLE_BYTES {
            return Err(format!(
                "Skill files can contain at most {MAX_SKILL_BUNDLE_BYTES} bytes in total."
            ));
        }
        paths.push(file.path.as_str());
        if file.path == "SKILL.md" {
            skill_md = Some(file.bytes.as_slice());
        }
    }
    validate_portable_path_set(paths.into_iter())?;
    let skill_md =
        skill_md.ok_or_else(|| format!("Skill {:?} is missing SKILL.md.", skill.name))?;
    let metadata = inspect_skill_md(&skill.name, skill_md)?;
    if metadata.description != skill.description {
        return Err(format!(
            "Skill {:?} metadata must match SKILL.md.",
            skill.name
        ));
    }
    Ok(())
}

fn inspect_skill(skill: &PortableSkill) -> Result<PortableSkillInspection, String> {
    let skill_md = skill
        .files
        .iter()
        .find(|file| file.path == "SKILL.md")
        .ok_or_else(|| format!("Skill {:?} is missing SKILL.md.", skill.name))?;
    let metadata = inspect_skill_md(&skill.name, &skill_md.bytes)?;
    let mut files = skill
        .files
        .iter()
        .map(|file| PortableSkillFileInspection {
            path: file.path.clone(),
            size: file.bytes.len() as u64,
            sha256: sha256_hex(&file.bytes),
            executable: file.executable,
            is_utf8: std::str::from_utf8(&file.bytes).is_ok(),
        })
        .collect::<Vec<_>>();
    files.sort_unstable_by(|left, right| left.path.cmp(&right.path));
    Ok(PortableSkillInspection {
        name: skill.name.clone(),
        description: skill.description.clone(),
        license: metadata.license,
        compatibility: metadata.compatibility,
        metadata: metadata.metadata,
        requested_allowed_tools: metadata.requested_allowed_tools,
        digest: skill_digest(skill)?,
        files,
    })
}

fn parse_skill_frontmatter(
    skill_name: &str,
    bytes: &[u8],
) -> Result<AgentSkillFrontmatter, String> {
    let content = std::str::from_utf8(bytes)
        .map_err(|_| format!("Skill {skill_name:?} SKILL.md is not UTF-8."))?;
    let normalized = content.replace("\r\n", "\n");
    let remainder = normalized
        .strip_prefix("---\n")
        .ok_or_else(|| format!("Skill {skill_name:?} SKILL.md needs YAML frontmatter."))?;
    let frontmatter = if let Some(closing) = remainder.find("\n---\n") {
        &remainder[..=closing]
    } else if let Some(frontmatter) = remainder.strip_suffix("---") {
        frontmatter
            .ends_with('\n')
            .then_some(frontmatter)
            .ok_or_else(|| format!("Skill {skill_name:?} SKILL.md frontmatter is incomplete."))?
    } else {
        return Err(format!(
            "Skill {skill_name:?} SKILL.md frontmatter is incomplete."
        ));
    };
    let raw_metadata: serde_yaml::Value = serde_yaml::from_str(frontmatter)
        .map_err(|error| format!("Skill {skill_name:?} frontmatter is invalid: {error}"))?;
    validate_frontmatter_value_types(skill_name, &raw_metadata)?;
    let metadata: AgentSkillFrontmatter = serde_yaml::from_value(raw_metadata)
        .map_err(|error| format!("Skill {skill_name:?} frontmatter is invalid: {error}"))?;
    validate_optional_metadata(skill_name, &metadata)?;
    Ok(metadata)
}

fn validate_frontmatter_value_types(
    skill_name: &str,
    metadata: &serde_yaml::Value,
) -> Result<(), String> {
    let mapping = metadata.as_mapping().ok_or_else(|| {
        format!("Skill {skill_name:?} SKILL.md frontmatter must be a key-value mapping.")
    })?;
    for field in [
        "name",
        "description",
        "license",
        "compatibility",
        "allowed-tools",
    ] {
        if mapping
            .get(serde_yaml::Value::String(field.to_string()))
            .is_some_and(|value| !value.is_string())
        {
            return Err(format!(
                "Skill {skill_name:?} SKILL.md {field} must be a string."
            ));
        }
    }
    if let Some(value) = mapping.get(serde_yaml::Value::String("metadata".to_string())) {
        let entries = value
            .as_mapping()
            .ok_or_else(|| format!("Skill {skill_name:?} SKILL.md metadata must be a mapping."))?;
        if entries
            .iter()
            .any(|(key, value)| !key.is_string() || !value.is_string())
        {
            return Err(format!(
                "Skill {skill_name:?} SKILL.md metadata keys and values must be strings."
            ));
        }
    }
    Ok(())
}

fn validate_optional_metadata(
    skill_name: &str,
    metadata: &AgentSkillFrontmatter,
) -> Result<(), String> {
    validate_skill_description(skill_name, &metadata.description)?;
    if metadata
        .compatibility
        .as_ref()
        .is_some_and(|value| value.is_empty() || value.chars().count() > 500)
    {
        return Err(format!(
            "Skill {skill_name:?} compatibility must contain 1 to 500 characters."
        ));
    }
    Ok(())
}

fn validate_skill_name(name: &str) -> Result<(), String> {
    let normalized = name.nfkc().collect::<String>();
    let valid = !name.is_empty()
        && name.chars().count() <= 64
        && name.len() <= 255
        && name.encode_utf16().count() <= 255
        && normalized == name
        && name.chars().flat_map(char::to_lowercase).eq(name.chars())
        && name
            .chars()
            .all(|character| character.is_alphanumeric() || character == '-')
        && name.chars().next().is_some_and(char::is_alphanumeric)
        && name.chars().last().is_some_and(char::is_alphanumeric)
        && !name.contains("--");
    if valid && !windows_reserved_segment(name) {
        Ok(())
    } else {
        Err(format!(
            "Skill name {name:?} does not follow the Agent Skills slug format."
        ))
    }
}

fn validate_skill_description(skill_name: &str, value: &str) -> Result<(), String> {
    if value.trim().is_empty() || value.chars().count() > 1_024 {
        return Err(format!("Skill {skill_name:?} has an invalid description."));
    }
    Ok(())
}

fn validate_portable_skill_path(path: &str) -> Result<(), String> {
    if path.is_empty()
        || path.len() > MAX_SKILL_PATH_BYTES
        || path.starts_with('/')
        || path.contains('\\')
        || path.nfc().collect::<String>() != path
        || path.chars().any(|character| {
            character.is_control()
                || matches!(character, '<' | '>' | ':' | '"' | '|' | '?' | '*')
                || is_suspicious_unicode(character)
        })
    {
        return Err(format!("{path:?} is not a portable Skill path."));
    }
    let segments = path.split('/').collect::<Vec<_>>();
    if segments.iter().any(|segment| {
        segment.is_empty()
            || matches!(*segment, "." | "..")
            || segment.starts_with(' ')
            || segment.ends_with('.')
            || segment.ends_with(' ')
            || segment.len() > 255
            || segment.encode_utf16().count() > 255
            || windows_reserved_segment(segment)
    }) {
        return Err(format!("{path:?} is not a portable Skill path."));
    }
    Ok(())
}

fn validate_portable_path_set<'a>(paths: impl Iterator<Item = &'a str>) -> Result<(), String> {
    let mut normalized = BTreeMap::new();
    for path in paths {
        let key = portable_path_key(path);
        if let Some(other) = normalized.insert(key, path) {
            return Err(format!(
                "Skill paths {other:?} and {path:?} collide on a portable filesystem."
            ));
        }
    }
    for (key, path) in &normalized {
        for (index, _) in key.match_indices('/') {
            if let Some(ancestor) = normalized.get(&key[..index]) {
                return Err(format!(
                    "Skill paths {ancestor:?} and {path:?} collide on a portable filesystem."
                ));
            }
        }
    }
    Ok(())
}

fn portable_path_key(path: &str) -> String {
    let normalized = path.nfc().collect::<String>();
    normalized.as_str().case_fold().nfc().collect()
}

fn windows_reserved_segment(segment: &str) -> bool {
    let stem = segment
        .split_once('.')
        .map_or(segment, |(candidate, _)| candidate)
        .to_ascii_uppercase();
    matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || stem
            .strip_prefix("COM")
            .or_else(|| stem.strip_prefix("LPT"))
            .is_some_and(|suffix| {
                matches!(suffix, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9")
                    || matches!(suffix, "¹" | "²" | "³")
            })
}

fn is_suspicious_unicode(character: char) -> bool {
    matches!(
        character as u32,
        0x00AD
            | 0x034F
            | 0x061C
            | 0x180E
            | 0x200B
            | 0x200E..=0x200F
            | 0x202A..=0x202E
            | 0x2060..=0x206F
            | 0xFEFF
            | 0xFFF9..=0xFFFB
    )
}

fn file_looks_like_it_contains_a_secret(content: &str) -> bool {
    let lowercase = content.to_ascii_lowercase();
    if [
        "-----begin private key-----",
        "-----begin rsa private key-----",
        "-----begin openssh private key-----",
        "authorization: bearer ",
        "database_url=",
        "database_url:",
    ]
    .iter()
    .any(|marker| lowercase.contains(marker))
    {
        return true;
    }

    content.lines().any(|line| {
        let line = line.trim().trim_start_matches("export ").trim();
        let Some((key, value)) = line.split_once('=').or_else(|| line.split_once(':')) else {
            return false;
        };
        let key = key
            .trim_matches(|character: char| {
                character == '"' || character == '\'' || character.is_whitespace()
            })
            .to_ascii_uppercase();
        let sensitive_key = [
            "API_KEY",
            "TOKEN",
            "PASSWORD",
            "SECRET",
            "PRIVATE_KEY",
            "AUTHORIZATION",
            "DATABASE_URL",
        ]
        .iter()
        .any(|marker| key.contains(marker));
        if !sensitive_key {
            return false;
        }
        let value = value
            .trim()
            .trim_matches(|character| character == '"' || character == '\'');
        value.len() >= 8 && !looks_like_placeholder(value)
    })
}

fn looks_like_placeholder(value: &str) -> bool {
    let lowercase = value.to_ascii_lowercase();
    [
        "example",
        "placeholder",
        "your-",
        "your_",
        "replace",
        "xxxx",
        "<",
        "${",
        "test-only",
    ]
    .iter()
    .any(|marker| lowercase.contains(marker))
}

fn skill_digest(skill: &PortableSkill) -> Result<String, String> {
    let mut files = skill.files.iter().collect::<Vec<_>>();
    files.sort_unstable_by(|left, right| left.path.cmp(&right.path));
    let mut hasher = Sha256::new();
    hash_field(&mut hasher, b"buzz-portable-agent-skill-v1");
    hash_field(&mut hasher, skill.name.as_bytes());
    hash_field(&mut hasher, skill.description.as_bytes());
    for file in files {
        hash_field(&mut hasher, file.path.as_bytes());
        hash_field(&mut hasher, &file.bytes);
        hash_field(&mut hasher, &[u8::from(file.executable)]);
    }
    Ok(hex::encode(hasher.finalize()))
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn hash_field(hasher: &mut Sha256, value: &[u8]) {
    hasher.update((value.len() as u64).to_be_bytes());
    hasher.update(value);
}

#[cfg(test)]
#[path = "skill_bundle/tests.rs"]
mod tests;
