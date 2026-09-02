mod ipc;
mod manifest;
mod path_security;
mod protocol;
mod storage;
mod template;

#[cfg(test)]
mod tests;

use std::{
    collections::{BTreeSet, HashMap, VecDeque},
    path::PathBuf,
    sync::{Arc, Mutex},
};

use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, State};
use tauri_plugin_opener::OpenerExt;

use manifest::ValidatedManifest;
use storage::{
    active_snapshot, clear_committed_updates, commit_snapshot, pending_updates, prepare_snapshot,
    project_source_location, prune_revisions, record_pending_update, record_source_binding,
    snapshot_for_revision, validate_widget_id, ProjectBinding, ProjectCanvasSourceLocation,
    ValidatedPackage,
};
use template::bundled_template;

const MAX_ACTIVE_LOADS: usize = 64;
/// Avatars the host may keep published for one project. A people lookup
/// returns at most 32 rows, so this holds two full lookups before the oldest
/// face falls back to initials.
const MAX_PUBLISHED_AVATARS: usize = 64;
/// Per-avatar byte ceiling. The host re-encodes to a small square before
/// publishing — a 96px WebP lands near 2 KiB — so this exists only to keep a
/// mistake from becoming a memory problem.
const MAX_PUBLISHED_AVATAR_BYTES: usize = 32 * 1024;
/// Combined byte ceiling for one project, evicted oldest-first. Holds even if
/// every avatar arrives at the per-avatar maximum.
const MAX_PUBLISHED_AVATAR_BYTES_PER_PROJECT: usize = 512 * 1024;
/// Projects that may hold published avatars, evicted least-recently-published.
const MAX_AVATAR_PROJECTS: usize = 4;

#[derive(Clone)]
pub(crate) struct ProjectCanvasRuntime {
    root: Option<PathBuf>,
    loads: Arc<Mutex<HashMap<String, ActiveLoad>>>,
    /// Avatar bytes the host has published, keyed by project then pubkey.
    ///
    /// Sandboxed frames run with `connect-src 'none'` and cannot fetch
    /// anything themselves, so avatars used to ride inside the RPC payload as
    /// base64 — which put every face in a people lookup under one 64 KiB
    /// message ceiling. The host instead fetches an avatar on its own audited
    /// webview path and hands the bytes here; the frame then loads
    /// `./__buzz/avatar/<pubkey>` like any ordinary image, outside any
    /// message. Nothing in this process fetches, so a hostile `picture` URL in
    /// a kind:0 profile never becomes a backend request.
    ///
    /// Deliberately *not* tied to load lifetime. Publishing and frame creation
    /// race in both directions, and a frame that requests an avatar before its
    /// bytes land gets a 404 it will never retry. Keying by project instead
    /// makes the order irrelevant; the ceilings above are what bound the store.
    avatars: Arc<Mutex<AvatarRegistry>>,
    activation_lock: Arc<Mutex<()>>,
}

impl Default for ProjectCanvasRuntime {
    fn default() -> Self {
        Self {
            // Resolve the nest lazily: setup selects `.buzz` or `.buzz-dev`
            // after managed state is constructed.
            root: None,
            loads: Arc::new(Mutex::new(HashMap::new())),
            avatars: Arc::new(Mutex::new(AvatarRegistry::default())),
            activation_lock: Arc::new(Mutex::new(())),
        }
    }
}

impl ProjectCanvasRuntime {
    #[cfg(test)]
    fn with_root(root: PathBuf) -> Self {
        Self {
            root: Some(root),
            loads: Arc::new(Mutex::new(HashMap::new())),
            avatars: Arc::new(Mutex::new(AvatarRegistry::default())),
            activation_lock: Arc::new(Mutex::new(())),
        }
    }

    fn root(&self) -> Result<PathBuf, String> {
        self.root
            .clone()
            .or_else(|| crate::managed_agents::nest_dir().map(|root| root.join("CANVASES")))
            .ok_or_else(|| "cannot resolve the nest directory for project canvases".to_string())
    }

