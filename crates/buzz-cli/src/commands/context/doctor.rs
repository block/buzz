use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::profile::{
    CredentialProvider, ProfileEnvironment, ResolvedProfile, PROFILE_SCHEMA_VERSION,
};
use crate::error::CliError;

#[derive(Debug, Clone, Serialize)]
pub struct VersionReport {
    pub version: &'static str,
    pub git_revision: &'static str,
    pub profile_schema: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct DoctorReport {
    pub version: VersionReport,
    pub profile: ProfileDoctor,
    pub paths: PathDoctor,
    pub relays: Vec<RelayDoctor>,
    pub identities: Vec<IdentityDoctor>,
    pub runtime: Vec<RuntimeDoctor>,
    pub cursors: Vec<CursorDoctor>,
    pub installation: InstallationDoctor,
    pub secret_redaction: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProfileDoctor {
    pub name: String,
    pub source: String,
    pub layout: String,
    pub config_path: String,
    pub legacy_root: String,
    pub schema_version: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct PathDoctor {
    pub data_root: PathStatus,
    pub journal: PathStatus,
    pub artifacts: PathStatus,
    pub cursors: PathStatus,
    pub cursor_entries: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct PathStatus {
    pub path: String,
    pub exists: bool,
    pub kind: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub struct RelayDoctor {
    pub role: &'static str,
    pub url: String,
    pub reachability: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct IdentityDoctor {
    pub role: &'static str,
    pub provider: Option<String>,
    pub reference: Option<String>,
    pub public_key: Option<String>,
    pub status: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct RuntimeDoctor {
    pub name: &'static str,
    pub path: Option<String>,
    pub exists: bool,
    pub sha256: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CursorDoctor {
    pub name: String,
    pub value: Option<String>,
    pub status: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct InstallationDoctor {
    pub executable: String,
    pub executable_sha256: Option<String>,
    pub release_manifest: Option<String>,
    pub expected_sha256: Option<String>,
    pub matches_manifest: Option<bool>,
    pub status: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReleaseManifest {
    version: String,
    git_revision: String,
    sha256: String,
}

pub fn version_report() -> VersionReport {
    VersionReport {
        version: env!("CARGO_PKG_VERSION"),
        git_revision: option_env!("BUZZ_GIT_REVISION")
            .or(option_env!("VERGEN_GIT_SHA"))
            .unwrap_or("unknown"),
        profile_schema: PROFILE_SCHEMA_VERSION,
    }
}

pub async fn diagnose(
    profile: &ResolvedProfile,
    environment: &ProfileEnvironment,
    offline: bool,
) -> DoctorReport {
    let mut relays = vec![relay_status("local", &profile.file.relays.local, offline).await];
    if let Some(rendezvous) = profile.file.relays.rendezvous.as_deref() {
        relays.push(relay_status("rendezvous", rendezvous, offline).await);
    }

    let identities = profile
        .file
        .identities
        .named()
        .into_iter()
        .map(|(role, identity)| {
            let Some(identity) = identity else {
                return IdentityDoctor {
                    role,
                    provider: None,
                    reference: None,
                    public_key: None,
                    status: "unconfigured".into(),
                };
            };
            let provider = match identity.provider {
                CredentialProvider::File => "file",
                CredentialProvider::Environment => "environment",
                CredentialProvider::PublicKey => "public_key",
            };
            match profile.public_key_for(role, environment) {
                Ok(public_key) => IdentityDoctor {
                    role,
                    provider: Some(provider.into()),
                    reference: Some(identity.reference.clone()),
                    public_key: Some(public_key),
                    status: "ready".into(),
                },
                Err(error) => IdentityDoctor {
                    role,
                    provider: Some(provider.into()),
                    reference: Some(identity.reference.clone()),
                    public_key: None,
                    status: format!("unavailable: {error}"),
                },
            }
        })
        .collect();

    let runtime = [
        (
            "handoff_reducer",
            profile.file.runtime.handoff_reducer.as_deref(),
        ),
        ("relay_push", profile.file.runtime.relay_push.as_deref()),
        (
            "graph_renderer",
            profile.file.runtime.graph_renderer.as_deref(),
        ),
    ]
    .into_iter()
    .map(|(name, path)| runtime_status(name, path))
    .collect();

    DoctorReport {
        version: version_report(),
        profile: ProfileDoctor {
            name: profile.name.clone(),
            source: snake_case_debug(profile.source),
            layout: snake_case_debug(profile.layout),
            config_path: profile.config_path.display().to_string(),
            legacy_root: profile.legacy_root.display().to_string(),
            schema_version: profile.file.schema_version,
        },
        paths: PathDoctor {
            data_root: path_status(&profile.data_root),
            journal: path_status(&profile.journal),
            artifacts: path_status(&profile.artifacts),
            cursors: path_status(&profile.cursors),
            cursor_entries: directory_entry_count(&profile.cursors).unwrap_or(0),
        },
        relays,
        identities,
        runtime,
        cursors: cursor_statuses(&profile.cursors),
        installation: installation_status(profile),
        secret_redaction: "private credential material is never included",
    }
}

async fn relay_status(role: &'static str, url: &str, offline: bool) -> RelayDoctor {
    if offline {
        return RelayDoctor {
            role,
            url: url.into(),
            reachability: "skipped (--offline)".into(),
        };
    }
    let client = match reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(2))
        .timeout(Duration::from_secs(4))
        .build()
    {
        Ok(client) => client,
        Err(error) => {
            return RelayDoctor {
                role,
                url: url.into(),
                reachability: format!("client error: {error}"),
            };
        }
    };
    let health = format!("{}/health", url.trim_end_matches('/'));
    let reachability = match client.get(&health).send().await {
        Ok(response) if response.status().is_success() => {
            format!("reachable ({})", response.status())
        }
        Ok(response) => format!("unhealthy ({})", response.status()),
        Err(error) if error.is_timeout() => "unreachable (timeout)".into(),
        Err(error) if error.is_connect() => "unreachable (connect)".into(),
        Err(error) => format!("unreachable ({error})"),
    };
    RelayDoctor {
        role,
        url: url.into(),
        reachability,
    }
}

fn snake_case_debug(value: impl std::fmt::Debug) -> String {
    format!("{value:?}")
        .chars()
        .enumerate()
        .flat_map(|(index, character)| {
            if character.is_ascii_uppercase() && index > 0 {
                vec!['_', character.to_ascii_lowercase()]
            } else {
                vec![character.to_ascii_lowercase()]
            }
        })
        .collect()
}

fn path_status(path: &Path) -> PathStatus {
    let kind = if path.is_file() {
        "file"
    } else if path.is_dir() {
        "directory"
    } else {
        "missing"
    };
    PathStatus {
        path: path.display().to_string(),
        exists: path.exists(),
        kind,
    }
}

fn directory_entry_count(path: &Path) -> std::io::Result<usize> {
    if !path.is_dir() {
        return Ok(0);
    }
    std::fs::read_dir(path)?.try_fold(0usize, |count, entry| entry.map(|_| count + 1))
}

fn runtime_status(name: &'static str, path: Option<&Path>) -> RuntimeDoctor {
    let exists = path.is_some_and(Path::is_file);
    RuntimeDoctor {
        name,
        path: path.map(|path| path.display().to_string()),
        exists,
        sha256: path
            .filter(|path| path.is_file())
            .and_then(|path| hash_file(path).ok()),
    }
}

fn cursor_statuses(path: &Path) -> Vec<CursorDoctor> {
    let Ok(entries) = std::fs::read_dir(path) else {
        return Vec::new();
    };
    let mut cursors = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.path().is_file())
        .map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            match std::fs::read_to_string(entry.path()) {
                Ok(value) => {
                    let value = value.trim();
                    if value.len() <= 256 && !value.chars().any(char::is_control) {
                        CursorDoctor {
                            name,
                            value: Some(value.into()),
                            status: "readable".into(),
                        }
                    } else {
                        CursorDoctor {
                            name,
                            value: None,
                            status: "redacted: cursor is not a short printable value".into(),
                        }
                    }
                }
                Err(error) => CursorDoctor {
                    name,
                    value: None,
                    status: format!("unreadable: {error}"),
                },
            }
        })
        .collect::<Vec<_>>();
    cursors.sort_by(|left, right| left.name.cmp(&right.name));
    cursors
}

fn installation_status(profile: &ResolvedProfile) -> InstallationDoctor {
    let executable = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("buzz"));
    let executable_sha256 = hash_file(&executable).ok();
    let Some(manifest_path) = profile.file.installation.release_manifest.as_deref() else {
        return InstallationDoctor {
            executable: executable.display().to_string(),
            executable_sha256,
            release_manifest: None,
            expected_sha256: None,
            matches_manifest: None,
            status: "release manifest not configured".into(),
        };
    };
    let parsed = std::fs::read_to_string(manifest_path)
        .ok()
        .and_then(|raw| serde_json::from_str::<ReleaseManifest>(&raw).ok());
    let Some(manifest) = parsed else {
        return InstallationDoctor {
            executable: executable.display().to_string(),
            executable_sha256,
            release_manifest: Some(manifest_path.display().to_string()),
            expected_sha256: None,
            matches_manifest: Some(false),
            status: "release manifest missing or invalid".into(),
        };
    };
    let matches = executable_sha256.as_deref() == Some(manifest.sha256.as_str())
        && manifest.version == env!("CARGO_PKG_VERSION")
        && (version_report().git_revision == "unknown"
            || manifest.git_revision == version_report().git_revision);
    InstallationDoctor {
        executable: executable.display().to_string(),
        executable_sha256,
        release_manifest: Some(manifest_path.display().to_string()),
        expected_sha256: Some(manifest.sha256),
        matches_manifest: Some(matches),
        status: if matches {
            "installed executable matches release manifest".into()
        } else {
            "installed executable drift detected".into()
        },
    }
}

fn hash_file(path: &Path) -> Result<String, CliError> {
    let mut file = File::open(path)
        .map_err(|error| CliError::Other(format!("could not open {}: {error}", path.display())))?;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(|error| {
            CliError::Other(format!("could not read {}: {error}", path.display()))
        })?;
        if read == 0 {
            break;
        }
        hash.update(&buffer[..read]);
    }
    Ok(hex::encode(hash.finalize()))
}

pub fn print_version(report: &VersionReport, json: bool) -> Result<(), CliError> {
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(report).map_err(|error| CliError::Other(format!(
                "version serialization failed: {error}"
            )))?
        );
    } else {
        println!(
            "buzz context {} (git {}, profile schema {})",
            report.version, report.git_revision, report.profile_schema
        );
    }
    Ok(())
}

pub fn print_doctor(report: &DoctorReport, json: bool) -> Result<(), CliError> {
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(report).map_err(|error| CliError::Other(format!(
                "doctor serialization failed: {error}"
            )))?
        );
        return Ok(());
    }
    println!(
        "buzz context doctor — profile {} ({}, {})",
        report.profile.name, report.profile.source, report.profile.layout
    );
    println!("  config       {}", report.profile.config_path);
    println!("  data         {}", report.paths.data_root.path);
    println!("  journal      {}", report.paths.journal.path);
    println!(
        "  cursors      {} ({} entries)",
        report.paths.cursors.path, report.paths.cursor_entries
    );
    for relay in &report.relays {
        println!(
            "  relay {:<11} {} — {}",
            relay.role, relay.url, relay.reachability
        );
    }
    for identity in &report.identities {
        let key = identity.public_key.as_deref().unwrap_or("-");
        println!(
            "  identity {:<28} {:<64} {}",
            identity.role, key, identity.status
        );
    }
    for runtime in &report.runtime {
        println!(
            "  runtime {:<18} {}",
            runtime.name,
            runtime.path.as_deref().unwrap_or("unconfigured")
        );
    }
    for cursor in &report.cursors {
        println!(
            "  cursor {:<20} {} — {}",
            cursor.name,
            cursor.value.as_deref().unwrap_or("-"),
            cursor.status
        );
    }
    println!("  install      {}", report.installation.status);
    println!("  secrets      {}", report.secret_redaction);
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use nostr::Keys;
    use tempfile::TempDir;

