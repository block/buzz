use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use nostr::Keys;
use serde::{Deserialize, Serialize};

use crate::client::BuzzClient;
use crate::error::CliError;

pub const PROFILE_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProfileFile {
    pub schema_version: u32,
    pub data_root: PathBuf,
    #[serde(default)]
    pub relays: RelayConfig,
    #[serde(default)]
    pub paths: PathConfig,
    #[serde(default)]
    pub identities: IdentityRoles,
    #[serde(default)]
    pub context: ContextConfig,
    #[serde(default)]
    pub replication: ReplicationConfig,
    #[serde(default)]
    pub runtime: RuntimeConfig,
    #[serde(default)]
    pub installation: InstallationConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RelayConfig {
    #[serde(default = "default_local_relay")]
    pub local: String,
    pub rendezvous: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct PathConfig {
    pub journal: PathBuf,
    pub artifacts: PathBuf,
    pub cursors: PathBuf,
}

impl Default for PathConfig {
    fn default() -> Self {
        Self {
            journal: PathBuf::from("sovereign.ndjson"),
            artifacts: PathBuf::from("artifacts"),
            cursors: PathBuf::from("cursors"),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct IdentityRoles {
    pub journal_author: Option<IdentityRef>,
    pub replication_transport: Option<IdentityRef>,
    pub relay_witness: Option<IdentityRef>,
    pub agent: Option<IdentityRef>,
    pub steward: Option<IdentityRef>,
    pub artifact_source_reader: Option<IdentityRef>,
    pub artifact_destination_owner: Option<IdentityRef>,
}

impl IdentityRoles {
    pub fn named(&self) -> [(&'static str, Option<&IdentityRef>); 7] {
        [
            ("journal_author", self.journal_author.as_ref()),
            ("replication_transport", self.replication_transport.as_ref()),
            ("relay_witness", self.relay_witness.as_ref()),
            ("agent", self.agent.as_ref()),
            ("steward", self.steward.as_ref()),
            (
                "artifact_source_reader",
                self.artifact_source_reader.as_ref(),
            ),
            (
                "artifact_destination_owner",
                self.artifact_destination_owner.as_ref(),
            ),
        ]
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct IdentityRef {
    pub provider: CredentialProvider,
    pub reference: String,
    pub auth_tag: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CredentialProvider {
    File,
    Environment,
    PublicKey,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ContextConfig {
    pub default_h: Option<String>,
    #[serde(default)]
    pub streams: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ReplicationConfig {
    pub source: Option<String>,
    #[serde(default)]
    pub streams: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RuntimeConfig {
    pub handoff_reducer: Option<PathBuf>,
    pub relay_push: Option<PathBuf>,
    pub graph_renderer: Option<PathBuf>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct InstallationConfig {
    pub release_manifest: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProfileSource {
    Configured,
    ExplicitOverride,
    LegacyDetected,
    Fresh,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LayoutStatus {
    Ready,
    LegacyUnmigrated,
    FreshUnconfigured,
    Conflicting,
}

#[derive(Debug, Clone)]
pub struct ResolvedProfile {
    pub name: String,
    pub config_path: PathBuf,
    pub source: ProfileSource,
    pub layout: LayoutStatus,
    pub file: ProfileFile,
    pub data_root: PathBuf,
    pub journal: PathBuf,
    pub artifacts: PathBuf,
    pub cursors: PathBuf,
    pub legacy_root: PathBuf,
}

#[derive(Debug, Clone)]
pub struct ProfileEnvironment {
    pub home: PathBuf,
    pub config_home: PathBuf,
    pub data_home: PathBuf,
    pub ctx_home_override: Option<PathBuf>,
    pub variables: BTreeMap<String, String>,
}

impl ProfileEnvironment {
    pub fn from_process() -> Result<Self, CliError> {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .ok_or_else(|| CliError::Usage("HOME must be set for context profiles".into()))?;
        let config_home =
            nonempty_path_env("XDG_CONFIG_HOME").unwrap_or_else(|| home.join(".config"));
        let data_home =
            nonempty_path_env("XDG_DATA_HOME").unwrap_or_else(|| home.join(".local/share"));
        let ctx_home_override = nonempty_path_env("BUZZ_CTX_HOME");
        let variables = std::env::vars().collect();
        Ok(Self {
            home,
            config_home,
            data_home,
            ctx_home_override,
            variables,
        })
    }
}

fn nonempty_path_env(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

pub fn validate_profile_name(name: &str) -> Result<(), CliError> {
    if name.is_empty()
        || name.len() > 64
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"-_".contains(&byte))
    {
        return Err(CliError::Usage(
            "profile names must be 1-64 lowercase letters, digits, '-' or '_'".into(),
        ));
    }
    Ok(())
}

pub fn resolve_profile(
    name: &str,
    environment: &ProfileEnvironment,
) -> Result<ResolvedProfile, CliError> {
    validate_profile_name(name)?;
    let legacy_root = environment.home.join(".buzz-local");
    let explicit = environment.ctx_home_override.as_ref();
    let config_path = explicit.map_or_else(
        || {
            environment
                .config_home
                .join("buzz/profiles")
                .join(format!("{name}.toml"))
        },
        |root| root.join("profile.toml"),
    );
    let config_dir = config_path.parent().map(Path::to_path_buf).ok_or_else(|| {
        CliError::Other(format!(
            "profile path has no parent: {}",
            config_path.display()
        ))
    })?;
    let default_data_root = explicit
        .cloned()
        .unwrap_or_else(|| environment.data_home.join("buzz-local-relay").join(name));

    let compatibility_local = environment
        .variables
        .get("BUZZ_CTX_LOCAL")
        .cloned()
        .unwrap_or_else(default_local_relay);
    let compatibility_rendezvous = environment.variables.get("BUZZ_CTX_CLOUD").cloned();
    let compatibility_context = environment.variables.get("BUZZ_CTX_CONTEXT").cloned();

    let (mut file, source) = if config_path.is_file() {
        (
            parse_profile(&config_path)?,
            if explicit.is_some() {
                ProfileSource::ExplicitOverride
            } else {
                ProfileSource::Configured
            },
        )
    } else if let Some(root) = explicit {
        (
            legacy_profile(
                root,
                compatibility_local.clone(),
                compatibility_rendezvous.clone(),
                compatibility_context.clone(),
            ),
            ProfileSource::ExplicitOverride,
        )
    } else if path_has_entries(&legacy_root)? {
        (
            legacy_profile(
                &legacy_root,
                compatibility_local,
                compatibility_rendezvous,
                compatibility_context,
            ),
            ProfileSource::LegacyDetected,
        )
    } else {
        (fresh_profile(&default_data_root), ProfileSource::Fresh)
    };

    if file.schema_version != PROFILE_SCHEMA_VERSION {
        return Err(CliError::Usage(format!(
            "profile {} uses unsupported schema_version {} (expected {})",
            config_path.display(),
            file.schema_version,
            PROFILE_SCHEMA_VERSION
        )));
    }
    file.relays.local = normalize_http_url(&file.relays.local, "relays.local")?;
    if let Some(rendezvous) = file.relays.rendezvous.as_mut() {
        *rendezvous = normalize_http_url(rendezvous, "relays.rendezvous")?;
    }

    let data_root = resolve_path(&config_dir, &file.data_root);
    file.data_root = data_root.clone();
    let journal = resolve_path(&data_root, &file.paths.journal);
    let artifacts = resolve_path(&data_root, &file.paths.artifacts);
    let cursors = resolve_path(&data_root, &file.paths.cursors);
    resolve_profile_references(&mut file, &config_dir);

    let legacy_has_state = path_has_entries(&legacy_root)?;
    let data_has_state = path_has_entries(&data_root)?;
    let layout = if explicit.is_some() {
        LayoutStatus::Ready
    } else if source == ProfileSource::Fresh {
        LayoutStatus::FreshUnconfigured
    } else if source == ProfileSource::LegacyDetected {
        if data_root != legacy_root && data_has_state {
            LayoutStatus::Conflicting
        } else {
            LayoutStatus::LegacyUnmigrated
        }
    } else if legacy_has_state && data_root != legacy_root {
        LayoutStatus::Conflicting
    } else {
        LayoutStatus::Ready
    };

    Ok(ResolvedProfile {
        name: name.to_string(),
        config_path,
        source,
        layout,
        file,
        data_root,
        journal,
        artifacts,
        cursors,
        legacy_root,
    })
}

fn parse_profile(path: &Path) -> Result<ProfileFile, CliError> {
    let raw = std::fs::read_to_string(path)
        .map_err(|error| CliError::Other(format!("could not read {}: {error}", path.display())))?;
    toml::from_str(&raw)
        .map_err(|error| CliError::Usage(format!("invalid profile {}: {error}", path.display())))
}

fn normalize_http_url(value: &str, field: &str) -> Result<String, CliError> {
    let parsed = url::Url::parse(value)
        .map_err(|error| CliError::Usage(format!("{field} is not a valid URL: {error}")))?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        return Err(CliError::Usage(format!(
            "{field} must use http:// or https:// with a host"
        )));
    }
    Ok(value.trim_end_matches('/').to_string())
}

fn resolve_profile_references(file: &mut ProfileFile, config_dir: &Path) {
    for identity in [
        file.identities.journal_author.as_mut(),
        file.identities.replication_transport.as_mut(),
        file.identities.relay_witness.as_mut(),
        file.identities.agent.as_mut(),
        file.identities.steward.as_mut(),
        file.identities.artifact_source_reader.as_mut(),
        file.identities.artifact_destination_owner.as_mut(),
    ]
    .into_iter()
    .flatten()
    {
        if identity.provider == CredentialProvider::File {
            identity.reference = resolve_path(config_dir, Path::new(&identity.reference))
                .to_string_lossy()
                .into_owned();
        }
        if let Some(path) = identity.auth_tag.as_mut() {
            *path = resolve_path(config_dir, path);
        }
    }
    for path in [
        file.runtime.handoff_reducer.as_mut(),
        file.runtime.relay_push.as_mut(),
        file.runtime.graph_renderer.as_mut(),
        file.installation.release_manifest.as_mut(),
    ]
    .into_iter()
    .flatten()
    {
        *path = resolve_path(config_dir, path);
    }
}

fn resolve_path(base: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    }
}

fn path_has_entries(path: &Path) -> Result<bool, CliError> {
    if !path.exists() {
        return Ok(false);
    }
    if !path.is_dir() {
        return Ok(true);
    }
    let mut entries = std::fs::read_dir(path).map_err(|error| {
        CliError::Other(format!("could not inspect {}: {error}", path.display()))
    })?;
    Ok(entries
        .next()
        .transpose()
        .map_err(|error| CliError::Other(format!("could not inspect {}: {error}", path.display())))?
        .is_some())
}

fn fresh_profile(data_root: &Path) -> ProfileFile {
    ProfileFile {
        schema_version: PROFILE_SCHEMA_VERSION,
        data_root: data_root.to_path_buf(),
        relays: RelayConfig::default(),
        paths: PathConfig::default(),
        identities: IdentityRoles::default(),
        context: ContextConfig::default(),
        replication: ReplicationConfig::default(),
        runtime: RuntimeConfig::default(),
        installation: InstallationConfig::default(),
    }
}

pub fn legacy_profile(
    root: &Path,
    local_relay: String,
    rendezvous: Option<String>,
    default_context: Option<String>,
) -> ProfileFile {
    let key = root.join("node.key").to_string_lossy().into_owned();
    let role = || IdentityRef {
        provider: CredentialProvider::File,
        reference: key.clone(),
        auth_tag: None,
    };
    ProfileFile {
        schema_version: PROFILE_SCHEMA_VERSION,
        data_root: root.to_path_buf(),
        relays: RelayConfig {
            local: local_relay,
            rendezvous,
        },
        paths: PathConfig::default(),
        identities: IdentityRoles {
            journal_author: Some(role()),
            replication_transport: Some(role()),
            relay_witness: None,
            agent: None,
            steward: None,
            artifact_source_reader: Some(role()),
            artifact_destination_owner: Some(role()),
        },
        context: ContextConfig {
            default_h: default_context,
            streams: Vec::new(),
        },
        replication: ReplicationConfig {
            source: Some("local/sovereign".into()),
            streams: Vec::new(),
        },
        runtime: RuntimeConfig {
            handoff_reducer: Some(root.join("bin/buzz-handoff-state")),
            relay_push: Some(root.join("bin/buzz-relay-push")),
            graph_renderer: Some(root.join("bin/buzz-ctx-graph")),
        },
        installation: InstallationConfig::default(),
    }
}

impl Default for RelayConfig {
    fn default() -> Self {
        Self {
            local: default_local_relay(),
            rendezvous: None,
        }
    }
}

fn default_local_relay() -> String {
    "http://127.0.0.1:7777".into()
}

impl ResolvedProfile {
    pub fn require_ready(&self) -> Result<(), CliError> {
        match self.layout {
            LayoutStatus::Ready => Ok(()),
            LayoutStatus::LegacyUnmigrated => Err(CliError::Usage(format!(
                "legacy state detected at {}; run `buzz context --profile {} migrate` first or set BUZZ_CTX_HOME explicitly",
                self.legacy_root.display(),
                self.name
            ))),
            LayoutStatus::FreshUnconfigured => Err(CliError::Usage(format!(
                "profile {} is not configured; create {} or run context migrate",
                self.name,
                self.config_path.display()
            ))),
            LayoutStatus::Conflicting => Err(CliError::Conflict(format!(
                "legacy and profile-managed state both exist; refusing ambiguous layout for profile {}",
                self.name
            ))),
        }
    }

    pub fn identity(&self, role: &str) -> Option<&IdentityRef> {
        match role {
            "journal_author" => self.file.identities.journal_author.as_ref(),
            "replication_transport" => self.file.identities.replication_transport.as_ref(),
            "relay_witness" => self.file.identities.relay_witness.as_ref(),
            "agent" => self.file.identities.agent.as_ref(),
            "steward" => self.file.identities.steward.as_ref(),
            "artifact_source_reader" => self.file.identities.artifact_source_reader.as_ref(),
            "artifact_destination_owner" => {
                self.file.identities.artifact_destination_owner.as_ref()
            }
            _ => None,
        }
    }

    pub fn keys_for(&self, role: &str, environment: &ProfileEnvironment) -> Result<Keys, CliError> {
        let identity = self.identity(role).ok_or_else(|| {
            CliError::Auth(format!(
                "profile {} does not configure role {role}",
                self.name
            ))
        })?;
        match identity.provider {
            CredentialProvider::File => {
                let path = Path::new(&identity.reference);
                let secret = std::fs::read_to_string(path).map_err(|error| {
                    CliError::Auth(format!(
                        "could not read credential for role {role} from {}: {error}",
                        path.display()
                    ))
                })?;
                Keys::parse(secret.trim()).map_err(|error| {
                    CliError::Key(format!("credential for role {role} is invalid: {error}"))
                })
            }
            CredentialProvider::Environment => {
                let secret = environment.variables.get(&identity.reference).ok_or_else(|| {
                    CliError::Auth(format!(
                        "credential provider environment variable {} is unavailable for role {role}",
                        identity.reference
                    ))
                })?;
                Keys::parse(secret.trim()).map_err(|error| {
                    CliError::Key(format!("credential for role {role} is invalid: {error}"))
                })
            }
            CredentialProvider::PublicKey => Err(CliError::Auth(format!(
                "role {role} is public-only and cannot sign"
            ))),
        }
    }

    pub fn public_key_for(
        &self,
        role: &str,
        environment: &ProfileEnvironment,
    ) -> Result<String, CliError> {
        let identity = self.identity(role).ok_or_else(|| {
            CliError::Auth(format!(
                "profile {} does not configure role {role}",
                self.name
            ))
        })?;
        if identity.provider == CredentialProvider::PublicKey {
            nostr::PublicKey::from_hex(&identity.reference).map_err(|error| {
                CliError::Key(format!(
                    "public credential for role {role} is invalid: {error}"
                ))
            })?;
            return Ok(identity.reference.to_ascii_lowercase());
        }
        Ok(self.keys_for(role, environment)?.public_key().to_hex())
    }

    pub fn client_for(
        &self,
        role: &str,
        relay: &str,
        environment: &ProfileEnvironment,
    ) -> Result<BuzzClient, CliError> {
        let keys = self.keys_for(role, environment)?;
        let identity = self.identity(role).ok_or_else(|| {
            CliError::Auth(format!(
                "profile {} does not configure role {role}",
                self.name
            ))
        })?;
        let (auth_tag, auth_tag_json) = match identity.auth_tag.as_deref() {
            Some(path) => {
                let json = std::fs::read_to_string(path).map_err(|error| {
                    CliError::Auth(format!(
                        "could not read owner attestation for role {role} from {}: {error}",
                        path.display()
                    ))
                })?;
                let json = json.trim().to_string();
                let tag = buzz_sdk::nip_oa::parse_auth_tag(&json).map_err(|error| {
                    CliError::Auth(format!(
                        "owner attestation for role {role} is malformed: {error}"
                    ))
                })?;
                buzz_sdk::nip_oa::verify_auth_tag(&json, &keys.public_key()).map_err(|error| {
                    CliError::Auth(format!(
                        "owner attestation for role {role} is invalid: {error}"
                    ))
                })?;
                (Some(tag), Some(json))
            }
            None => (None, None),
        };
        BuzzClient::new(relay.to_string(), keys, auth_tag, auth_tag_json)
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    fn environment(root: &TempDir) -> ProfileEnvironment {
        let home = root.path().join("home");
        ProfileEnvironment {
            config_home: home.join(".config"),
            data_home: home.join(".local/share"),
            ctx_home_override: None,
            variables: BTreeMap::new(),
            home,
        }
    }

    #[test]
    fn fresh_profile_uses_xdg_roots_not_legacy_home() {
        let root = TempDir::new().expect("tempdir");
        let env = environment(&root);
        let resolved = resolve_profile("solo", &env).expect("profile resolves");
        assert_eq!(resolved.source, ProfileSource::Fresh);
        assert_eq!(resolved.layout, LayoutStatus::FreshUnconfigured);
        assert_eq!(
            resolved.config_path,
            env.config_home.join("buzz/profiles/solo.toml")
        );
        assert_eq!(
            resolved.data_root,
            env.data_home.join("buzz-local-relay/solo")
        );
        assert_ne!(resolved.data_root, env.home.join(".buzz-local"));
    }

    #[test]
    fn explicit_ctx_home_preserves_existing_profile_in_place() {
        let root = TempDir::new().expect("tempdir");
        let mut env = environment(&root);
        let legacy = env.home.join("custom-context");
        std::fs::create_dir_all(&legacy).expect("legacy dir");
        env.ctx_home_override = Some(legacy.clone());
        let resolved = resolve_profile("default", &env).expect("profile resolves");
        assert_eq!(resolved.source, ProfileSource::ExplicitOverride);
        assert_eq!(resolved.layout, LayoutStatus::Ready);
        assert_eq!(resolved.data_root, legacy);
    }

    #[test]
    fn unmanaged_legacy_layout_is_detected_without_becoming_the_default() {
        let root = TempDir::new().expect("tempdir");
        let env = environment(&root);
        let legacy = env.home.join(".buzz-local");
        std::fs::create_dir_all(&legacy).expect("legacy dir");
        std::fs::write(legacy.join("sovereign.ndjson"), "{}\n").expect("journal");
        let resolved = resolve_profile("default", &env).expect("profile resolves");
        assert_eq!(resolved.source, ProfileSource::LegacyDetected);
        assert_eq!(resolved.layout, LayoutStatus::LegacyUnmigrated);
    }

    #[test]
    fn profile_names_reject_path_traversal() {
        for invalid in ["", "../owner", "Upper", "space name", "name/child"] {
            assert!(validate_profile_name(invalid).is_err(), "{invalid}");
        }
    }
}