    fn get_or_activate(
        &self,
        request: ProjectCanvasPackageRequest,
        template: Option<&ValidatedPackage>,
    ) -> Result<ProjectCanvasPackageDescriptor, String> {
        let binding = ProjectBinding::parse(request)?;
        ensure_supported_platform()?;
        let _guard = self
            .activation_lock
            .lock()
            .map_err(|_| "project canvas activation lock is unavailable".to_string())?;

        let root = self.root()?;
        let snapshot = match active_snapshot(&root, &binding)? {
            Some(snapshot) => snapshot,
            None => prepare_snapshot(&root, &binding, template)?,
        };
        let mut retained = self.referenced_revisions(&binding)?;
        retained.insert(snapshot.revision.clone());
        prune_revisions(&root, &binding, &retained)?;
        // The index is agent-facing discovery metadata, not runtime authority. A
        // malformed or manually edited index must not block a validated package.
        let _ = record_source_binding(&root, &binding);
        self.issue_load(binding, snapshot)
    }

    fn activate(
        &self,
        request: ProjectCanvasPackageRequest,
        template: Option<&ValidatedPackage>,
    ) -> Result<ProjectCanvasPackageDescriptor, String> {
        let binding = ProjectBinding::parse(request)?;
        ensure_supported_platform()?;
        let _guard = self
            .activation_lock
            .lock()
            .map_err(|_| "project canvas activation lock is unavailable".to_string())?;
        let root = self.root()?;
        let snapshot = prepare_snapshot(&root, &binding, template)?;
        let mut retained = self.referenced_revisions(&binding)?;
        retained.insert(snapshot.revision.clone());
        prune_revisions(&root, &binding, &retained)?;
        let _ = record_source_binding(&root, &binding);
        self.issue_load(binding, snapshot)
    }

    fn commit(&self, load_id: &str) -> Result<(), String> {
        let load = self
            .load(load_id)?
            .ok_or_else(|| "project canvas load not found".to_string())?;
        let _guard = self
            .activation_lock
            .lock()
            .map_err(|_| "project canvas activation lock is unavailable".to_string())?;
        let root = self.root()?;
        commit_snapshot(&root, &load.binding, &load.revision)?;
        clear_committed_updates(&root, &load.binding, &load.revision)?;
        let retained = self.referenced_revisions(&load.binding)?;
        prune_revisions(&root, &load.binding, &retained)
    }

    fn accept_agent_update(
        &self,
        request: ProjectCanvasAgentUpdateRequest,
    ) -> Result<ProjectCanvasUpdateAccepted, String> {
        request.validate()?;
        let binding = ProjectBinding::parse(ProjectCanvasPackageRequest {
            community_id: request.community_id.clone(),
            project_id: request.project_id.clone(),
        })?;
        ensure_supported_platform()?;
        let _guard = self
            .activation_lock
            .lock()
            .map_err(|_| "project canvas activation lock is unavailable".to_string())?;
        let root = self.root()?;
        let snapshot = prepare_snapshot(&root, &binding, None)?;
        validate_widget_in_data(&snapshot.data, &request.widget_id)?;
        record_pending_update(
            &root,
            &binding,
            request.change,
            &request.notification_id,
            &request.widget_id,
            &snapshot.revision,
        )?;
        let retained = self.referenced_revisions(&binding)?;
        prune_revisions(&root, &binding, &retained)?;
        Ok(ProjectCanvasUpdateAccepted {
            change: request.change,
            community_id: request.community_id,
            notification_id: request.notification_id,
            project_id: request.project_id,
            revision: snapshot.revision,
            widget_id: request.widget_id,
        })
    }

    fn updates(
        &self,
        request: ProjectCanvasPackageRequest,
    ) -> Result<ProjectCanvasPendingUpdates, String> {
        let binding = ProjectBinding::parse(request)?;
        ensure_supported_platform()?;
        let _guard = self
            .activation_lock
            .lock()
            .map_err(|_| "project canvas activation lock is unavailable".to_string())?;
        let root = self.root()?;
        let updates = pending_updates(&root, &binding)?;
        let presentation = match updates.presentation {
            Some(update) => {
                let snapshot = snapshot_for_revision(&root, &binding, &update.revision)?;
                Some(ProjectCanvasPendingPresentation {
                    notification_id: update.notification_id,
                    package: self.issue_load(binding.clone(), snapshot)?,
                    widget_id: update.widget_id,
                })
            }
            None => None,
        };
        let data = match updates.data {
            Some(update) => {
                let snapshot = snapshot_for_revision(&root, &binding, &update.revision)?;
                Some(ProjectCanvasPendingData {
                    data: snapshot.data,
                    notification_id: update.notification_id,
                    revision: update.revision,
                    widget_id: update.widget_id,
                })
            }
            None => None,
        };
        Ok(ProjectCanvasPendingUpdates { data, presentation })
    }

