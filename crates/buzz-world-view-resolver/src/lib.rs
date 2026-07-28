mod presentation;

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use buzz_core::world_view::{
    WorldAuthorityRegistry, WorldViewBinding, WorldViewBindingScope, WorldViewBindingsDocument,
    WorldViewReference, WORLD_VIEW_BINDINGS_VERSION,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::io::AsyncWriteExt;
use uuid::Uuid;

pub use presentation::*;

const WORLD_VIEW_RESOLUTION_FORMAT_VERSION: u8 = 1;
const WORLD_VIEW_CATALOG_FORMAT_VERSION: u8 = 1;

/// Everything needed to resolve one binding without consulting ambient UI state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorldViewResolutionRequest {
    pub channel_id: Uuid,
    pub binding: WorldViewBinding,
    pub declared_scope: WorldViewBindingScope,
    pub effective_scope: WorldViewBindingScope,
    pub binding_revision_event_id: String,
}

impl WorldViewResolutionRequest {
    pub fn validate(&self) -> Result<(), WorldViewResolutionError> {
        WorldViewBindingsDocument {
            version: WORLD_VIEW_BINDINGS_VERSION,
            scope: self.declared_scope.clone(),
            bindings: vec![self.binding.clone()],
        }
        .validate()
        .map_err(WorldViewResolutionError::InvalidRequest)?;
        self.effective_scope
            .validate()
            .map_err(WorldViewResolutionError::InvalidRequest)?;
        validate_event_id("bindingRevisionEventId", &self.binding_revision_event_id)?;
        Ok(())
    }
}
/// Machine-local authorization selected for one exact world-view reference.
#[derive(Debug, Clone, PartialEq, Eq)]
enum WorldViewResolutionAccess {
    TrustedPublicOrigin {
        origin: String,
    },
    LocalSourceRoot {
        origin: String,
        mirror_id: String,
        source_root: PathBuf,
    },
    HostedEditShareFile {
        origin: String,
        hosted_world_id: String,
        credential_file: PathBuf,
    },
}

