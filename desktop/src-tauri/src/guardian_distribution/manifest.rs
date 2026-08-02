use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::HashSet;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Manifest {
    pub schema_version: u32,
    pub manifest_sequence: u64,
    pub component: String,
    pub version: String,
    pub cli_version: String,
    pub record_schema_version: String,
    pub source: Source,
    pub license: License,
    pub limits: Limits,
    pub artifacts: Vec<Artifact>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Source {
    pub repository: String,
    pub tag: String,
    pub commit: String,
    pub release_url: String,
    pub workflow_run_url: String,
    pub upstream_attestation: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct License {
    pub spdx: String,
    pub path: String,
    pub sha256: String,
    pub notice_path: String,
    pub notice_sha256: String,
}

#[derive(Debug, Deserialize, Clone, Copy)]
#[serde(deny_unknown_fields)]
pub(crate) struct Limits {
    pub max_entries: u64,
    pub max_expanded_size: u64,
    pub max_single_file_size: u64,
    pub max_compression_ratio: u64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Artifact {
    pub os: String,
    pub arch: String,
    pub archive_kind: ArchiveKind,
    pub asset_name: String,
    pub url: String,
    pub archive_sha256: String,
    pub archive_size: u64,
    pub expanded_size: u64,
    pub binary_path: String,
    pub binary_sha256: String,
    pub binary_size: u64,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ArchiveKind {
    TarGz,
    Zip,
}

pub(crate) fn builtin_manifest() -> Result<Manifest, String> {
    let manifest: Manifest = serde_json::from_str(include_str!("numbat-manifest-v1.json"))
        .map_err(|e| format!("invalid built-in Guardian manifest: {e}"))?;
    manifest.validate()?;
    Ok(manifest)
}

pub(crate) fn builtin_manifest_sha256() -> String {
    hex::encode(Sha256::digest(include_bytes!("numbat-manifest-v1.json")))
}

impl Manifest {
    fn validate(&self) -> Result<(), String> {
        if self.schema_version != 1
            || self.manifest_sequence != 1
            || self.component != "numbat"
            || self.version != "0.1.2"
            || self.cli_version != "0.1.2"
        {
            return Err("unsupported manifest identity".into());
        }
        if self.source.upstream_attestation.is_some() {
            return Err("v0.1.2 has no verified upstream attestation".into());
        }
        if self.source.repository != "https://github.com/perplexityai/numbat"
            || self.source.tag != "v0.1.2"
            || self.source.commit != "0e41ad66f5557f412eae330576271f2ee809d3de"
            || self.source.release_url
                != "https://github.com/perplexityai/numbat/releases/tag/v0.1.2"
            || self.source.workflow_run_url
                != "https://github.com/perplexityai/numbat/actions/runs/30680582672"
            || self.record_schema_version != "0.2.0"
            || self.license.spdx != "Apache-2.0"
            || self.license.path != "LICENSE"
            || self.license.notice_path != "THIRD_PARTY_LICENSES.txt"
        {
            return Err("invalid manifest metadata".into());
        }
        validate_hex(&self.license.sha256)?;
        validate_hex(&self.license.notice_sha256)?;
        if self.limits.max_entries == 0
            || self.limits.max_expanded_size == 0
            || self.limits.max_single_file_size == 0
            || self.limits.max_compression_ratio == 0
        {
            return Err("archive limits must be non-zero".into());
        }
        let mut targets = HashSet::new();
        for artifact in &self.artifacts {
            if !targets.insert((&artifact.os, &artifact.arch)) {
                return Err("duplicate target".into());
            }
            validate_hex(&artifact.archive_sha256)?;
            validate_hex(&artifact.binary_sha256)?;
            if artifact.archive_size == 0
                || artifact.binary_size == 0
                || artifact.expanded_size == 0
                || artifact.binary_size > artifact.expanded_size
                || artifact.expanded_size > self.limits.max_expanded_size
            {
                return Err("invalid artifact sizes".into());
            }
            let (expected_kind, expected_binary) = if artifact.os == "windows" {
                (ArchiveKind::Zip, "numbat.exe")
            } else {
                (ArchiveKind::TarGz, "numbat")
            };
            if artifact.archive_kind != expected_kind || artifact.binary_path != expected_binary {
                return Err("archive kind or binary path does not match target".into());
            }
            let expected_name = format!(
                "numbat_{}_{}_{}.{}",
                self.version,
                artifact.os,
                artifact.arch,
                if artifact.archive_kind == ArchiveKind::Zip {
                    "zip"
                } else {
                    "tar.gz"
                }
            );
            if artifact.asset_name != expected_name
                || artifact.url
                    != format!(
                        "https://github.com/perplexityai/numbat/releases/download/v{}/{}",
                        self.version, expected_name
                    )
            {
                return Err("artifact URL or name does not match target".into());
            }
        }
        let expected = HashSet::from([
            ("darwin", "amd64"),
            ("darwin", "arm64"),
            ("linux", "amd64"),
            ("linux", "arm64"),
            ("windows", "amd64"),
            ("windows", "arm64"),
        ]);
        if targets.len() != expected.len()
            || !targets
                .iter()
                .all(|(os, arch)| expected.contains(&(os.as_str(), arch.as_str())))
        {
            return Err("manifest must contain exactly the six authorized targets".into());
        }
        Ok(())
    }

    pub(crate) fn artifact_for(&self, os: &str, arch: &str) -> Result<&Artifact, String> {
        let os = match os {
            "macos" => "darwin",
            other => other,
        };
        let arch = match arch {
            "x86_64" => "amd64",
            "aarch64" => "arm64",
            other => other,
        };
        self.artifacts
            .iter()
            .find(|a| a.os == os && a.arch == arch)
            .ok_or_else(|| format!("unsupported Guardian target {os}/{arch}"))
    }
}

fn validate_hex(value: &str) -> Result<(), String> {
    if is_lower_hex(value, 64) {
        Ok(())
    } else {
        Err("digest must be 64 lowercase hexadecimal characters".into())
    }
}

fn is_lower_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn built_in_manifest_is_strict_and_selects_targets() {
        let manifest = builtin_manifest().unwrap();
        assert_eq!(
            manifest.artifact_for("macos", "aarch64").unwrap().arch,
            "arm64"
        );
        assert_eq!(
            manifest.artifact_for("windows", "x86_64").unwrap().arch,
            "amd64"
        );
        assert!(manifest.artifact_for("freebsd", "amd64").is_err());
    }
    #[test]
    fn unknown_fields_are_rejected() {
        let raw = include_str!("numbat-manifest-v1.json").replacen("{", "{\"surprise\":true,", 1);
        assert!(serde_json::from_str::<Manifest>(&raw).is_err());
    }

    #[test]
    fn target_layout_is_pinned() {
        let manifest = builtin_manifest().unwrap();
        for artifact in &manifest.artifacts {
            if artifact.os == "windows" {
                assert_eq!(artifact.archive_kind, ArchiveKind::Zip);
                assert_eq!(artifact.binary_path, "numbat.exe");
            } else {
                assert_eq!(artifact.archive_kind, ArchiveKind::TarGz);
                assert_eq!(artifact.binary_path, "numbat");
            }
        }
    }
}