    fn source_location(
        &self,
        request: ProjectCanvasPackageRequest,
    ) -> Result<ProjectCanvasSourceLocation, String> {
        let binding = ProjectBinding::parse(request)?;
        ensure_supported_platform()?;
        let _guard = self
            .activation_lock
            .lock()
            .map_err(|_| "project canvas activation lock is unavailable".to_string())?;
        let root = self.root()?;
        let location = project_source_location(&root, &binding)?;
        let _ = record_source_binding(&root, &binding);
        Ok(location)
    }

    fn issue_load(
        &self,
        binding: ProjectBinding,
        snapshot: storage::ValidatedSnapshot,
    ) -> Result<ProjectCanvasPackageDescriptor, String> {
        let load_id = uuid::Uuid::new_v4().simple().to_string();
        let nonce = uuid::Uuid::new_v4().simple().to_string();
        let manifest = snapshot.manifest.clone();
        let data = snapshot.data.clone();
        let revision = snapshot.revision.clone();
        let scope = binding.scope();
        let load = ActiveLoad {
            binding,
            files: snapshot.files,
            nonce: nonce.clone(),
            scope,
            granted_capabilities: manifest.capabilities.clone(),
            manifest,
            revision: revision.clone(),
        };

        let mut loads = self
            .loads
            .lock()
            .map_err(|_| "project canvas load registry is unavailable".to_string())?;
        if loads.len() >= MAX_ACTIVE_LOADS {
            if let Some(oldest) = loads.keys().next().cloned() {
                loads.remove(&oldest);
            }
        }
        loads.insert(load_id.clone(), load);

        Ok(ProjectCanvasPackageDescriptor {
            url: protocol_url(&load_id),
            load_id,
            revision,
            nonce,
            capabilities: snapshot.manifest.capabilities,
            data,
        })
    }

    fn load(&self, load_id: &str) -> Result<Option<ActiveLoad>, String> {
        let loads = self
            .loads
            .lock()
            .map_err(|_| "project canvas load registry is unavailable".to_string())?;
        let load = loads.get(load_id).cloned();
        if let Some(load) = &load {
            if !load.scope.is_valid() || load.granted_capabilities != load.manifest.capabilities {
                return Err("project canvas load binding is invalid".to_string());
            }
        }
        Ok(load)
    }

    /// Publishes avatar bytes that frames bound to `request`'s project may load
    /// from `__buzz/avatar/<pubkey>`.
    ///
    /// Every entry is validated before the lock is taken, so a malformed batch
    /// leaves the store exactly as it was rather than half-applied.
    fn publish_avatars(
        &self,
        request: ProjectCanvasPackageRequest,
        avatars: Vec<ProjectCanvasAvatarInput>,
    ) -> Result<(), String> {
        let binding = ProjectBinding::parse(request)?;
        ensure_supported_platform()?;
        if avatars.len() > MAX_PUBLISHED_AVATARS {
            return Err(format!(
                "at most {MAX_PUBLISHED_AVATARS} project canvas avatars may be published at once"
            ));
        }
        let decoded = avatars
            .into_iter()
            .map(ProjectCanvasAvatarInput::validate)
            .collect::<Result<Vec<_>, _>>()?;
        let mut registry = self
            .avatars
            .lock()
            .map_err(|_| "project canvas avatar registry is unavailable".to_string())?;
        registry.publish(&binding.cache_key(), decoded);
        Ok(())
    }

    fn avatar(
        &self,
        binding: &ProjectBinding,
        pubkey: &str,
    ) -> Result<Option<CanvasAvatar>, String> {
        let registry = self
            .avatars
            .lock()
            .map_err(|_| "project canvas avatar registry is unavailable".to_string())?;
        Ok(registry.get(&binding.cache_key(), pubkey))
    }

    fn referenced_revisions(&self, binding: &ProjectBinding) -> Result<BTreeSet<String>, String> {
        let loads = self
            .loads
            .lock()
            .map_err(|_| "project canvas load registry is unavailable".to_string())?;
        Ok(loads
            .values()
            .filter(|load| load.binding.matches(binding))
            .map(|load| load.revision.clone())
            .collect())
    }

    fn release(&self, load_id: &str) -> Result<(), String> {
        let parsed = uuid::Uuid::parse_str(load_id)
            .map_err(|_| "invalid project canvas load id".to_string())?;
        let key = parsed.simple().to_string();
        let mut loads = self
            .loads
            .lock()
            .map_err(|_| "project canvas load registry is unavailable".to_string())?;
        loads.remove(&key);
        Ok(())
    }
}