/// Credential-free authority readback for the source that produced a resolution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum WorldViewResolutionAuthority {
    HostedWorldViewExport {
        origin: String,
    },
    HostedWorldLiveViewShare {
        origin: String,
        #[serde(rename = "hostedWorldId")]
        hosted_world_id: String,
    },
    LocalWorldMirrorLatest {
        origin: String,
        #[serde(rename = "mirrorId")]
        mirror_id: String,
    },
    HostedWorldLatest {
        origin: String,
        #[serde(rename = "hostedWorldId")]
        hosted_world_id: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WorldViewResolutionFreshness {
    Pinned,
    LatestAtResolution,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedWorldViewEntity {
    pub name: String,
    pub qualified_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorldViewCatalogEntry {
    pub name: String,
    pub qualified_name: String,
    pub realm: ResolvedWorldViewEntity,
}

/// Canonical authored view identities available through one public source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorldViewCatalog {
    pub format_version: u8,
    pub revision: String,
    pub world_qualified_name: String,
    pub views: Vec<WorldViewCatalogEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResolvedWorldViewCounts {
    pub nodes: usize,
    pub edges: usize,
    pub ready: usize,
    pub actionable_ready: usize,
    pub satisfied: usize,
    pub blocked: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ResolvedWorldViewNodeStatus {
    Satisfied,
    Ready,
    Blocked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResolvedWorldViewNote {
    pub preview: Option<String>,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResolvedWorldViewSignalCase {
    pub name: String,
    pub evidence: Vec<WorldViewSignalEvidence>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ResolvedWorldViewSignalTarget {
    Preference,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ResolvedWorldViewSignalMode {
    First,
    All,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResolvedWorldViewSignal {
    pub name: String,
    pub target: ResolvedWorldViewSignalTarget,
    pub mode: ResolvedWorldViewSignalMode,
    pub cases: Vec<ResolvedWorldViewSignalCase>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResolvedWorldViewNode {
    pub preference: String,
    pub qualified_name: String,
    pub status: ResolvedWorldViewNodeStatus,
    pub actionable: bool,
    pub leaf: bool,
    pub in_focus: bool,
    pub in_satisfied: bool,
    pub blockers: Vec<String>,
    pub enablers: Vec<String>,
    pub note: ResolvedWorldViewNote,
    pub signals: Vec<ResolvedWorldViewSignal>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResolvedWorldViewEdge {
    pub downstream: String,
    pub upstream: String,
    pub relation: ResolvedWorldViewEdgeRelation,
    pub connection_type: WorldViewConnectionType,
    pub flowspace: String,
    pub flowspace_qualified_name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ResolvedWorldViewEdgeRelation {
    Blocker,
    Enabler,
}

/// Compact, agent-ready subset of `world view dump` with no untyped JSON payloads.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResolvedWorldViewDump {
    pub counts: ResolvedWorldViewCounts,
    pub nodes: Vec<ResolvedWorldViewNode>,
    pub ready_leaves: Vec<ResolvedWorldViewNode>,
    pub satisfied_nodes: Vec<ResolvedWorldViewNode>,
    pub blocked_nodes: Vec<ResolvedWorldViewNode>,
    pub edges: Vec<ResolvedWorldViewEdge>,
}

/// Canonical result shared by Buzz CLI, desktop, and agent prompt delivery.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResolvedWorldView {
    pub format_version: u8,
    pub binding_id: Uuid,
    pub channel_id: Uuid,
    pub declared_scope: WorldViewBindingScope,
    pub effective_scope: WorldViewBindingScope,
    pub binding_revision_event_id: String,
    pub source_revision: String,
    pub freshness: WorldViewResolutionFreshness,
    pub authority: WorldViewResolutionAuthority,
    pub realm: ResolvedWorldViewEntity,
    pub view: ResolvedWorldViewEntity,
    pub view_dump: ResolvedWorldViewDump,
    pub presentation: WorldViewPresentationVariants,
    pub resolved_at: DateTime<Utc>,
    pub next_command: String,
}

#[derive(Debug, Error)]
pub enum WorldViewResolutionError {
    #[error("invalid world-view resolution request: {0}")]
    InvalidRequest(String),
    #[error(
        "hosted world `{hosted_world_id}` has no private edit-share authority registered on this client"
    )]
    MissingHostedAuthority { hosted_world_id: String },
    #[error(
        "Shivai origin `{origin}` is not trusted on this device; trust it explicitly before resolving this binding"
    )]
    UntrustedOrigin { origin: String },
    #[error("could not launch the Shivai world resolver `{binary}`: {source}")]
    Launch {
        binary: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("could not send private Shivai world input to `{binary}` over stdin: {source}")]
    Input {
        binary: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("Shivai world-view resolution failed: {0}")]
    CommandFailed(String),
    #[error("Shivai hosted-world revision conflict: {0}")]
    RevisionConflict(String),
    #[error("Shivai world resolver returned invalid JSON: {0}")]
    Decode(#[from] serde_json::Error),
    #[error("Shivai world resolver returned an invalid result: {0}")]
    InvalidResult(String),
}

#[derive(Debug, Deserialize)]
struct WorldResultEnvelope<T> {
    ok: bool,
    result: Option<T>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WorldViewDumpResult {
    revision: String,
    hosted_world_id: Option<String>,
    realm: ResolvedWorldViewEntity,
    view: ResolvedWorldViewEntity,
    counts: ResolvedWorldViewCounts,
    presentation: WorldViewPresentationVariants,
    nodes: Vec<ResolvedWorldViewNode>,
    ready_leaves: Vec<ResolvedWorldViewNode>,
    satisfied_nodes: Vec<ResolvedWorldViewNode>,
    blocked_nodes: Vec<ResolvedWorldViewNode>,
    edges: Vec<ResolvedWorldViewEdge>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WorldViewCatalogResult {
    command: String,
    format_version: u8,
    revision: String,
    world_qualified_name: String,
    views: Vec<WorldViewCatalogEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostedEditShareInspection {
    pub hosted_world_id: String,
    pub revision: String,
}

/// Stable public live-view capability minted from private hosted authority.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PublishedHostedLiveViewShare {
    pub hosted_world_id: String,
    pub source_revision: String,
    pub package_revision: String,
    pub realm_qualified_name: String,
    pub view_qualified_name: String,
    pub share_token: String,
    pub share_url_path: String,
    pub title: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WorldHostedPublishLiveViewShareResult {
    command: String,
    live_view_share: WorldHostedPublishLiveViewShareResponse,
    revision: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WorldHostedPublishLiveViewShareResponse {
    live_view_share: WorldHostedLiveViewShare,
    source: WorldHostedLiveViewShareSource,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WorldHostedLiveViewShare {
    hosted_world_id: String,
    realm_qualified_name: String,
    share_token: String,
    share_url_path: String,
    title: String,
    view_qualified_name: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WorldHostedLiveViewShareSource {
    hosted_world_id: String,
    revision_id: String,
    package_revision: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WorldHostedLatestResult {
    projection: WorldHostedLatestProjection,
    revision: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WorldHostedLatestProjection {
    hosted_world_id: String,
}

/// Resolve using `SHIVAI_WORLD_BIN`, or `world` when the override is absent.
pub async fn resolve_world_view(
    request: WorldViewResolutionRequest,
    registry: &WorldAuthorityRegistry,
) -> Result<ResolvedWorldView, WorldViewResolutionError> {
    let binary = std::env::var_os("SHIVAI_WORLD_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("world"));
    resolve_world_view_with_binary(request, binary, registry).await
}

/// Resolve through one explicitly selected source `world` binary.
pub async fn resolve_world_view_with_binary(
    request: WorldViewResolutionRequest,
    binary: impl AsRef<Path>,
    registry: &WorldAuthorityRegistry,
) -> Result<ResolvedWorldView, WorldViewResolutionError> {
    request.validate()?;
    let access = resolution_access(registry, &request.binding.reference)?;
    let binary = binary.as_ref();
    let invocation = world_cli_invocation(&request.binding, &access)?;
    let stdout =
        run_world_cli_invocation(binary, invocation, &request.binding.reference, &access).await?;
    decode_world_view_resolution(&request, &stdout, Utc::now())
}

/// List canonical authored views using `SHIVAI_WORLD_BIN`, or `world`.
pub async fn catalog_world_views(
    reference: WorldViewReference,
    registry: &WorldAuthorityRegistry,
) -> Result<WorldViewCatalog, WorldViewResolutionError> {
    let binary = std::env::var_os("SHIVAI_WORLD_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("world"));
    catalog_world_views_with_binary(reference, binary, registry).await
}

/// List canonical authored views through one explicitly selected source binary.
pub async fn catalog_world_views_with_binary(
    reference: WorldViewReference,
    binary: impl AsRef<Path>,
    registry: &WorldAuthorityRegistry,
) -> Result<WorldViewCatalog, WorldViewResolutionError> {
    reference
        .validate()
        .map_err(WorldViewResolutionError::InvalidRequest)?;
    let access = resolution_access(registry, &reference)?;
    let binary = binary.as_ref();
    let invocation = world_view_cli_invocation(&reference, &access, "catalog")?;
    let stdout = run_world_cli_invocation(binary, invocation, &reference, &access).await?;
    decode_world_view_catalog(&stdout)
}

fn resolution_access(
    registry: &WorldAuthorityRegistry,
    reference: &WorldViewReference,
) -> Result<WorldViewResolutionAccess, WorldViewResolutionError> {
    let origin = reference.origin();
    if !registry.is_trusted_origin(origin) {
        return Err(WorldViewResolutionError::UntrustedOrigin {
            origin: origin.into(),
        });
    }

    match reference {
        WorldViewReference::LocalWorldMirrorLatest { origin, mirror_id } => {
            if let Some(authority) = registry.resolve_local(origin, mirror_id) {
                return Ok(WorldViewResolutionAccess::LocalSourceRoot {
                    origin: origin.clone(),
                    mirror_id: mirror_id.clone(),
                    source_root: authority.source_root.clone().into(),
                });
            }
            Ok(WorldViewResolutionAccess::TrustedPublicOrigin {
                origin: origin.clone(),
            })
        }
        WorldViewReference::HostedWorldViewExport { origin, .. }
        | WorldViewReference::HostedWorldLiveViewShare { origin, .. } => {
            Ok(WorldViewResolutionAccess::TrustedPublicOrigin {
                origin: origin.clone(),
            })
        }
        WorldViewReference::HostedWorldLatest {
            origin,
            hosted_world_id,
        } => {
            let authority = registry
                .resolve_hosted(origin, hosted_world_id)
                .ok_or_else(|| WorldViewResolutionError::MissingHostedAuthority {
                    hosted_world_id: hosted_world_id.clone(),
                })?;
            Ok(WorldViewResolutionAccess::HostedEditShareFile {
                origin: origin.clone(),
                hosted_world_id: hosted_world_id.clone(),
                credential_file: authority.credential_file.clone().into(),
            })
        }
    }
}

async fn run_world_cli_invocation(
    binary: &Path,
    invocation: WorldCliInvocation<'_>,
    reference: &WorldViewReference,
    access: &WorldViewResolutionAccess,
) -> Result<Vec<u8>, WorldViewResolutionError> {
    let mut command = tokio::process::Command::new(binary);
    command
        .args(&invocation.args)
        .kill_on_drop(true)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if invocation.stdin.is_some() {
        command.stdin(Stdio::piped());
    }
    let mut child = command
        .spawn()
        .map_err(|source| WorldViewResolutionError::Launch {
            binary: binary.to_owned(),
            source,
        })?;
    if let Some(input) = invocation.stdin {
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| WorldViewResolutionError::Input {
                binary: binary.to_owned(),
                source: std::io::Error::other("resolver stdin pipe was not available"),
            })?;
        stdin.write_all(input.as_bytes()).await.map_err(|source| {
            WorldViewResolutionError::Input {
                binary: binary.to_owned(),
                source,
            }
        })?;
    }
    let output =
        child
            .wait_with_output()
            .await
            .map_err(|source| WorldViewResolutionError::Launch {
                binary: binary.to_owned(),
                source,
            })?;
    if !output.status.success() {
        let diagnostics = redact_diagnostics(
            String::from_utf8_lossy(&output.stderr).trim(),
            reference,
            access,
        );
        return Err(command_failure(diagnostics));
    }
    Ok(output.stdout)
}

/// Inspect one private edit-share credential without placing it in process arguments.
pub async fn inspect_hosted_edit_share(
    origin: &str,
    credential_file: impl AsRef<Path>,
) -> Result<HostedEditShareInspection, WorldViewResolutionError> {
    let binary = std::env::var_os("SHIVAI_WORLD_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("world"));
    inspect_hosted_edit_share_with_binary(origin, credential_file, binary).await
}

/// Inspect one private edit-share credential through an explicit source `world` binary.
pub async fn inspect_hosted_edit_share_with_binary(
    origin: &str,
    credential_file: impl AsRef<Path>,
    binary: impl AsRef<Path>,
) -> Result<HostedEditShareInspection, WorldViewResolutionError> {
    let binary = binary.as_ref();
    let output = tokio::process::Command::new(binary)
        .args([
            "hosted",
            "latest",
            "--json",
            "--base-url",
            origin,
            "--edit-share-file",
            &credential_file.as_ref().to_string_lossy(),
            "--anonymous-session",
        ])
        .kill_on_drop(true)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|source| WorldViewResolutionError::Launch {
            binary: binary.to_owned(),
            source,
        })?;
    if !output.status.success() {
        let diagnostics = redact_credential_file_path(
            String::from_utf8_lossy(&output.stderr).trim(),
            credential_file.as_ref(),
        );
        return Err(command_failure(diagnostics));
    }

    let envelope: WorldResultEnvelope<WorldHostedLatestResult> =
        serde_json::from_slice(&output.stdout)?;
    if !envelope.ok {
        return Err(WorldViewResolutionError::InvalidResult(
            "the command returned a non-success envelope".into(),
        ));
    }
    let result = envelope.result.ok_or_else(|| {
        WorldViewResolutionError::InvalidResult("the success envelope omitted `result`".into())
    })?;
    if result.projection.hosted_world_id.trim().is_empty() {
        return Err(WorldViewResolutionError::InvalidResult(
            "the hosted-world id is blank".into(),
        ));
    }
    if result.revision.trim().is_empty() {
        return Err(WorldViewResolutionError::InvalidResult(
            "the hosted-world revision is blank".into(),
        ));
    }
    Ok(HostedEditShareInspection {
        hosted_world_id: result.projection.hosted_world_id,
        revision: result.revision,
    })
}

/// Apply one revision-checked hosted WorldLang script through machine-local
/// edit authority without exposing the credential path to the caller.
pub async fn apply_hosted_world_script(
    origin: &str,
    hosted_world_id: &str,
    credential_file: impl AsRef<Path>,
    expected_revision: &str,
    script: &str,
) -> Result<serde_json::Value, WorldViewResolutionError> {
    let binary = std::env::var_os("SHIVAI_WORLD_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("world"));
    apply_hosted_world_script_with_binary(
        origin,
        hosted_world_id,
        credential_file,
        expected_revision,
        script,
        binary,
    )
    .await
}

/// Apply one revision-checked hosted WorldLang script through an explicit
/// source `world` binary.
pub async fn apply_hosted_world_script_with_binary(
    origin: &str,
    hosted_world_id: &str,
    credential_file: impl AsRef<Path>,
    expected_revision: &str,
    script: &str,
    binary: impl AsRef<Path>,
) -> Result<serde_json::Value, WorldViewResolutionError> {
    let credential_file = credential_file.as_ref();
    let binary = binary.as_ref();
    validate_event_id("expectedRevision", expected_revision)?;
    if script.trim().is_empty() {
        return Err(WorldViewResolutionError::InvalidRequest(
            "hosted world script must not be blank".into(),
        ));
    }

    let inspection = inspect_hosted_edit_share_with_binary(origin, credential_file, binary).await?;
    if inspection.hosted_world_id != hosted_world_id {
        return Err(WorldViewResolutionError::InvalidResult(format!(
            "the private authority resolved hosted world `{}` instead of `{hosted_world_id}`",
            inspection.hosted_world_id
        )));
    }
    if inspection.revision != expected_revision {
        return Err(WorldViewResolutionError::RevisionConflict(format!(
            "expected revision `{expected_revision}`, current revision `{}`; no mutation was attempted",
            inspection.revision
        )));
    }

    let reference = WorldViewReference::HostedWorldLatest {
        origin: origin.to_owned(),
        hosted_world_id: hosted_world_id.to_owned(),
    };
    reference
        .validate()
        .map_err(WorldViewResolutionError::InvalidRequest)?;
    let access = WorldViewResolutionAccess::HostedEditShareFile {
        origin: origin.to_owned(),
        hosted_world_id: hosted_world_id.to_owned(),
        credential_file: credential_file.to_owned(),
    };
    let invocation = WorldCliInvocation {
        args: vec![
            "hosted".into(),
            "script".into(),
            "--json".into(),
            "--base-url".into(),
            origin.into(),
            "--edit-share-file".into(),
            credential_file.to_string_lossy().into_owned(),
            "--anonymous-session".into(),
            "--expected-revision".into(),
            expected_revision.into(),
            "--stdin".into(),
        ],
        stdin: Some(script),
    };
    let stdout = run_world_cli_invocation(binary, invocation, &reference, &access).await?;
    let mut envelope: serde_json::Value = serde_json::from_slice(&stdout)?;
    if envelope.get("ok").and_then(serde_json::Value::as_bool) != Some(true) {
        return invalid_result("the hosted script returned a non-success envelope");
    }
    let result = envelope
        .get("result")
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| {
            WorldViewResolutionError::InvalidResult(
                "the hosted script success envelope omitted `result`".into(),
            )
        })?;
    if result.get("command").and_then(serde_json::Value::as_str) != Some("hosted script") {
        return invalid_result("the hosted script result carried an unexpected command");
    }
    if result
        .get("revision")
        .and_then(serde_json::Value::as_str)
        .is_none_or(str::is_empty)
    {
        return invalid_result("the hosted script result omitted its revision");
    }
    redact_credential_file_path_in_json(&mut envelope, credential_file);
    Ok(envelope)
}

/// Apply one revision-checked local WorldLang script through a machine-local
/// broker without exposing the mutable source root to the caller.
pub async fn apply_local_world_script(
    origin: &str,
    mirror_id: &str,
    source_root: impl AsRef<Path>,
    expected_revision: &str,
    script: &str,
) -> Result<serde_json::Value, WorldViewResolutionError> {
    let binary = std::env::var_os("SHIVAI_WORLD_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("world"));
    apply_local_world_script_with_binary(
        origin,
        mirror_id,
        source_root,
        expected_revision,
        script,
        binary,
    )
    .await
}

/// Apply one revision-checked local WorldLang script through an explicit
/// source `world` binary.
pub async fn apply_local_world_script_with_binary(
    origin: &str,
    mirror_id: &str,
    source_root: impl AsRef<Path>,
    expected_revision: &str,
    script: &str,
    binary: impl AsRef<Path>,
) -> Result<serde_json::Value, WorldViewResolutionError> {
    validate_event_id("expectedRevision", expected_revision)?;
    if script.trim().is_empty() {
        return Err(WorldViewResolutionError::InvalidRequest(
            "local world script must not be blank".into(),
        ));
    }
    let reference = WorldViewReference::LocalWorldMirrorLatest {
        origin: origin.to_owned(),
        mirror_id: mirror_id.to_owned(),
    };
    reference
        .validate()
        .map_err(WorldViewResolutionError::InvalidRequest)?;

    let source_root = source_root.as_ref();
    let binary = binary.as_ref();
    let output = tokio::process::Command::new(binary)
        .args([
            "script",
            "--json",
            "--root",
            source_root.to_string_lossy().as_ref(),
            "--expected-revision",
            expected_revision,
            "--stdin",
        ])
        .kill_on_drop(true)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|source| WorldViewResolutionError::Launch {
            binary: binary.to_owned(),
            source,
        })?;
    let mut child = output;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| WorldViewResolutionError::Input {
            binary: binary.to_owned(),
            source: std::io::Error::other("local script stdin pipe was not available"),
        })?;
    stdin
        .write_all(script.as_bytes())
        .await
        .map_err(|source| WorldViewResolutionError::Input {
            binary: binary.to_owned(),
            source,
        })?;
    drop(stdin);
    let output =
        child
            .wait_with_output()
            .await
            .map_err(|source| WorldViewResolutionError::Launch {
                binary: binary.to_owned(),
                source,
            })?;
    if !output.status.success() {
        let diagnostics =
            redact_local_world_root(String::from_utf8_lossy(&output.stderr).trim(), source_root);
        return Err(command_failure(diagnostics));
    }

    let mut envelope: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    if envelope.get("ok").and_then(serde_json::Value::as_bool) != Some(true) {
        return invalid_result("the local script returned a non-success envelope");
    }
    let result = envelope
        .get("result")
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| {
            WorldViewResolutionError::InvalidResult(
                "the local script success envelope omitted `result`".into(),
            )
        })?;
    if result.get("command").and_then(serde_json::Value::as_str) != Some("script") {
        return invalid_result("the local script result carried an unexpected command");
    }
    if result
        .get("revision")
        .and_then(serde_json::Value::as_str)
        .is_none_or(str::is_empty)
    {
        return invalid_result("the local script result omitted its revision");
    }
    redact_local_world_root_in_json(&mut envelope, source_root);
    Ok(envelope)
}

/// Mint or reuse a stable public live-view share using `SHIVAI_WORLD_BIN`.
pub async fn publish_hosted_live_view_share(
    origin: &str,
    credential_file: impl AsRef<Path>,
    view_qualified_name: &str,
) -> Result<PublishedHostedLiveViewShare, WorldViewResolutionError> {
    let binary = std::env::var_os("SHIVAI_WORLD_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("world"));
    publish_hosted_live_view_share_with_binary(origin, credential_file, view_qualified_name, binary)
        .await
}

/// Mint or reuse a stable public live-view share through an explicit `world` binary.
pub async fn publish_hosted_live_view_share_with_binary(
    origin: &str,
    credential_file: impl AsRef<Path>,
    view_qualified_name: &str,
    binary: impl AsRef<Path>,
) -> Result<PublishedHostedLiveViewShare, WorldViewResolutionError> {
    let binary = binary.as_ref();
    let output = tokio::process::Command::new(binary)
        .args([
            "hosted",
            "view",
            "share-live",
            "--json",
            "--base-url",
            origin,
            "--edit-share-file",
            &credential_file.as_ref().to_string_lossy(),
            "--anonymous-session",
            "--view",
            view_qualified_name,
        ])
        .kill_on_drop(true)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|source| WorldViewResolutionError::Launch {
            binary: binary.to_owned(),
            source,
        })?;
    if !output.status.success() {
        let diagnostics = redact_credential_file_path(
            String::from_utf8_lossy(&output.stderr).trim(),
            credential_file.as_ref(),
        );
        return Err(WorldViewResolutionError::CommandFailed(
            if diagnostics.is_empty() {
                "the command exited without diagnostics".into()
            } else {
                diagnostics
            },
        ));
    }

    let envelope: WorldResultEnvelope<WorldHostedPublishLiveViewShareResult> =
        serde_json::from_slice(&output.stdout)?;
    if !envelope.ok {
        return invalid_result("the command returned a non-success envelope");
    }
    let result = envelope.result.ok_or_else(|| {
        WorldViewResolutionError::InvalidResult("the success envelope omitted `result`".into())
    })?;
    if result.command != "hosted view share-live" {
        return invalid_result(format!(
            "unexpected live-share command `{}`",
            result.command
        ));
    }
    let published = result.live_view_share;
    let live_view_share = published.live_view_share;
    let source = published.source;
    for (field, value) in [
        ("revision", result.revision.as_str()),
        ("source.hostedWorldId", source.hosted_world_id.as_str()),
        ("source.revisionId", source.revision_id.as_str()),
        ("source.packageRevision", source.package_revision.as_str()),
        (
            "liveViewShare.hostedWorldId",
            live_view_share.hosted_world_id.as_str(),
        ),
        (
            "liveViewShare.realmQualifiedName",
            live_view_share.realm_qualified_name.as_str(),
        ),
        (
            "liveViewShare.viewQualifiedName",
            live_view_share.view_qualified_name.as_str(),
        ),
        (
            "liveViewShare.shareToken",
            live_view_share.share_token.as_str(),
        ),
        (
            "liveViewShare.shareUrlPath",
            live_view_share.share_url_path.as_str(),
        ),
        ("liveViewShare.title", live_view_share.title.as_str()),
    ] {
        if value.trim().is_empty() {
            return invalid_result(format!("live-share `{field}` is blank"));
        }
    }
    if result.revision != source.package_revision {
        return invalid_result(
            "live-share result revision did not match its source package revision",
        );
    }
    if source.revision_id == source.package_revision {
        return invalid_result(
            "live-share source revision id unexpectedly matched its package revision",
        );
    }
    if live_view_share.hosted_world_id != source.hosted_world_id {
        return invalid_result("live-share hosted world did not match its source hosted world");
    }
    if live_view_share.view_qualified_name != view_qualified_name {
        return invalid_result(format!(
            "live-share view `{}` did not match requested `{view_qualified_name}`",
            live_view_share.view_qualified_name
        ));
    }

    Ok(PublishedHostedLiveViewShare {
        hosted_world_id: source.hosted_world_id,
        source_revision: source.revision_id,
        package_revision: source.package_revision,
        realm_qualified_name: live_view_share.realm_qualified_name,
        view_qualified_name: live_view_share.view_qualified_name,
        share_token: live_view_share.share_token,
        share_url_path: live_view_share.share_url_path,
        title: live_view_share.title,
    })
}

fn decode_world_view_catalog(stdout: &[u8]) -> Result<WorldViewCatalog, WorldViewResolutionError> {
    let envelope: WorldResultEnvelope<WorldViewCatalogResult> = serde_json::from_slice(stdout)?;
    if !envelope.ok {
        return invalid_result("the command returned a non-success envelope");
    }
    let result = envelope.result.ok_or_else(|| {
        WorldViewResolutionError::InvalidResult("the success envelope omitted `result`".into())
    })?;
    if result.command != "view.catalog" {
        return invalid_result(format!("unexpected catalog command `{}`", result.command));
    }
    if result.format_version != WORLD_VIEW_CATALOG_FORMAT_VERSION {
        return invalid_result(format!(
            "unsupported catalog format version {}",
            result.format_version
        ));
    }
    if result.revision.trim().is_empty() {
        return invalid_result("catalog `revision` is blank");
    }
    if result.world_qualified_name.trim().is_empty() {
        return invalid_result("catalog `worldQualifiedName` is blank");
    }

    let mut qualified_names = HashSet::with_capacity(result.views.len());
    for view in &result.views {
        if view.name.trim().is_empty()
            || view.qualified_name.trim().is_empty()
            || view.realm.name.trim().is_empty()
            || view.realm.qualified_name.trim().is_empty()
        {
            return invalid_result("catalog view names and realm identities must not be blank");
        }
        if !qualified_names.insert(&view.qualified_name) {
            return invalid_result(format!(
                "duplicate catalog view qualified name `{}`",
                view.qualified_name
            ));
        }
    }

    Ok(WorldViewCatalog {
        format_version: result.format_version,
        revision: result.revision,
        world_qualified_name: result.world_qualified_name,
        views: result.views,
    })
}

fn decode_world_view_resolution(
    request: &WorldViewResolutionRequest,
    stdout: &[u8],
    resolved_at: DateTime<Utc>,
) -> Result<ResolvedWorldView, WorldViewResolutionError> {
    let envelope: WorldResultEnvelope<WorldViewDumpResult> = serde_json::from_slice(stdout)?;
    if !envelope.ok {
        return Err(WorldViewResolutionError::InvalidResult(
            "the command returned a non-success envelope".into(),
        ));
    }
    let result = envelope.result.ok_or_else(|| {
        WorldViewResolutionError::InvalidResult("the success envelope omitted `result`".into())
    })?;
    validate_dump_result(request, &result)?;

    let (authority, freshness) = authority_readback(&request.binding.reference, &result)?;
    Ok(ResolvedWorldView {
        format_version: WORLD_VIEW_RESOLUTION_FORMAT_VERSION,
        binding_id: request.binding.id,
        channel_id: request.channel_id,
        declared_scope: request.declared_scope.clone(),
        effective_scope: request.effective_scope.clone(),
        binding_revision_event_id: request.binding_revision_event_id.clone(),
        source_revision: result.revision,
        freshness,
        authority,
        realm: result.realm,
        view: result.view,
        view_dump: ResolvedWorldViewDump {
            counts: result.counts,
            nodes: result.nodes,
            ready_leaves: result.ready_leaves,
            satisfied_nodes: result.satisfied_nodes,
            blocked_nodes: result.blocked_nodes,
            edges: result.edges,
        },
        presentation: result.presentation,
        resolved_at,
        next_command: next_command(request),
    })
}

fn validate_dump_result(
    request: &WorldViewResolutionRequest,
    result: &WorldViewDumpResult,
) -> Result<(), WorldViewResolutionError> {
    if result.revision.trim().is_empty() {
        return invalid_result("`revision` is blank");
    }
    if result.realm.qualified_name != request.binding.realm_qualified_name {
        return invalid_result(format!(
            "realm `{}` did not match requested `{}`",
            result.realm.qualified_name, request.binding.realm_qualified_name
        ));
    }
    if result.view.qualified_name != request.binding.view_qualified_name {
        return invalid_result(format!(
            "view `{}` did not match requested `{}`",
            result.view.qualified_name, request.binding.view_qualified_name
        ));
    }
    if result.counts.nodes != result.nodes.len() {
        return invalid_result(format!(
            "node count {} did not match {} returned nodes",
            result.counts.nodes,
            result.nodes.len()
        ));
    }
    if result.counts.edges != result.edges.len() {
        return invalid_result(format!(
            "edge count {} did not match {} returned edges",
            result.counts.edges,
            result.edges.len()
        ));
    }
    if result.presentation.format_version != WORLD_VIEW_RESOLUTION_FORMAT_VERSION {
        return invalid_result(format!(
            "unsupported presentation format version {}",
            result.presentation.format_version
        ));
    }
    for (appearance, model) in [
        ("dark", &result.presentation.dark),
        ("light", &result.presentation.light),
    ] {
        if model.selection.realm_qualified_name != request.binding.realm_qualified_name
            || model.selection.view_qualified_name != request.binding.view_qualified_name
        {
            return invalid_result(format!(
                "{appearance} presentation selection did not match the requested realm/view"
            ));
        }
        if model.revision.as_deref() != Some(result.revision.as_str()) {
            return invalid_result(format!(
                "{appearance} presentation revision did not match the resolved source revision"
            ));
        }
    }
    Ok(())
}

fn invalid_result<T>(message: impl Into<String>) -> Result<T, WorldViewResolutionError> {
    Err(WorldViewResolutionError::InvalidResult(message.into()))
}

fn authority_readback(
    reference: &WorldViewReference,
    result: &WorldViewDumpResult,
) -> Result<(WorldViewResolutionAuthority, WorldViewResolutionFreshness), WorldViewResolutionError>
{
    match reference {
        WorldViewReference::HostedWorldViewExport { origin, .. } => Ok((
            WorldViewResolutionAuthority::HostedWorldViewExport {
                origin: origin.clone(),
            },
            WorldViewResolutionFreshness::Pinned,
        )),
        WorldViewReference::HostedWorldLiveViewShare { origin, .. } => {
            let hosted_world_id = result
                .hosted_world_id
                .as_deref()
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| {
                    WorldViewResolutionError::InvalidResult(
                        "hosted live-view resolution omitted `hostedWorldId`".into(),
                    )
                })?;
            Ok((
                WorldViewResolutionAuthority::HostedWorldLiveViewShare {
                    origin: origin.clone(),
                    hosted_world_id: hosted_world_id.to_owned(),
                },
                WorldViewResolutionFreshness::LatestAtResolution,
            ))
        }
        WorldViewReference::LocalWorldMirrorLatest { origin, mirror_id } => Ok((
            WorldViewResolutionAuthority::LocalWorldMirrorLatest {
                origin: origin.clone(),
                mirror_id: mirror_id.clone(),
            },
            WorldViewResolutionFreshness::LatestAtResolution,
        )),
        WorldViewReference::HostedWorldLatest {
            origin,
            hosted_world_id,
        } => Ok((
            WorldViewResolutionAuthority::HostedWorldLatest {
                origin: origin.clone(),
                hosted_world_id: hosted_world_id.clone(),
            },
            WorldViewResolutionFreshness::LatestAtResolution,
        )),
    }
}

struct WorldCliInvocation<'a> {
    args: Vec<String>,
    stdin: Option<&'a str>,
}

fn world_cli_invocation<'a>(
    binding: &'a WorldViewBinding,
    access: &WorldViewResolutionAccess,
) -> Result<WorldCliInvocation<'a>, WorldViewResolutionError> {
    let mut invocation = world_view_cli_invocation(&binding.reference, access, "dump")?;
    invocation.args.extend([
        "--realm".into(),
        binding.realm_qualified_name.clone(),
        "--view".into(),
        binding.view_qualified_name.clone(),
    ]);
    Ok(invocation)
}

fn world_view_cli_invocation<'a>(
    reference: &'a WorldViewReference,
    access: &WorldViewResolutionAccess,
    subcommand: &str,
) -> Result<WorldCliInvocation<'a>, WorldViewResolutionError> {
    let (args, stdin) = match reference {
        WorldViewReference::LocalWorldMirrorLatest { origin, mirror_id } => match access {
            WorldViewResolutionAccess::LocalSourceRoot {
                origin: authorized_origin,
                mirror_id: authorized_mirror_id,
                source_root,
            } if authorized_origin == origin && authorized_mirror_id == mirror_id => (
                vec![
                    "view".into(),
                    subcommand.into(),
                    "--json".into(),
                    "--root".into(),
                    source_root.to_string_lossy().into_owned(),
                ],
                None,
            ),
            WorldViewResolutionAccess::TrustedPublicOrigin {
                origin: authorized_origin,
            } if authorized_origin == origin => (
                vec![
                    "hosted".into(),
                    "view".into(),
                    subcommand.into(),
                    "--json".into(),
                    "--base-url".into(),
                    origin.clone(),
                    "--local-mirror".into(),
                    mirror_id.clone(),
                ],
                None,
            ),
            _ => return Err(resolution_access_mismatch()),
        },
        WorldViewReference::HostedWorldViewExport {
            origin,
            share_token,
        } => {
            let WorldViewResolutionAccess::TrustedPublicOrigin {
                origin: authorized_origin,
            } = access
            else {
                return Err(resolution_access_mismatch());
            };
            if authorized_origin != origin {
                return Err(resolution_access_mismatch());
            }
            (
                vec![
                    "hosted".into(),
                    "view".into(),
                    subcommand.into(),
                    "--json".into(),
                    "--base-url".into(),
                    origin.clone(),
                    "--share-token-stdin".into(),
                ],
                Some(share_token.as_str()),
            )
        }
        WorldViewReference::HostedWorldLiveViewShare {
            origin,
            share_token,
        } => {
            let WorldViewResolutionAccess::TrustedPublicOrigin {
                origin: authorized_origin,
            } = access
            else {
                return Err(resolution_access_mismatch());
            };
            if authorized_origin != origin {
                return Err(resolution_access_mismatch());
            }
            (
                vec![
                    "hosted".into(),
                    "view".into(),
                    subcommand.into(),
                    "--json".into(),
                    "--base-url".into(),
                    origin.clone(),
                    "--live-share-token-stdin".into(),
                ],
                Some(share_token.as_str()),
            )
        }
        WorldViewReference::HostedWorldLatest {
            origin,
            hosted_world_id,
        } => {
            let WorldViewResolutionAccess::HostedEditShareFile {
                origin: authorized_origin,
                hosted_world_id: authorized_hosted_world_id,
                credential_file,
            } = access
            else {
                return Err(resolution_access_mismatch());
            };
            if authorized_origin != origin || authorized_hosted_world_id != hosted_world_id {
                return Err(resolution_access_mismatch());
            }
            (
                vec![
                    "hosted".into(),
                    "view".into(),
                    subcommand.into(),
                    "--json".into(),
                    "--base-url".into(),
                    origin.clone(),
                    "--edit-share-file".into(),
                    credential_file.to_string_lossy().into_owned(),
                    "--anonymous-session".into(),
                ],
                None,
            )
        }
    };
    Ok(WorldCliInvocation { args, stdin })
}

fn redact_diagnostics(
    diagnostics: &str,
    reference: &WorldViewReference,
    access: &WorldViewResolutionAccess,
) -> String {
    let diagnostics = match reference {
        WorldViewReference::HostedWorldViewExport { share_token, .. } => {
            diagnostics.replace(share_token, "<redacted>")
        }
        WorldViewReference::HostedWorldLiveViewShare { share_token, .. } => {
            diagnostics.replace(share_token, "<redacted>")
        }
        WorldViewReference::LocalWorldMirrorLatest { .. }
        | WorldViewReference::HostedWorldLatest { .. } => diagnostics.to_owned(),
    };
    match access {
        WorldViewResolutionAccess::HostedEditShareFile {
            credential_file, ..
        } => redact_credential_file_path(&diagnostics, credential_file),
        WorldViewResolutionAccess::LocalSourceRoot { source_root, .. } => {
            redact_local_world_root(&diagnostics, source_root)
        }
        WorldViewResolutionAccess::TrustedPublicOrigin { .. } => diagnostics,
    }
}

fn resolution_access_mismatch() -> WorldViewResolutionError {
    WorldViewResolutionError::InvalidRequest(
        "world-view resolution authorization does not match the current binding reference".into(),
    )
}

fn command_failure(diagnostics: String) -> WorldViewResolutionError {
    if diagnostics.contains("world.hosted.revision_conflict")
        || diagnostics.contains("world.workflow.revision_conflict")
    {
        WorldViewResolutionError::RevisionConflict(diagnostics)
    } else {
        WorldViewResolutionError::CommandFailed(diagnostics)
    }
}

fn redact_credential_file_path_in_json(value: &mut serde_json::Value, credential_file: &Path) {
    match value {
        serde_json::Value::String(text) => {
            *text = redact_credential_file_path(text, credential_file);
        }
        serde_json::Value::Array(values) => {
            for value in values {
                redact_credential_file_path_in_json(value, credential_file);
            }
        }
        serde_json::Value::Object(values) => {
            for value in values.values_mut() {
                redact_credential_file_path_in_json(value, credential_file);
            }
        }
        serde_json::Value::Null | serde_json::Value::Bool(_) | serde_json::Value::Number(_) => {}
    }
}

fn redact_credential_file_path(diagnostics: &str, credential_file: &Path) -> String {
    if credential_file.as_os_str().is_empty() {
        return diagnostics.to_owned();
    }
    diagnostics.replace(
        credential_file.to_string_lossy().as_ref(),
        "<redacted-credential-file>",
    )
}

fn redact_local_world_root_in_json(value: &mut serde_json::Value, source_root: &Path) {
    match value {
        serde_json::Value::String(text) => {
            *text = redact_local_world_root(text, source_root);
        }
        serde_json::Value::Array(values) => {
            for value in values {
                redact_local_world_root_in_json(value, source_root);
            }
        }
        serde_json::Value::Object(values) => {
            for value in values.values_mut() {
                redact_local_world_root_in_json(value, source_root);
            }
        }
        serde_json::Value::Null | serde_json::Value::Bool(_) | serde_json::Value::Number(_) => {}
    }
}

fn redact_local_world_root(diagnostics: &str, source_root: &Path) -> String {
    if source_root.as_os_str().is_empty() {
        return diagnostics.to_owned();
    }
    diagnostics.replace(
        source_root.to_string_lossy().as_ref(),
        "<redacted-local-world-root>",
    )
}

fn next_command(request: &WorldViewResolutionRequest) -> String {
    let mut command = format!("buzz world-views resolve --channel {}", request.channel_id);
    if let Some(thread_root_event_id) = request.declared_scope.thread_root_event_id() {
        command.push_str(" --thread-root ");
        command.push_str(thread_root_event_id);
    }
    command.push_str(" --binding ");
    command.push_str(&request.binding.id.to_string());
    command
}

fn validate_event_id(field: &str, value: &str) -> Result<(), WorldViewResolutionError> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(WorldViewResolutionError::InvalidRequest(format!(
            "{field} must be 64 lowercase hex characters"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use buzz_core::world_view::WorldViewDisplayMode;
    use chrono::TimeZone;
    use serde_json::json;

    fn request(reference: WorldViewReference) -> WorldViewResolutionRequest {
        WorldViewResolutionRequest {
            channel_id: Uuid::parse_str("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa").unwrap(),
            binding: WorldViewBinding {
                id: Uuid::parse_str("11111111-1111-4111-8111-111111111111").unwrap(),
                label: Some("Launch board".into()),
                reference,
                realm_qualified_name: "world::main".into(),
                view_qualified_name: "world::main::@Board".into(),
                display_mode: WorldViewDisplayMode::Graph,
            },
            declared_scope: WorldViewBindingScope::Channel,
            effective_scope: WorldViewBindingScope::Channel,
            binding_revision_event_id: "b".repeat(64),
        }
    }

    fn presentation(revision: &str) -> serde_json::Value {
        let graph = json!({
            "kind": "ready",
            "graphBackgroundHex": "#111113",
            "graphPattern": "dots",
            "clusters": [],
            "nodes": [{
                "id": "world::main::Ship",
                "label": "Ship",
                "preferenceQualifiedName": "world::main::Ship",
                "status": "ready",
                "targetState": null,
                "isReady": true,
                "isLeaf": true,
                "signalCases": [{
                    "caseName": "implementing",
                    "evidence": [{
                        "kind": "typedForm",
                        "appearance": null,
                        "formQualifiedName": "coordination::AgentAssignment",
                        "matchedEntries": [],
                        "value": {
                            "kind": "object",
                            "entries": [{
                                "key": "state",
                                "value": {
                                    "kind": "string",
                                    "value": "active"
                                }
                            }]
                        }
                    }],
                    "signalName": "CodexThread",
                    "meanings": [
                        { "kind": "targetState", "state": "implementing" },
                        { "kind": "codexThread" }
                    ]
                }],
                "signalCaseNames": ["implementing"],
                "fillHex": "#1c2024",
                "borderHex": "#3e63dd",
                "textHex": "#f0f0f3",
                "deemphasis": null,
                "effect": null,
                "position": { "x": 150, "y": 57.5 },
                "size": { "width": 300, "height": 115 }
            }],
            "edges": [],
            "bounds": { "width": 420, "height": 235 }
        });
        let model = json!({
            "graph": graph,
            "revision": revision,
            "selection": {
                "realmQualifiedName": "world::main",
                "viewQualifiedName": "world::main::@Board"
            }
        });
        json!({ "formatVersion": 1, "dark": model, "light": model })
    }

    fn success_stdout(revision: &str) -> Vec<u8> {
        serde_json::to_vec(&json!({
            "ok": true,
            "result": {
                "revision": revision,
                "realm": { "name": "main", "qualifiedName": "world::main" },
                "view": {
                    "name": "Board",
                    "qualifiedName": "world::main::@Board",
                    "slots": {},
                    "flowspaces": [],
                    "lenses": []
                },
                "counts": {
                    "nodes": 1,
                    "edges": 0,
                    "ready": 1,
                    "actionableReady": 1,
                    "satisfied": 0,
                    "blocked": 0
                },
                "presentation": presentation(revision),
                "nodes": [{
                    "preference": "Ship",
                    "qualifiedName": "world::main::Ship",
                    "status": "ready",
                    "actionable": true,
                    "leaf": true,
                    "inFocus": false,
                    "inSatisfied": false,
                    "blockers": [],
                    "enablers": [],
                    "note": { "preview": null, "truncated": false },
                    "signals": []
                }],
                "readyLeaves": [{
                    "preference": "Ship",
                    "qualifiedName": "world::main::Ship",
                    "status": "ready",
                    "actionable": true,
                    "leaf": true,
                    "inFocus": false,
                    "inSatisfied": false,
                    "blockers": [],
                    "enablers": [],
                    "note": { "preview": null, "truncated": false },
                    "signals": []
                }],
                "satisfiedNodes": [],
                "blockedNodes": [],
                "edges": []
            }
        }))
        .unwrap()
    }

    #[test]
    fn decodes_canonical_view_catalog_identities() {
        let stdout = serde_json::to_vec(&json!({
            "ok": true,
            "result": {
                "command": "view.catalog",
                "formatVersion": 1,
                "revision": "source-revision-1",
                "root": "hosted-local-mirror:mirror-1",
                "worldQualifiedName": "world",
                "views": [{
                    "name": "@Board",
                    "qualifiedName": "@main::Board",
                    "realm": {
                        "name": "main",
                        "qualifiedName": "world::main"
                    }
                }]
            },
            "diagnostics": []
        }))
        .unwrap();

        let catalog = decode_world_view_catalog(&stdout).unwrap();

        assert_eq!(catalog.world_qualified_name, "world");
        assert_eq!(catalog.views[0].qualified_name, "@main::Board");
        assert_eq!(catalog.views[0].realm.qualified_name, "world::main");
    }

    #[test]
    fn catalog_routes_export_capability_over_stdin_without_a_selection() {
        let reference = WorldViewReference::HostedWorldViewExport {
            origin: "https://manifest.shivai.space".into(),
            share_token: "secret-view-token".into(),
        };

        let invocation = world_view_cli_invocation(
            &reference,
            &WorldViewResolutionAccess::TrustedPublicOrigin {
                origin: "https://manifest.shivai.space".into(),
            },
            "catalog",
        )
        .unwrap();

        assert_eq!(&invocation.args[..3], ["hosted", "view", "catalog"]);
        assert!(invocation
            .args
            .iter()
            .any(|argument| argument == "--share-token-stdin"));
        assert!(!invocation.args.iter().any(|argument| argument == "--realm"));
        assert_eq!(invocation.stdin, Some("secret-view-token"));
    }

    #[test]
    fn decodes_one_typed_resolution_and_omits_the_hosted_token() {
        let request = request(WorldViewReference::HostedWorldViewExport {
            origin: "https://manifest.shivai.space".into(),
            share_token: "secret-view-token".into(),
        });
        let resolved_at = Utc.with_ymd_and_hms(2026, 7, 24, 12, 0, 0).unwrap();
        let resolved = decode_world_view_resolution(
            &request,
            &success_stdout("source-revision-1"),
            resolved_at,
        )
        .unwrap();

        assert_eq!(resolved.binding_id, request.binding.id);
        assert_eq!(resolved.source_revision, "source-revision-1");
        assert_eq!(resolved.view_dump.counts.nodes, 1);
        assert_eq!(resolved.freshness, WorldViewResolutionFreshness::Pinned);
        let encoded = serde_json::to_string(&resolved).unwrap();
        assert!(!encoded.contains("secret-view-token"));
        assert!(encoded.contains("buzz world-views resolve"));
    }

    #[test]
    fn rejects_a_selection_that_does_not_match_the_binding() {
        let mut value: serde_json::Value =
            serde_json::from_slice(&success_stdout("source-revision-1")).unwrap();
        value["result"]["view"]["qualifiedName"] = json!("world::main::@Wrong");
        let error = decode_world_view_resolution(
            &request(WorldViewReference::LocalWorldMirrorLatest {
                origin: "https://manifest.shivai.space".into(),
                mirror_id: "mirror-1".into(),
            }),
            &serde_json::to_vec(&value).unwrap(),
            Utc::now(),
        )
        .expect_err("mismatched view must fail");

        assert!(error.to_string().contains("did not match requested"));
    }

    #[test]
    fn redacts_hosted_tokens_from_command_diagnostics() {
        let reference = WorldViewReference::HostedWorldViewExport {
            origin: "https://manifest.shivai.space".into(),
            share_token: "secret-view-token".into(),
        };
        assert_eq!(
            redact_diagnostics(
                "failed secret-view-token",
                &reference,
                &WorldViewResolutionAccess::TrustedPublicOrigin {
                    origin: "https://manifest.shivai.space".into(),
                },
            ),
            "failed <redacted>"
        );
    }

    #[test]
    fn redacts_host_credential_paths_from_command_diagnostics() {
        let reference = WorldViewReference::HostedWorldLatest {
            origin: "https://manifest.shivai.space".into(),
            hosted_world_id: "hosted-1".into(),
        };
        let access = WorldViewResolutionAccess::HostedEditShareFile {
            origin: "https://manifest.shivai.space".into(),
            hosted_world_id: "hosted-1".into(),
            credential_file: PathBuf::from("/private/edit-share.txt"),
        };

        assert_eq!(
            redact_diagnostics(
                "failed to read /private/edit-share.txt",
                &reference,
                &access,
            ),
            "failed to read <redacted-credential-file>"
        );
    }
    #[test]
    fn routes_private_local_read_authority_through_the_source_root() {
        let request = request(WorldViewReference::LocalWorldMirrorLatest {
            origin: "https://manifest.shivai.space".into(),
            mirror_id: "mirror-1".into(),
        });
        let access = WorldViewResolutionAccess::LocalSourceRoot {
            origin: "https://manifest.shivai.space".into(),
            mirror_id: "mirror-1".into(),
            source_root: PathBuf::from("/private/delivery.world"),
        };

        let invocation = world_cli_invocation(&request.binding, &access).unwrap();

        assert_eq!(&invocation.args[..3], ["view", "dump", "--json"]);
        assert!(invocation
            .args
            .windows(2)
            .any(|pair| pair == ["--root", "/private/delivery.world"]));
        assert!(!invocation
            .args
            .iter()
            .any(|argument| argument == "--local-mirror"));
        assert_eq!(invocation.stdin, None);
        assert_eq!(
            redact_diagnostics(
                "failed to read /private/delivery.world",
                &request.binding.reference,
                &access,
            ),
            "failed to read <redacted-local-world-root>"
        );
    }

    #[test]
    fn routes_hosted_export_capability_over_stdin_not_process_arguments() {
        let request = request(WorldViewReference::HostedWorldViewExport {
            origin: "https://manifest.shivai.space".into(),
            share_token: "secret-view-token".into(),
        });

        let invocation = world_cli_invocation(
            &request.binding,
            &WorldViewResolutionAccess::TrustedPublicOrigin {
                origin: "https://manifest.shivai.space".into(),
            },
        )
        .unwrap();

        assert!(invocation
            .args
            .iter()
            .any(|argument| argument == "--share-token-stdin"));
        assert!(!invocation.args.join(" ").contains("secret-view-token"));
        assert_eq!(invocation.stdin, Some("secret-view-token"));
    }

    #[test]
    fn routes_private_hosted_read_authority_through_a_credential_file() {
        let request = request(WorldViewReference::HostedWorldLatest {
            origin: "https://manifest.shivai.space".into(),
            hosted_world_id: "hosted-1".into(),
        });
        let access = WorldViewResolutionAccess::HostedEditShareFile {
            origin: "https://manifest.shivai.space".into(),
            hosted_world_id: "hosted-1".into(),
            credential_file: PathBuf::from("/private/edit-share.txt"),
        };

        let invocation = world_cli_invocation(&request.binding, &access).unwrap();

        assert!(invocation
            .args
            .windows(2)
            .any(|pair| { pair == ["--edit-share-file", "/private/edit-share.txt"] }));
        assert!(invocation
            .args
            .iter()
            .any(|argument| argument == "--anonymous-session"));
        assert_eq!(invocation.stdin, None);
        let envelope: WorldResultEnvelope<WorldViewDumpResult> =
            serde_json::from_slice(&success_stdout("source-revision-1")).unwrap();
        let result = envelope.result.unwrap();
        assert_eq!(
            authority_readback(&request.binding.reference, &result).unwrap(),
            (
                WorldViewResolutionAuthority::HostedWorldLatest {
                    origin: "https://manifest.shivai.space".into(),
                    hosted_world_id: "hosted-1".into(),
                },
                WorldViewResolutionFreshness::LatestAtResolution,
            )
        );
    }

    #[tokio::test]
    async fn rejects_an_untrusted_binding_origin_before_launching_world() {
        let untrusted_origin = "https://attacker.example";
        let error = resolve_world_view_with_binary(
            request(WorldViewReference::HostedWorldViewExport {
                origin: untrusted_origin.into(),
                share_token: "attacker-authored-token".into(),
            }),
            Path::new("/world-must-not-be-launched"),
            &WorldAuthorityRegistry::default(),
        )
        .await
        .expect_err("untrusted origin must fail before process launch");

        assert!(matches!(
            error,
            WorldViewResolutionError::UntrustedOrigin { origin }
                if origin == untrusted_origin
        ));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn writes_hosted_export_capability_to_the_child_stdin_pipe() {
        use std::os::unix::fs::PermissionsExt;

        let temp_root =
            std::env::temp_dir().join(format!("buzz-world-view-resolver-{}", Uuid::new_v4()));
        std::fs::create_dir(&temp_root).expect("create temp resolver root");
        let output_path = temp_root.join("world-output.json");
        std::fs::write(&output_path, success_stdout("source-revision-1"))
            .expect("write fake world output");
        let binary_path = temp_root.join("world");
        std::fs::write(
            &binary_path,
            format!(
                "#!/bin/sh\n\
                 token=$(cat)\n\
                 test \"$token\" = \"secret-view-token\" || exit 41\n\
                 cat '{}'\n",
                output_path.display()
            ),
        )
        .expect("write fake world binary");
        let mut permissions = std::fs::metadata(&binary_path)
            .expect("read fake world metadata")
            .permissions();
        permissions.set_mode(0o700);
        std::fs::set_permissions(&binary_path, permissions).expect("make fake world executable");

        let registry = WorldAuthorityRegistry::default();
        let resolved = resolve_world_view_with_binary(
            request(WorldViewReference::HostedWorldViewExport {
                origin: "https://manifest.shivai.space".into(),
                share_token: "secret-view-token".into(),
            }),
            &binary_path,
            &registry,
        )
        .await
        .expect("resolve through stdin-aware child");

        assert_eq!(resolved.source_revision, "source-revision-1");
        std::fs::remove_dir_all(temp_root).expect("remove temp resolver root");
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn decodes_canonical_nested_hosted_live_share_payload() {
        use std::os::unix::fs::PermissionsExt;

        let temp_root =
            std::env::temp_dir().join(format!("buzz-hosted-live-share-{}", Uuid::new_v4()));
        std::fs::create_dir(&temp_root).expect("create temp resolver root");
        let output_path = temp_root.join("world-output.json");
        let output = serde_json::to_vec(&json!({
            "ok": true,
            "result": {
                "baseUrl": "https://manifest.shivai.space",
                "command": "hosted view share-live",
                "liveViewShare": {
                    "liveViewShare": {
                        "id": "live-share-1",
                        "hostedWorldId": "hosted-1",
                        "shareToken": "public-live-share-token",
                        "shareUrlPath": "/world/live/public-live-share-token",
                        "title": "Focused scope",
                        "viewQualifiedName": "world::main::@Focused scope",
                        "viewLocalName": "Focused scope",
                        "realmQualifiedName": "world::main",
                        "referencedFlowspaceQualifiedNames": [],
                        "referencedSpaceQualifiedNames": [],
                        "createdAt": "2026-07-28T00:00:00.000Z"
                    },
                    "source": {
                        "hostedWorldId": "hosted-1",
                        "revisionId": "revision-id-1",
                        "packageRevision": "package-revision-1",
                        "manifestWorldQualifiedName": "world"
                    }
                },
                "revision": "package-revision-1",
                "target": { "kind": "edit-share" }
            },
            "diagnostics": []
        }))
        .expect("encode canonical live-share output");
        std::fs::write(&output_path, output).expect("write fake world output");
        let credential_file = temp_root.join("authority.edit-share");
        std::fs::write(&credential_file, "private-edit-share")
            .expect("write private edit-share fixture");
        let binary_path = temp_root.join("world");
        std::fs::write(
            &binary_path,
            format!("#!/bin/sh\ncat '{}'\n", output_path.display()),
        )
        .expect("write fake world binary");
        let mut permissions = std::fs::metadata(&binary_path)
            .expect("read fake world metadata")
            .permissions();
        permissions.set_mode(0o700);
        std::fs::set_permissions(&binary_path, permissions).expect("make fake world executable");

        let published = publish_hosted_live_view_share_with_binary(
            "https://manifest.shivai.space",
            &credential_file,
            "world::main::@Focused scope",
            &binary_path,
        )
        .await
        .expect("decode canonical hosted live-share payload");

        assert_eq!(
            published,
            PublishedHostedLiveViewShare {
                hosted_world_id: "hosted-1".into(),
                source_revision: "revision-id-1".into(),
                package_revision: "package-revision-1".into(),
                realm_qualified_name: "world::main".into(),
                view_qualified_name: "world::main::@Focused scope".into(),
                share_token: "public-live-share-token".into(),
                share_url_path: "/world/live/public-live-share-token".into(),
                title: "Focused scope".into(),
            }
        );
        std::fs::remove_dir_all(temp_root).expect("remove temp resolver root");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn resolves_a_connected_local_source_without_returning_its_root() {
        use std::os::unix::fs::PermissionsExt;

        let temp_root = std::env::temp_dir().join(format!("buzz-local-resolve-{}", Uuid::new_v4()));
        std::fs::create_dir(&temp_root).expect("create temp local resolver root");
        let source_root = temp_root.join("private.world");
        let output_path = temp_root.join("world-output.json");
        std::fs::write(&output_path, success_stdout("local-source-revision"))
            .expect("write fake local world output");
        let captured_args = temp_root.join("args.txt");
        let binary_path = temp_root.join("world");
        std::fs::write(
            &binary_path,
            format!(
                "#!/bin/sh\nprintf '%s\\n' \"$@\" > '{}'\ncat '{}'\n",
                captured_args.display(),
                output_path.display(),
            ),
        )
        .expect("write fake local world binary");
        let mut permissions = std::fs::metadata(&binary_path)
            .expect("read fake local world metadata")
            .permissions();
        permissions.set_mode(0o700);
        std::fs::set_permissions(&binary_path, permissions)
            .expect("make fake local world executable");
        let request = request(WorldViewReference::LocalWorldMirrorLatest {
            origin: "https://manifest.shivai.space".into(),
            mirror_id: "mirror-1".into(),
        });

        let mut registry = WorldAuthorityRegistry::default();
        registry
            .upsert_local(buzz_core::world_view::LocalWorldAuthority {
                origin: "https://manifest.shivai.space".into(),
                mirror_id: "mirror-1".into(),
                source_root: source_root.to_string_lossy().into_owned(),
                capability_secret_file: temp_root
                    .join("local-capability")
                    .to_string_lossy()
                    .into_owned(),
            })
            .unwrap();
        let resolved = resolve_world_view_with_binary(request, &binary_path, &registry)
            .await
            .expect("resolve connected local source");

        assert_eq!(resolved.source_revision, "local-source-revision");
        let args = std::fs::read_to_string(&captured_args).expect("read captured arguments");
        assert!(args.contains("--root"));
        assert!(args.contains(&source_root.to_string_lossy().to_string()));
        assert!(!args.contains("--local-mirror"));
        let encoded = serde_json::to_string(&resolved).expect("encode resolved local view");
        assert!(!encoded.contains(&source_root.to_string_lossy().to_string()));
        std::fs::remove_dir_all(temp_root).expect("remove temp local resolver root");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn applies_scoped_hosted_script_without_returning_credential_path() {
        use std::os::unix::fs::PermissionsExt;

        let temp_root = std::env::temp_dir().join(format!("buzz-hosted-script-{}", Uuid::new_v4()));
        std::fs::create_dir(&temp_root).expect("create temp hosted script root");
        let credential_file = temp_root.join("authority.edit-share");
        std::fs::write(&credential_file, "private-edit-share")
            .expect("write private authority fixture");
        let expected_revision = "a".repeat(64);
        let next_revision = "b".repeat(64);
        let latest_output = temp_root.join("latest.json");
        std::fs::write(
            &latest_output,
            serde_json::to_vec(&json!({
                "ok": true,
                "result": {
                    "projection": { "hostedWorldId": "hosted-1" },
                    "revision": expected_revision,
                }
            }))
            .expect("encode latest output"),
        )
        .expect("write latest output");
        let script_output = temp_root.join("script.json");
        std::fs::write(
            &script_output,
            serde_json::to_vec(&json!({
                "ok": true,
                "result": {
                    "command": "hosted script",
                    "credentialPathDiagnostic": credential_file,
                    "revision": next_revision,
                    "script": { "lineCount": 1 },
                }
            }))
            .expect("encode script output"),
        )
        .expect("write script output");
        let captured_stdin = temp_root.join("stdin.txt");
        let captured_args = temp_root.join("args.txt");
        let binary_path = temp_root.join("world");
        std::fs::write(
            &binary_path,
            format!(
                "#!/bin/sh\n\
                 if [ \"$1\" = hosted ] && [ \"$2\" = latest ]; then\n\
                   cat '{}'\n\
                   exit 0\n\
                 fi\n\
                 if [ \"$1\" = hosted ] && [ \"$2\" = script ]; then\n\
                   printf '%s\\n' \"$@\" > '{}'\n\
                   cat > '{}'\n\
                   cat '{}'\n\
                   exit 0\n\
                 fi\n\
                 exit 42\n",
                latest_output.display(),
                captured_args.display(),
                captured_stdin.display(),
                script_output.display(),
            ),
        )
        .expect("write fake world binary");
        let mut permissions = std::fs::metadata(&binary_path)
            .expect("read fake world metadata")
            .permissions();
        permissions.set_mode(0o700);
        std::fs::set_permissions(&binary_path, permissions).expect("make fake world executable");

        let script = "world add --disconnected \"Scoped triage\"";
        let result = apply_hosted_world_script_with_binary(
            "https://manifest.shivai.space",
            "hosted-1",
            &credential_file,
            &expected_revision,
            script,
            &binary_path,
        )
        .await
        .expect("apply hosted script");

        assert_eq!(
            result
                .pointer("/result/revision")
                .and_then(serde_json::Value::as_str),
            Some(next_revision.as_str())
        );
        assert_eq!(
            std::fs::read_to_string(&captured_stdin).expect("read captured script"),
            script
        );
        let args = std::fs::read_to_string(&captured_args).expect("read captured arguments");
        assert!(args.contains("--edit-share-file"));
        assert!(args.contains(&credential_file.to_string_lossy().to_string()));
        let encoded = serde_json::to_string(&result).expect("encode broker result");
        assert!(!encoded.contains(&credential_file.to_string_lossy().to_string()));
        assert!(encoded.contains("<redacted-credential-file>"));
        assert!(!encoded.contains("private-edit-share"));
        std::fs::remove_dir_all(temp_root).expect("remove temp hosted script root");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn stale_hosted_script_revision_never_launches_mutation() {
        use std::os::unix::fs::PermissionsExt;

        let temp_root = std::env::temp_dir().join(format!("buzz-hosted-stale-{}", Uuid::new_v4()));
        std::fs::create_dir(&temp_root).expect("create temp stale script root");
        let credential_file = temp_root.join("authority.edit-share");
        std::fs::write(&credential_file, "private-edit-share")
            .expect("write private authority fixture");
        let expected_revision = "a".repeat(64);
        let current_revision = "b".repeat(64);
        let latest_output = temp_root.join("latest.json");
        std::fs::write(
            &latest_output,
            serde_json::to_vec(&json!({
                "ok": true,
                "result": {
                    "projection": { "hostedWorldId": "hosted-1" },
                    "revision": current_revision,
                }
            }))
            .expect("encode latest output"),
        )
        .expect("write latest output");
        let mutation_marker = temp_root.join("mutation-ran");
        let binary_path = temp_root.join("world");
        std::fs::write(
            &binary_path,
            format!(
                "#!/bin/sh\n\
                 if [ \"$1\" = hosted ] && [ \"$2\" = latest ]; then\n\
                   cat '{}'\n\
                   exit 0\n\
                 fi\n\
                 if [ \"$1\" = hosted ] && [ \"$2\" = script ]; then\n\
                   touch '{}'\n\
                   exit 0\n\
                 fi\n\
                 exit 42\n",
                latest_output.display(),
                mutation_marker.display(),
            ),
        )
        .expect("write fake world binary");
        let mut permissions = std::fs::metadata(&binary_path)
            .expect("read fake world metadata")
            .permissions();
        permissions.set_mode(0o700);
        std::fs::set_permissions(&binary_path, permissions).expect("make fake world executable");

        let error = apply_hosted_world_script_with_binary(
            "https://manifest.shivai.space",
            "hosted-1",
            &credential_file,
            &expected_revision,
            "world add --disconnected \"Stale triage\"",
            &binary_path,
        )
        .await
        .expect_err("stale revision must fail");

        assert!(matches!(
            error,
            WorldViewResolutionError::RevisionConflict(_)
        ));
        assert!(error.to_string().contains(&current_revision));
        assert!(!mutation_marker.exists());
        std::fs::remove_dir_all(temp_root).expect("remove temp stale script root");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn applies_scoped_local_script_without_returning_source_root() {
        use std::os::unix::fs::PermissionsExt;

        let temp_root = std::env::temp_dir().join(format!("buzz-local-script-{}", Uuid::new_v4()));
        std::fs::create_dir(&temp_root).expect("create temp local script root");
        let source_root = temp_root.join("private.world");
        let expected_revision = "a".repeat(64);
        let next_revision = "b".repeat(64);
        let script_output = temp_root.join("script.json");
        std::fs::write(
            &script_output,
            serde_json::to_vec(&json!({
                "ok": true,
                "result": {
                    "command": "script",
                    "sourceRootDiagnostic": source_root,
                    "revision": next_revision,
                    "script": { "lineCount": 1 },
                }
            }))
            .expect("encode local script output"),
        )
        .expect("write local script output");
        let captured_stdin = temp_root.join("stdin.txt");
        let captured_args = temp_root.join("args.txt");
        let binary_path = temp_root.join("world");
        std::fs::write(
            &binary_path,
            format!(
                "#!/bin/sh\n\
                 printf '%s\\n' \"$@\" > '{}'\n\
                 cat > '{}'\n\
                 cat '{}'\n",
                captured_args.display(),
                captured_stdin.display(),
                script_output.display(),
            ),
        )
        .expect("write fake local world binary");
        let mut permissions = std::fs::metadata(&binary_path)
            .expect("read fake local world metadata")
            .permissions();
        permissions.set_mode(0o700);
        std::fs::set_permissions(&binary_path, permissions)
            .expect("make fake local world executable");

        let script = "world add --disconnected \"Scoped local triage\"";
        let result = apply_local_world_script_with_binary(
            "https://manifest.shivai.space",
            "mirror-1",
            &source_root,
            &expected_revision,
            script,
            &binary_path,
        )
        .await
        .expect("apply local script");

        assert_eq!(
            result
                .pointer("/result/revision")
                .and_then(serde_json::Value::as_str),
            Some(next_revision.as_str())
        );
        assert_eq!(
            std::fs::read_to_string(&captured_stdin).expect("read captured local script"),
            script
        );
        let args = std::fs::read_to_string(&captured_args).expect("read captured local arguments");
        assert!(args.contains("--root"));
        assert!(args.contains(&source_root.to_string_lossy().to_string()));
        assert!(args.contains("--expected-revision"));
        assert!(args.contains(&expected_revision));
        let encoded = serde_json::to_string(&result).expect("encode local broker result");
        assert!(!encoded.contains(&source_root.to_string_lossy().to_string()));
        assert!(encoded.contains("<redacted-local-world-root>"));
        std::fs::remove_dir_all(temp_root).expect("remove temp local script root");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn stale_local_script_revision_returns_redacted_conflict_without_mutation() {
        use std::os::unix::fs::PermissionsExt;

        let temp_root = std::env::temp_dir().join(format!("buzz-local-stale-{}", Uuid::new_v4()));
        std::fs::create_dir(&temp_root).expect("create temp local stale root");
        let source_root = temp_root.join("private.world");
        let expected_revision = "a".repeat(64);
        let captured_args = temp_root.join("args.txt");
        let mutation_marker = temp_root.join("mutation-ran");
        let binary_path = temp_root.join("world");
        std::fs::write(
            &binary_path,
            format!(
                "#!/bin/sh\n\
                 printf '%s\\n' \"$@\" > '{}'\n\
                 echo 'Error (world.workflow.revision_conflict): stale {}' >&2\n\
                 exit 1\n\
                 touch '{}'\n",
                captured_args.display(),
                source_root.display(),
                mutation_marker.display(),
            ),
        )
        .expect("write fake stale local world binary");
        let mut permissions = std::fs::metadata(&binary_path)
            .expect("read fake stale local world metadata")
            .permissions();
        permissions.set_mode(0o700);
        std::fs::set_permissions(&binary_path, permissions)
            .expect("make fake stale local world executable");

        let error = apply_local_world_script_with_binary(
            "https://manifest.shivai.space",
            "mirror-1",
            &source_root,
            &expected_revision,
            "world add --disconnected \"Stale local triage\"",
            &binary_path,
        )
        .await
        .expect_err("stale local revision must fail");

        assert!(matches!(
            error,
            WorldViewResolutionError::RevisionConflict(_)
        ));
        let diagnostics = error.to_string();
        assert!(!diagnostics.contains(&source_root.to_string_lossy().to_string()));
        assert!(diagnostics.contains("<redacted-local-world-root>"));
        let args = std::fs::read_to_string(&captured_args).expect("read stale local arguments");
        assert!(args.contains("--expected-revision"));
        assert!(args.contains(&expected_revision));
        assert!(!mutation_marker.exists());
        std::fs::remove_dir_all(temp_root).expect("remove temp local stale root");
    }
}
