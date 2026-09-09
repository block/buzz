//! Shared browser plugin service and command data.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
/// Browser surface bounds in physical window coordinates.
pub struct Bounds {
    /// Left coordinate.
    pub x: f64,
    /// Top coordinate.
    pub y: f64,
    /// Surface width.
    pub width: f64,
    /// Surface height.
    pub height: f64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
/// Main-page load transition reported by the native browser.
pub enum PageLoad {
    /// A tracked load started.
    Started,
    /// A tracked load finished.
    Finished,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
/// Opaque identifier for one plugin session.
pub struct SessionId(pub String);

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
/// Authoritative registry summary for one installed plugin.
pub struct InstalledPlugin {
    /// Manifest plugin identifier.
    pub plugin_id: String,
    /// Display name.
    pub name: String,
    /// Installed version.
    pub version: String,
    /// Displayed publisher.
    pub publisher: String,
    /// Whether new sessions may start.
    pub enabled: bool,
    /// Browser contributions supplied by the plugin.
    pub contributions: Vec<ContributionSummary>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
/// Display summary for one browser contribution.
pub struct ContributionSummary {
    /// Manifest contribution identifier.
    pub contribution_id: String,
    /// Surface title.
    pub title: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
/// Browser surface identity returned to the command layer.
pub struct OpenedSurface {
    /// Opaque session identifier.
    pub session_id: String,
    /// Session generation used to reject stale events.
    pub generation: u64,
    /// Home URL admitted by the plugin.
    pub home_url: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
/// Fixed browser error payload emitted to the frontend.
pub struct BrowserError {
    /// Opaque session identifier.
    pub session_id: String,
    /// Session generation that produced the error.
    pub generation: u64,
    /// Closed browser error code.
    pub code: String,
    /// Host-owned fixed error message.
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
/// Strict browser plugin manifest.
pub struct Manifest {
    /// Package format version.
    pub package_format_version: String,
    /// Host contract version.
    pub contract_version: String,
    /// Reverse-domain plugin identifier.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Three-component semantic version.
    pub version: String,
    /// Displayed publisher.
    pub publisher: String,
    /// SPDX license identifier.
    pub license: String,
    /// Optional HTTPS project page.
    pub homepage: Option<String>,
    /// Native executable configuration.
    pub runtime: RuntimeManifest,
    /// Requested permission grants.
    pub grants: Vec<String>,
    /// Browser contributions supplied by the plugin.
    pub contributions: Vec<BrowserContribution>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
/// Stdio executable targets declared by a manifest.
pub struct RuntimeManifest {
    #[serde(rename = "type")]
    /// Runtime transport type.
    pub runtime_type: String,
    /// Executable metadata keyed by exact Cargo target triple.
    pub targets: BTreeMap<String, RuntimeTarget>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
/// Integrity metadata for one target executable.
pub struct RuntimeTarget {
    /// Package-relative executable path.
    pub path: String,
    /// Lowercase SHA-256 digest.
    pub sha256: String,
    /// Exact executable length.
    pub bytes: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
/// Browser contribution declared by a manifest.
pub struct BrowserContribution {
    /// Contribution kind.
    pub kind: String,
    /// Package-local contribution identifier.
    pub id: String,
    /// Surface title.
    pub title: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// Address category passed to `browser.resolve`.
pub enum ResolveKind {
    /// Resolve the contribution home page.
    Home,
    /// Resolve user-supplied address text.
    Address,
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// Host-admitted navigation returned by a plugin call.
pub struct Admitted {
    /// Absolute HTTP or HTTPS URL.
    pub url: String,
    /// Optional plugin-provided title.
    pub title: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// Active headless plugin session.
pub struct Session {
    /// Opaque session identifier.
    pub id: SessionId,
    /// Generation used to reject stale results.
    pub generation: u64,
    /// Home URL resolved when the session starts.
    pub home_url: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// Immutable staged package awaiting registry commit.
pub struct StagedPackage {
    /// Validated manifest copied into staging.
    pub manifest: Manifest,
    /// Digest of the staged target executable.
    pub executable_sha256: String,
    /// Host-owned staging directory.
    pub staged_path: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// Reason an install request was rejected.
pub enum InstallReject {
    /// Manifest content failed validation.
    InvalidManifest(String),
    /// Package layout failed validation.
    InvalidPackage(String),
    /// The manifest lacks the current Cargo target triple.
    UnknownTarget,
    /// Target executable bytes differ from the manifest digest.
    DigestMismatch,
    /// The manifest id already has an installed record, enabled or not.
    AlreadyInstalled,
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// Error returned by the headless plugin host.
pub enum PluginError {
    /// Package installation failed validation.
    Install(InstallReject),
    /// A resolved address failed host navigation policy.
    NavigationDenied { address: String },
    /// A plugin request exceeded its configured deadline.
    PluginTimeout,
    /// The plugin process exited or could not be used.
    PluginUnavailable,
    /// A plugin response violated the framed protocol.
    PluginProtocol(String),
    /// Discovery or tool inventory violated the contract.
    ContractMismatch(String),
    /// A lifecycle change invalidated an in-flight result.
    StaleGeneration,
    /// The session does not exist.
    NoSession,
    /// The requested platform operation is unsupported.
    Unsupported,
    /// The registry file could not be read, parsed, or written.
    Registry(String),
    /// No installed plugin has this id.
    UnknownPlugin,
}