#[derive(Clone)]
struct ActiveLoad {
    binding: ProjectBinding,
    files: Arc<std::collections::BTreeMap<String, Vec<u8>>>,
    nonce: String,
    scope: storage::CanvasScope,
    granted_capabilities: Vec<String>,
    manifest: ValidatedManifest,
    revision: String,
}

/// An avatar published for one pubkey, ready to serve verbatim.
#[derive(Clone)]
pub(super) struct CanvasAvatar {
    /// Always one of the allowlisted image types, never a caller-supplied
    /// string — so the frame cannot be handed a content type of its choosing.
    pub(super) content_type: &'static str,
    pub(super) bytes: Arc<Vec<u8>>,
}

/// Published avatars for every project, bounded by project count.
#[derive(Default)]
struct AvatarRegistry {
    projects: HashMap<String, AvatarCache>,
    order: VecDeque<String>,
}

impl AvatarRegistry {
    fn publish(&mut self, project: &str, avatars: Vec<(String, CanvasAvatar)>) {
        if !self.projects.contains_key(project) {
            self.projects
                .insert(project.to_string(), AvatarCache::default());
            self.order.push_back(project.to_string());
            while self.order.len() > MAX_AVATAR_PROJECTS {
                let Some(evicted) = self.order.pop_front() else {
                    break;
                };
                self.projects.remove(&evicted);
            }
        }
        let Some(cache) = self.projects.get_mut(project) else {
            return;
        };
        for (pubkey, avatar) in avatars {
            cache.insert(pubkey, avatar);
        }
    }

    fn get(&self, project: &str, pubkey: &str) -> Option<CanvasAvatar> {
        self.projects.get(project)?.entries.get(pubkey).cloned()
    }
}

/// One project's published avatars, bounded by both count and total bytes and
/// evicted oldest-first.
#[derive(Default)]
struct AvatarCache {
    entries: HashMap<String, CanvasAvatar>,
    order: VecDeque<String>,
    bytes: usize,
}