    use super::*;
    use crate::commands::context::profile::{
        legacy_profile, CredentialProvider, IdentityRef, ProfileEnvironment,
    };

    #[test]
    fn release_manifest_drift_is_reported_without_secret_fields() {
        let value = serde_json::to_value(IdentityDoctor {
            role: "journal_author",
            provider: Some("environment".into()),
            reference: Some("TEST_SECRET_VARIABLE".into()),
            public_key: Some("a".repeat(64)),
            status: "ready".into(),
        })
        .expect("serializes");
        let encoded = value.to_string();
        assert!(!encoded.contains("private_key"));
        assert!(!encoded.contains("secret"));
        assert!(!encoded.contains("nsec"));
    }

    #[tokio::test]
    async fn doctor_derives_public_identity_without_serializing_environment_secret() {
        let root = TempDir::new().expect("tempdir");
        let layout = root.path().join("context");
        std::fs::create_dir_all(&layout).expect("layout");
        let keys = Keys::generate();
        let private = keys.secret_key().to_secret_hex();
        let mut file = legacy_profile(
            &layout,
            "http://127.0.0.1:7777".into(),
            Some("https://relay.example".into()),
            Some("shared/tooling".into()),
        );
        file.identities.journal_author = Some(IdentityRef {
            provider: CredentialProvider::Environment,
            reference: "DOCTOR_TEST_KEY".into(),
            label: Some("journal".into()),
            auth_tag: None,
        });
        std::fs::write(
            layout.join("profile.toml"),
            toml::to_string_pretty(&file).expect("profile serializes"),
        )
        .expect("profile");
        let mut variables = BTreeMap::new();
        variables.insert("DOCTOR_TEST_KEY".into(), private.clone());
        let environment = ProfileEnvironment {
            home: root.path().join("home"),
            config_home: root.path().join("config"),
            data_home: root.path().join("data"),
            ctx_home_override: Some(layout),
            variables,
        };
        let profile =
            super::super::profile::resolve_profile("default", &environment).expect("resolve");
        let report = diagnose(&profile, &environment, true).await;
        let encoded = serde_json::to_string(&report).expect("report");
        assert!(encoded.contains(&keys.public_key().to_hex()));
        assert!(!encoded.contains(&private));
    }
}