impl AvatarCache {
    fn insert(&mut self, pubkey: String, avatar: CanvasAvatar) {
        let added = avatar.bytes.len();
        match self.entries.insert(pubkey.clone(), avatar) {
            Some(replaced) => self.bytes = self.bytes.saturating_sub(replaced.bytes.len()),
            None => self.order.push_back(pubkey),
        }
        self.bytes = self.bytes.saturating_add(added);
        while self.order.len() > MAX_PUBLISHED_AVATARS
            || self.bytes > MAX_PUBLISHED_AVATAR_BYTES_PER_PROJECT
        {
            let Some(evicted) = self.order.pop_front() else {
                break;
            };
            if let Some(removed) = self.entries.remove(&evicted) {
                self.bytes = self.bytes.saturating_sub(removed.bytes.len());
            }
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ProjectCanvasAvatarInput {
    pubkey: String,
    content_type: String,
    /// Standard base64 of the image bytes. Sent as text because the Tauri IPC
    /// encodes a `Vec<u8>` as a JSON number array — several times worse than
    /// the base64 it would be replacing.
    data: String,
}

impl ProjectCanvasAvatarInput {
    fn validate(self) -> Result<(String, CanvasAvatar), String> {
        let pubkey = normalized_pubkey(&self.pubkey)?;
        let content_type = image_content_type(&self.content_type).ok_or_else(|| {
            format!(
                "unsupported project canvas avatar type '{}'",
                self.content_type
            )
        })?;
        // Check the encoded length first: decoding is what would allocate.
        if self.data.len() > MAX_PUBLISHED_AVATAR_BYTES.div_ceil(3) * 4 + 4 {
            return Err("project canvas avatar is too large".to_string());
        }
        let bytes = BASE64
            .decode(self.data.as_bytes())
            .map_err(|_| "project canvas avatar data must be base64".to_string())?;
        if bytes.is_empty() || bytes.len() > MAX_PUBLISHED_AVATAR_BYTES {
            return Err("project canvas avatar is too large".to_string());
        }
        // `nosniff` already stops the webview reinterpreting these bytes, but
        // this is the sandbox boundary and the frame is untrusted: bytes that
        // do not open with their declared type's signature are not an image.
        if !image_bytes_match(content_type, &bytes) {
            return Err(format!(
                "project canvas avatar bytes are not {content_type} data"
            ));
        }
        Ok((
            pubkey,
            CanvasAvatar {
                content_type,
                bytes: Arc::new(bytes),
            },
        ))
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ProjectCanvasPackageRequest {
    community_id: String,
    project_id: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProjectCanvasPackageDescriptor {
    load_id: String,
    url: String,
    revision: String,
    nonce: String,
    capabilities: Vec<String>,
    data: serde_json::Value,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum ProjectCanvasUpdateChange {
    Presentation,
    Data,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ProjectCanvasAgentUpdateRequest {
    format: String,
    version: u32,
    notification_id: String,
    community_id: String,
    project_id: String,
    widget_id: String,
    change: ProjectCanvasUpdateChange,
}

impl ProjectCanvasAgentUpdateRequest {
    fn validate(&self) -> Result<(), String> {
        if self.format != ipc::UPDATE_FORMAT || self.version != ipc::UPDATE_VERSION {
            return Err("unsupported project canvas update request".to_string());
        }
        let parsed = uuid::Uuid::parse_str(&self.notification_id)
            .map_err(|_| "invalid project canvas update notification id".to_string())?;
        if parsed.simple().to_string() != self.notification_id {
            return Err("invalid project canvas update notification id".to_string());
        }
        validate_widget_id(&self.widget_id)
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProjectCanvasUpdateAccepted {
    change: ProjectCanvasUpdateChange,
    community_id: String,
    notification_id: String,
    project_id: String,
    revision: String,
    widget_id: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProjectCanvasPendingUpdates {
    data: Option<ProjectCanvasPendingData>,
    presentation: Option<ProjectCanvasPendingPresentation>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProjectCanvasPendingData {
    data: serde_json::Value,
    notification_id: String,
    revision: String,
    widget_id: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProjectCanvasPendingPresentation {
    notification_id: String,
    package: ProjectCanvasPackageDescriptor,
    widget_id: String,
}

#[tauri::command]
pub(crate) async fn get_project_canvas_package(
    request: ProjectCanvasPackageRequest,
    runtime: State<'_, ProjectCanvasRuntime>,
) -> Result<ProjectCanvasPackageDescriptor, String> {
    let runtime = runtime.inner().clone();
    run_blocking(move || {
        let template = bundled_template()?;
        runtime.get_or_activate(request, Some(&template))
    })
    .await
}

#[tauri::command]
pub(crate) async fn get_project_canvas_updates(
    request: ProjectCanvasPackageRequest,
    runtime: State<'_, ProjectCanvasRuntime>,
) -> Result<ProjectCanvasPendingUpdates, String> {
    let runtime = runtime.inner().clone();
    run_blocking(move || runtime.updates(request)).await
}

#[tauri::command]
pub(crate) async fn activate_project_canvas_package(
    request: ProjectCanvasPackageRequest,
    runtime: State<'_, ProjectCanvasRuntime>,
) -> Result<ProjectCanvasPackageDescriptor, String> {
    let runtime = runtime.inner().clone();
    run_blocking(move || {
        let template = bundled_template()?;
        runtime.activate(request, Some(&template))
    })
    .await
}

/// Publishes avatar bytes for a project's canvas frames to load by URL.
///
/// Synchronous on purpose: it only decodes and stores, adding no IO, which is
/// what lets the protocol handler that reads it stay synchronous too.
#[tauri::command]
pub(crate) fn publish_project_canvas_avatars(
    request: ProjectCanvasPackageRequest,
    avatars: Vec<ProjectCanvasAvatarInput>,
    runtime: State<'_, ProjectCanvasRuntime>,
) -> Result<(), String> {
    runtime.publish_avatars(request, avatars)
}

#[tauri::command]
pub(crate) fn release_project_canvas_package(
    load_id: String,
    runtime: State<'_, ProjectCanvasRuntime>,
) -> Result<(), String> {
    runtime.release(&load_id)
}

#[tauri::command]
pub(crate) async fn commit_project_canvas_package(
    load_id: String,
    runtime: State<'_, ProjectCanvasRuntime>,
) -> Result<(), String> {
    let runtime = runtime.inner().clone();
    run_blocking(move || runtime.commit(&load_id)).await
}

#[tauri::command]
pub(crate) async fn open_project_canvas_source(
    request: ProjectCanvasPackageRequest,
    app: AppHandle,
    runtime: State<'_, ProjectCanvasRuntime>,
) -> Result<(), String> {
    let runtime = runtime.inner().clone();
    let location = run_blocking(move || runtime.source_location(request)).await?;
    app.opener()
        .open_path(&location.source_path, None::<&str>)
        .map_err(|error| format!("open project canvas source: {error}"))
}

#[tauri::command]
pub(crate) async fn get_project_canvas_source(
    request: ProjectCanvasPackageRequest,
    runtime: State<'_, ProjectCanvasRuntime>,
) -> Result<ProjectCanvasSourceLocation, String> {
    let runtime = runtime.inner().clone();
    run_blocking(move || runtime.source_location(request)).await
}

pub(crate) fn handle_request(
    app: &AppHandle,
    request: &tauri::http::Request<Vec<u8>>,
) -> tauri::http::Response<Vec<u8>> {
    let runtime = app.state::<ProjectCanvasRuntime>();
    protocol::handle(&runtime, request)
}

pub(crate) fn start_agent_update_listener(app: AppHandle) -> Result<(), String> {
    ipc::start(app)
}

async fn run_blocking<T, F>(task: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, String> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(task)
        .await
        .map_err(|error| format!("project canvas task failed: {error}"))?
}

fn protocol_url(load_id: &str) -> String {
    if cfg!(target_os = "windows") {
        format!("http://buzz-canvas.localhost/{load_id}/")
    } else {
        format!("buzz-canvas://localhost/{load_id}/")
    }
}

pub(super) fn normalized_pubkey(value: &str) -> Result<String, String> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("project canvas avatar pubkey must be 64 hex characters".to_string());
    }
    Ok(value.to_ascii_lowercase())
}

/// Maps a caller-supplied media type onto the fixed set a canvas may serve,
/// discarding any parameters. Returning `'static` is what keeps a
/// caller-controlled string out of the response headers.
fn image_content_type(value: &str) -> Option<&'static str> {
    match value
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "image/png" => Some("image/png"),
        "image/jpeg" => Some("image/jpeg"),
        "image/webp" => Some("image/webp"),
        "image/gif" => Some("image/gif"),
        _ => None,
    }
}

fn image_bytes_match(content_type: &str, bytes: &[u8]) -> bool {
    match content_type {
        "image/png" => bytes.starts_with(b"\x89PNG\r\n\x1a\n"),
        "image/jpeg" => bytes.starts_with(b"\xff\xd8\xff"),
        "image/gif" => bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a"),
        "image/webp" => bytes.len() > 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP",
        _ => false,
    }
}

fn ensure_supported_platform() -> Result<(), String> {
    if !cfg!(target_os = "macos") {
        return Err(
            "sandboxed project canvases are macOS-only until iframe IPC isolation is proven on this platform"
                .to_string(),
        );
    }
    Ok(())
}

fn validate_widget_in_data(data: &serde_json::Value, widget_id: &str) -> Result<(), String> {
    let dashboards = data
        .get("dashboards")
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| "project canvas data must contain a dashboards object".to_string())?;
    let matches = dashboards
        .values()
        .filter_map(|dashboard| dashboard.get("widgets"))
        .filter_map(serde_json::Value::as_array)
        .flatten()
        .filter(|widget| widget.get("id").and_then(serde_json::Value::as_str) == Some(widget_id))
        .count();
    match matches {
        1 => Ok(()),
        0 => Err(format!(
            "widget id '{widget_id}' does not exist in the Canvas data"
        )),
        _ => Err(format!(
            "widget id '{widget_id}' must be unique across Canvas dashboards"
        )),
    }
}

/// Decides whether the main webview may commit a top-level navigation.
///
/// `dev_url` is the dev server origin the app was actually built against
/// (`webview.config().build.dev_url`). Every `just` desktop recipe derives a
/// per-worktree Vite port via `scripts/instance-env.sh`, so the origin cannot
/// be hardcoded — and because the dev server load *is* the initial navigation,
/// cancelling it leaves a blank window rather than a blocked link.
pub(crate) fn allow_webview_navigation(url: &tauri::Url, dev_url: Option<&tauri::Url>) -> bool {
    match url.scheme() {
        "about" => url.as_str() == "about:blank",
        "buzz-canvas" => url.host_str() == Some("localhost"),
        "tauri" => url.host_str() == Some("localhost"),
        // Debug builds serve the frontend from a dev server; release builds
        // have none, so plain http stays blocked there.
        "http" if cfg!(debug_assertions) => {
            dev_url.is_some_and(|dev_url| dev_url.origin() == url.origin())
        }
        _ => false,
    }
}
