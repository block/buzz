use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, OpenOptions},
    io::{ErrorKind, Write},
    path::{Path, PathBuf},
    sync::Arc,
};

use atomic_write_file::AtomicWriteFile;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{
    manifest::{
        is_text, mime_type, validate_manifest, ValidatedManifest, MAX_FILE_BYTES,
        MAX_PACKAGE_BYTES, MAX_PACKAGE_FILES, MAX_TEXT_BYTES,
    },
    path_security::{
        canonical_canvas_root, ensure_no_symlink, ensure_secure_descendant, ensure_secure_file,
        make_snapshot_read_only, make_tree_writable, read_file_with_cap, read_package_tree,
    },
    ProjectCanvasPackageRequest,
};

const RUNTIME_ROOT_DIR: &str = ".runtime";
const REVISIONS_DIR: &str = "revisions";
const ACTIVE_FILE: &str = "active.json";
const UPDATES_FILE: &str = "updates.json";
const INDEX_FILE: &str = "index.json";
const INDEX_FORMAT: &str = "buzz-project-canvas-index";
const INDEX_VERSION: u32 = 1;
const MAX_INDEX_BYTES: usize = 1024 * 1024;
const MAX_INDEX_ENTRIES: usize = 4_096;
const RECENT_REVISION_RETENTION: usize = 2;
const UPDATE_STATE_VERSION: u32 = 1;
const MAX_UPDATE_STATE_BYTES: usize = 16 * 1024;

#[derive(Clone)]
pub(super) struct ProjectBinding {
    community_id: String,
    community_key: String,
    owner: String,
    project_key: String,
    project_id: String,
}

#[derive(Clone)]
pub(super) struct CanvasScope {
    community_key: String,
    project_id: String,
}

impl CanvasScope {
    pub(super) fn is_valid(&self) -> bool {
        !self.community_key.is_empty() && !self.project_id.is_empty()
    }
}

impl ProjectBinding {
    pub(super) fn parse(request: ProjectCanvasPackageRequest) -> Result<Self, String> {
        validate_scope_value("community id", &request.community_id, 128)?;

        let mut coordinate = request.project_id.splitn(3, ':');
        let kind = coordinate.next();
        let owner = coordinate.next();
        let dtag = coordinate.next();
        let (Some("30621"), Some(owner), Some(dtag)) = (kind, owner, dtag) else {
            return Err("project id must be a 30621:<owner>:<dtag> coordinate".to_string());
        };
        if owner.len() != 64 || !owner.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err("project id owner must be a 64-character hex public key".to_string());
        }
        validate_scope_value("project d tag", dtag, 512)?;

        let owner = owner.to_ascii_lowercase();
        Ok(Self {
            community_id: request.community_id.clone(),
            community_key: scope_hash(&request.community_id),
            owner: owner.clone(),
            project_key: scope_hash(dtag),
            project_id: format!("30621:{owner}:{dtag}"),
        })
    }

    pub(super) fn scope(&self) -> CanvasScope {
        CanvasScope {
            community_key: self.community_key.clone(),
            project_id: self.project_id.clone(),
        }
    }

    pub(super) fn matches(&self, other: &Self) -> bool {
        self.community_key == other.community_key
            && self.owner == other.owner
            && self.project_key == other.project_key
    }

    /// Key for in-memory state scoped to this project, agreeing exactly with
    /// [`Self::matches`]. Every component is a hash or hex, so the key carries
    /// no raw community id or `d` tag.
    pub(super) fn cache_key(&self) -> String {
        format!("{}/{}/{}", self.community_key, self.owner, self.project_key)
    }

    fn project_root(&self, canvas_root: &Path) -> PathBuf {
        canvas_root
            .join(&self.community_key)
            .join(&self.owner)
            .join(&self.project_key)
    }

    fn runtime_root(&self, canvas_root: &Path) -> PathBuf {
        canvas_root
            .join(RUNTIME_ROOT_DIR)
            .join(&self.community_key)
            .join(&self.owner)
            .join(&self.project_key)
    }

    #[cfg(test)]
    pub(super) fn project_root_for_test(&self, canvas_root: &Path) -> PathBuf {
        self.project_root(canvas_root)
    }

    #[cfg(test)]
    pub(super) fn runtime_root_for_test(&self, canvas_root: &Path) -> PathBuf {
        self.runtime_root(canvas_root)
    }
}

#[derive(Debug)]
pub(super) struct ValidatedSnapshot {
    pub(super) files: Arc<BTreeMap<String, Vec<u8>>>,
    pub(super) revision: String,
    pub(super) manifest: ValidatedManifest,
    pub(super) data: serde_json::Value,
}

pub(super) struct ValidatedPackage {
    files: BTreeMap<String, Vec<u8>>,
    revision: String,
    manifest: ValidatedManifest,
    data: serde_json::Value,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ActiveRevision {
    revision: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct PendingCanvasUpdate {
    pub(super) notification_id: String,
    pub(super) revision: String,
    pub(super) widget_id: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct PendingCanvasUpdates {
    version: u32,
    pub(super) presentation: Option<PendingCanvasUpdate>,
    pub(super) data: Option<PendingCanvasUpdate>,
}

impl Default for PendingCanvasUpdates {
    fn default() -> Self {
        Self {
            version: UPDATE_STATE_VERSION,
            presentation: None,
            data: None,
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CanvasIndex {
    format: String,
    version: u32,
    canvases: Vec<CanvasIndexEntry>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CanvasIndexEntry {
    community_id: String,
    project_id: String,
    source_path: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProjectCanvasSourceLocation {
    pub(crate) community_id: String,
    pub(crate) project_id: String,
    pub(crate) source_path: String,
    pub(crate) index_path: String,
}

pub(super) fn active_snapshot(
    canvas_root: &Path,
    binding: &ProjectBinding,
) -> Result<Option<ValidatedSnapshot>, String> {
    let Some(canvas_root) = canonical_canvas_root(canvas_root, false)? else {
        return Ok(None);
    };
    let project_root = binding.project_root(&canvas_root);
    if !project_root.exists() {
        return Ok(None);
    }
    ensure_secure_descendant(&canvas_root, &project_root, false)?;
    let runtime_root = binding.runtime_root(&canvas_root);
    if !runtime_root.exists() {
        return Ok(None);
    }
    ensure_secure_descendant(&canvas_root, &runtime_root, false)?;
    let active_path = runtime_root.join(ACTIVE_FILE);
    if !active_path.exists() {
        return Ok(None);
    }
    ensure_secure_file(&canvas_root, &active_path)?;
    let raw = read_file_with_cap(&canvas_root, &active_path, 1024)?;
    let active: ActiveRevision = serde_json::from_slice(&raw)
        .map_err(|error| format!("invalid project canvas active revision: {error}"))?;
    validate_revision(&active.revision)?;

    let revision_root = runtime_root.join(REVISIONS_DIR).join(&active.revision);
    ensure_secure_descendant(&canvas_root, &revision_root, false)?;
    let package = scan_package(&canvas_root, &revision_root)?;
    if package.revision != active.revision {
        return Err("active project canvas snapshot failed its content hash".to_string());
    }

    Ok(Some(ValidatedSnapshot {
        files: Arc::new(package.files),
        revision: package.revision,
        manifest: package.manifest,
        data: package.data,
    }))
}

pub(super) fn snapshot_for_revision(
    canvas_root: &Path,
    binding: &ProjectBinding,
    revision: &str,
) -> Result<ValidatedSnapshot, String> {
    validate_revision(revision)?;
    let canvas_root = canonical_canvas_root(canvas_root, false)?
        .ok_or_else(|| "project canvas root does not exist".to_string())?;
    let runtime_root = binding.runtime_root(&canvas_root);
    ensure_secure_descendant(&canvas_root, &runtime_root, false)?;
    let revision_root = runtime_root.join(REVISIONS_DIR).join(revision);
    ensure_secure_descendant(&canvas_root, &revision_root, false)?;
    let package = scan_package(&canvas_root, &revision_root)?;
    if package.revision != revision {
        return Err("project canvas update snapshot failed its content hash".to_string());
    }
    Ok(ValidatedSnapshot {
        files: Arc::new(package.files),
        revision: package.revision,
        manifest: package.manifest,
        data: package.data,
    })
}

pub(super) fn prepare_snapshot(
    canvas_root: &Path,
    binding: &ProjectBinding,
    template: Option<&ValidatedPackage>,
) -> Result<ValidatedSnapshot, String> {
    let canvas_root = canonical_canvas_root(canvas_root, true)?
        .ok_or_else(|| "project canvas root was not created".to_string())?;
    let project_root = binding.project_root(&canvas_root);
    let project_parent = project_root
        .parent()
        .ok_or_else(|| "project canvas directory has no parent".to_string())?;
    ensure_secure_descendant(&canvas_root, project_parent, true)?;
    seed_if_missing(&canvas_root, &project_root, template)?;

    // Validation reads every source byte before creating a candidate revision.
    // The active pointer is advanced only after the iframe reports a successful
    // render through the bound MessageChannel.
    let package = scan_package(&canvas_root, &project_root)?;
    let runtime_root = binding.runtime_root(&canvas_root);
    let revisions_root = runtime_root.join(REVISIONS_DIR);
    ensure_secure_descendant(&canvas_root, &revisions_root, true)?;
    let revision_root = revisions_root.join(&package.revision);

    if revision_root.exists() {
        ensure_secure_descendant(&canvas_root, &revision_root, false)?;
        let existing = scan_package(&canvas_root, &revision_root)?;
        if existing.revision != package.revision {
            return Err("existing project canvas revision failed its content hash".to_string());
        }
    } else {
        create_snapshot(&revisions_root, &revision_root, &package)?;
    }

    Ok(ValidatedSnapshot {
        files: Arc::new(package.files),
        revision: package.revision,
        manifest: package.manifest,
        data: package.data,
    })
}

pub(super) fn commit_snapshot(
    canvas_root: &Path,
    binding: &ProjectBinding,
    revision: &str,
) -> Result<(), String> {
    validate_revision(revision)?;
    let canvas_root = canonical_canvas_root(canvas_root, false)?
        .ok_or_else(|| "project canvas root does not exist".to_string())?;
    let project_root = binding.project_root(&canvas_root);
    ensure_secure_descendant(&canvas_root, &project_root, false)?;
    let runtime_root = binding.runtime_root(&canvas_root);
    ensure_secure_descendant(&canvas_root, &runtime_root, false)?;
    let revision_root = runtime_root.join(REVISIONS_DIR).join(revision);
    ensure_secure_descendant(&canvas_root, &revision_root, false)?;
    let package = scan_package(&canvas_root, &revision_root)?;
    if package.revision != revision {
        return Err("project canvas candidate failed its content hash".to_string());
    }
    write_active_revision(&runtime_root.join(ACTIVE_FILE), revision)
}

pub(super) fn record_pending_update(
    canvas_root: &Path,
    binding: &ProjectBinding,
    change: super::ProjectCanvasUpdateChange,
    notification_id: &str,
    widget_id: &str,
    revision: &str,
) -> Result<(), String> {
    validate_notification_id(notification_id)?;
    validate_widget_id(widget_id)?;
    validate_revision(revision)?;
    let canvas_root = canonical_canvas_root(canvas_root, false)?
        .ok_or_else(|| "project canvas root does not exist".to_string())?;
    let runtime_root = binding.runtime_root(&canvas_root);
    ensure_secure_descendant(&canvas_root, &runtime_root, false)?;
    let revision_root = runtime_root.join(REVISIONS_DIR).join(revision);
    ensure_secure_descendant(&canvas_root, &revision_root, false)?;

    let mut updates = read_pending_updates_from_root(&canvas_root, &runtime_root)?;
    let update = Some(PendingCanvasUpdate {
        notification_id: notification_id.to_string(),
        revision: revision.to_string(),
        widget_id: widget_id.to_string(),
    });
    match change {
        super::ProjectCanvasUpdateChange::Presentation => {
            updates.presentation = update;
            updates.data = None;
        }
        super::ProjectCanvasUpdateChange::Data => updates.data = update,
    }
    write_pending_updates(&canvas_root, &runtime_root, &updates)
}

pub(super) fn pending_updates(
    canvas_root: &Path,
    binding: &ProjectBinding,
) -> Result<PendingCanvasUpdates, String> {
    let Some(canvas_root) = canonical_canvas_root(canvas_root, false)? else {
        return Ok(PendingCanvasUpdates::default());
    };
    let runtime_root = binding.runtime_root(&canvas_root);
    if !runtime_root.exists() {
        return Ok(PendingCanvasUpdates::default());
    }
    ensure_secure_descendant(&canvas_root, &runtime_root, false)?;
    read_pending_updates_from_root(&canvas_root, &runtime_root)
}

pub(super) fn clear_committed_updates(
    canvas_root: &Path,
    binding: &ProjectBinding,
    revision: &str,
) -> Result<(), String> {
    let Some(canvas_root) = canonical_canvas_root(canvas_root, false)? else {
        return Ok(());
    };
    let runtime_root = binding.runtime_root(&canvas_root);
    if !runtime_root.exists() {
        return Ok(());
    }
    ensure_secure_descendant(&canvas_root, &runtime_root, false)?;
    let mut updates = read_pending_updates_from_root(&canvas_root, &runtime_root)?;
    if updates
        .presentation
        .as_ref()
        .is_some_and(|update| update.revision == revision)
    {
        updates.presentation = None;
    }
    if let Some(update) = &updates.data {
        let committed = snapshot_for_revision(&canvas_root, binding, revision)?;
        let pending = snapshot_for_revision(&canvas_root, binding, &update.revision)?;
        if update.revision == revision || pending.data == committed.data {
            updates.data = None;
        }
    }
    write_pending_updates(&canvas_root, &runtime_root, &updates)
}

pub(super) fn prune_revisions(
    canvas_root: &Path,
    binding: &ProjectBinding,
    retained: &BTreeSet<String>,
) -> Result<(), String> {
    let Some(canvas_root) = canonical_canvas_root(canvas_root, false)? else {
        return Ok(());
    };
    let project_root = binding.project_root(&canvas_root);
    if !project_root.exists() {
        return Ok(());
    }
    ensure_secure_descendant(&canvas_root, &project_root, false)?;
    let runtime_root = binding.runtime_root(&canvas_root);
    if !runtime_root.exists() {
        return Ok(());
    }
    ensure_secure_descendant(&canvas_root, &runtime_root, false)?;
    let revisions_root = runtime_root.join(REVISIONS_DIR);
    if !revisions_root.exists() {
        return Ok(());
    }
    ensure_secure_descendant(&canvas_root, &revisions_root, false)?;

    let mut keep = retained.clone();
    let updates = read_pending_updates_from_root(&canvas_root, &runtime_root)?;
    keep.extend(
        [updates.presentation, updates.data]
            .into_iter()
            .flatten()
            .map(|update| update.revision),
    );
    let active_path = runtime_root.join(ACTIVE_FILE);
    if active_path.exists() {
        ensure_secure_file(&canvas_root, &active_path)?;
        let raw = read_file_with_cap(&canvas_root, &active_path, 1024)?;
        let active: ActiveRevision = serde_json::from_slice(&raw)
            .map_err(|error| format!("invalid project canvas active revision: {error}"))?;
        validate_revision(&active.revision)?;
        keep.insert(active.revision);
    }

    let mut revisions = Vec::new();
    for entry in fs::read_dir(&revisions_root)
        .map_err(|error| format!("read project canvas revisions: {error}"))?
    {
        let entry = entry.map_err(|error| format!("read project canvas revision: {error}"))?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| "project canvas revision names must be UTF-8".to_string())?;
        if name == ".DS_Store" {
            continue;
        }
        if let Some(id) = name.strip_prefix(".staging-") {
            if uuid::Uuid::parse_str(id)
                .map(|parsed| parsed.simple().to_string())
                .as_deref()
                != Ok(id)
            {
                return Err("invalid project canvas staging revision".to_string());
            }
            let path = entry.path();
            ensure_secure_descendant(&canvas_root, &path, false)?;
            make_tree_writable(&path)?;
            fs::remove_dir_all(&path).map_err(|error| {
                format!("remove stale project canvas staging revision: {error}")
            })?;
            continue;
        }
        validate_revision(&name)?;
        let path = entry.path();
        ensure_secure_descendant(&canvas_root, &path, false)?;
        let modified = entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .map_err(|error| format!("inspect project canvas revision: {error}"))?;
        revisions.push((modified, name, path));
    }
    revisions.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| right.1.cmp(&left.1)));
    keep.extend(
        revisions
            .iter()
            .take(RECENT_REVISION_RETENTION)
            .map(|(_, revision, _)| revision.clone()),
    );

    for (_, revision, path) in revisions {
        if keep.contains(&revision) {
            continue;
        }
        make_tree_writable(&path)?;
        fs::remove_dir_all(&path)
            .map_err(|error| format!("remove old project canvas revision: {error}"))?;
    }
    Ok(())
}

pub(super) fn record_source_binding(
    canvas_root: &Path,
    binding: &ProjectBinding,
) -> Result<ProjectCanvasSourceLocation, String> {
    let location = project_source_location(canvas_root, binding)?;
    let canvas_root = canonical_canvas_root(canvas_root, false)?
        .ok_or_else(|| "project canvas root does not exist".to_string())?;
    let index_path = canvas_root.join(INDEX_FILE);
    let mut index = if index_path.exists() {
        ensure_secure_file(&canvas_root, &index_path)?;
        let raw = read_file_with_cap(&canvas_root, &index_path, MAX_INDEX_BYTES)?;
        serde_json::from_slice::<CanvasIndex>(&raw)
            .map_err(|error| format!("invalid project canvas index: {error}"))?
    } else {
        CanvasIndex {
            format: INDEX_FORMAT.to_string(),
            version: INDEX_VERSION,
            canvases: Vec::new(),
        }
    };
    validate_index(&canvas_root, &index)?;

    index.canvases.retain(|entry| {
        entry.community_id != binding.community_id || entry.project_id != binding.project_id
    });
    index.canvases.push(CanvasIndexEntry {
        community_id: binding.community_id.clone(),
        project_id: binding.project_id.clone(),
        source_path: location.source_path.clone(),
    });
    index.canvases.sort_by(|left, right| {
        left.community_id
            .cmp(&right.community_id)
            .then_with(|| left.project_id.cmp(&right.project_id))
    });
    validate_index(&canvas_root, &index)?;
    let bytes = serde_json::to_vec_pretty(&index)
        .map_err(|error| format!("encode project canvas index: {error}"))?;
    if bytes.len() > MAX_INDEX_BYTES {
        return Err("project canvas index exceeds 1 MiB".to_string());
    }
    if index_path.exists() {
        ensure_secure_file(&canvas_root, &index_path)?;
    }
    let mut file = AtomicWriteFile::open(&index_path)
        .map_err(|error| format!("open project canvas index: {error}"))?;
    file.write_all(&bytes)
        .map_err(|error| format!("write project canvas index: {error}"))?;
    file.commit()
        .map_err(|error| format!("commit project canvas index: {error}"))?;

    Ok(location)
}

pub(super) fn project_source_location(
    canvas_root: &Path,
    binding: &ProjectBinding,
) -> Result<ProjectCanvasSourceLocation, String> {
    let canvas_root = canonical_canvas_root(canvas_root, false)?
        .ok_or_else(|| "project canvas root does not exist".to_string())?;
    let project_root = binding.project_root(&canvas_root);
    ensure_secure_descendant(&canvas_root, &project_root, false)?;
    Ok(ProjectCanvasSourceLocation {
        community_id: binding.community_id.clone(),
        project_id: binding.project_id.clone(),
        source_path: project_root.to_string_lossy().into_owned(),
        index_path: canvas_root.join(INDEX_FILE).to_string_lossy().into_owned(),
    })
}

fn validate_index(canvas_root: &Path, index: &CanvasIndex) -> Result<(), String> {
    if index.format != INDEX_FORMAT || index.version != INDEX_VERSION {
        return Err("unsupported project canvas index format".to_string());
    }
    if index.canvases.len() > MAX_INDEX_ENTRIES {
        return Err("project canvas index exceeds 4096 entries".to_string());
    }
    let mut seen = BTreeSet::new();
    for entry in &index.canvases {
        if !seen.insert((&entry.community_id, &entry.project_id)) {
            return Err("project canvas index contains a duplicate binding".to_string());
        }
        let indexed = ProjectBinding::parse(ProjectCanvasPackageRequest {
            community_id: entry.community_id.clone(),
            project_id: entry.project_id.clone(),
        })?;
        let expected_path = indexed.project_root(canvas_root);
        let expected = expected_path.to_string_lossy();
        if entry.source_path != expected {
            return Err("project canvas index contains a mismatched source path".to_string());
        }
    }
    Ok(())
}

fn seed_if_missing(
    canvas_root: &Path,
    project_root: &Path,
    template: Option<&ValidatedPackage>,
) -> Result<(), String> {
    if project_root.join("manifest.json").is_file() {
        ensure_secure_descendant(canvas_root, project_root, false)?;
        return ensure_secure_file(canvas_root, &project_root.join("manifest.json"));
    }
    if project_root.exists() {
        ensure_secure_descendant(canvas_root, project_root, false)?;
        let has_source = fs::read_dir(project_root)
            .map_err(|error| format!("read project canvas directory: {error}"))?
            .filter_map(Result::ok)
            .next()
            .is_some();
        if has_source {
            return Err(
                "project canvas source is incomplete; manifest.json is missing".to_string(),
            );
        }
        fs::remove_dir(project_root)
            .map_err(|error| format!("remove empty project canvas directory: {error}"))?;
    }

    let package = template.ok_or_else(|| "project canvas template is unavailable".to_string())?;
    let parent = project_root
        .parent()
        .ok_or_else(|| "project canvas directory has no parent".to_string())?;
    ensure_secure_descendant(canvas_root, parent, false)?;
    let staging = parent.join(format!(".seed-{}", uuid::Uuid::new_v4().simple()));
    fs::create_dir(&staging)
        .map_err(|error| format!("create project canvas seed staging directory: {error}"))?;
    let result = (|| {
        write_package_files(&staging, &package.files)?;
        fs::rename(&staging, project_root)
            .map_err(|error| format!("activate seeded project canvas package: {error}"))?;
        Ok(())
    })();
    if result.is_err() {
        let _ = make_tree_writable(&staging);
        let _ = fs::remove_dir_all(&staging);
    }
    result
}

fn scan_package(trusted_root: &Path, root: &Path) -> Result<ValidatedPackage, String> {
    ensure_package_dir(root)?;
    validate_package_files(read_package_tree(trusted_root, root)?)
}

#[cfg(test)]
pub(super) fn scan_package_for_test(trusted_root: &Path, root: &Path) -> Result<(), String> {
    scan_package(trusted_root, root).map(|_| ())
}

/// One non-following inspection covering existence, symlink, and directory.
///
/// `symlink_metadata` names the path it failed on, where the previous
/// `ensure_no_symlink` + `Path::is_dir()` pair died inside the metadata call
/// with an unnamed `os error 2`. It is also strictly stronger: the directory
/// test now reads off the non-following metadata rather than `Path::is_dir()`,
/// which follows links.
fn ensure_package_dir(root: &Path) -> Result<(), String> {
    let metadata = match fs::symlink_metadata(root) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => {
            return Err(format!(
                "project canvas package directory does not exist: {}",
                root.display()
            ))
        }
        Err(error) => return Err(format!("inspect project canvas path: {error}")),
    };
    if metadata.file_type().is_symlink() {
        return Err(format!(
            "project canvas paths cannot be symlinks: {}",
            root.display()
        ));
    }
    if !metadata.is_dir() {
        return Err(format!(
            "project canvas package is not a directory: {}",
            root.display()
        ));
    }
    Ok(())
}

/// The single validation gate for every canvas package, read off disk or
/// embedded in the binary. There is no second, weaker path.
pub(super) fn validate_package_files(
    files: BTreeMap<String, Vec<u8>>,
) -> Result<ValidatedPackage, String> {
    if files.len() > MAX_PACKAGE_FILES {
        return Err(format!(
            "project canvas package exceeds {MAX_PACKAGE_FILES} files"
        ));
    }

    let mut total = 0usize;
    for (path, bytes) in &files {
        if mime_type(path).is_none() && path != "manifest.json" {
            return Err(format!("unsupported project canvas file type: {path}"));
        }
        if bytes.len() > MAX_FILE_BYTES {
            return Err(format!("project canvas file exceeds 8 MiB: {path}"));
        }
        if is_text(path) {
            if bytes.len() > MAX_TEXT_BYTES {
                return Err(format!("project canvas text file exceeds 2 MiB: {path}"));
            }
            std::str::from_utf8(bytes)
                .map_err(|_| format!("project canvas text file must be UTF-8: {path}"))?;
        }
        total = total
            .checked_add(bytes.len())
            .ok_or_else(|| "project canvas package size overflow".to_string())?;
    }
    if total > MAX_PACKAGE_BYTES {
        return Err("project canvas package exceeds 32 MiB".to_string());
    }

    let (manifest, data) = validate_manifest(&files)?;
    let revision = hash_files(&files);
    Ok(ValidatedPackage {
        files,
        revision,
        manifest,
        data,
    })
}

fn create_snapshot(
    revisions_root: &Path,
    revision_root: &Path,
    package: &ValidatedPackage,
) -> Result<(), String> {
    let staging = revisions_root.join(format!(".staging-{}", uuid::Uuid::new_v4().simple()));
    fs::create_dir(&staging)
        .map_err(|error| format!("create project canvas staging revision: {error}"))?;
    let result = (|| {
        write_package_files(&staging, &package.files)?;
        make_snapshot_read_only(&staging)?;
        fs::rename(&staging, revision_root)
            .map_err(|error| format!("activate project canvas revision: {error}"))?;
        Ok(())
    })();
    if result.is_err() {
        let _ = make_tree_writable(&staging);
        let _ = fs::remove_dir_all(&staging);
    }
    result
}

fn write_package_files(root: &Path, files: &BTreeMap<String, Vec<u8>>) -> Result<(), String> {
    for (relative, bytes) in files {
        let destination = root.join(relative);
        let parent = destination
            .parent()
            .ok_or_else(|| "project canvas file has no parent".to_string())?;
        ensure_secure_descendant(root, parent, true)?;
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&destination)
            .map_err(|error| format!("create project canvas file: {error}"))?;
        file.write_all(bytes)
            .map_err(|error| format!("write project canvas file: {error}"))?;
        file.sync_all()
            .map_err(|error| format!("sync project canvas file: {error}"))?;
    }
    Ok(())
}

fn write_active_revision(path: &Path, revision: &str) -> Result<(), String> {
    ensure_no_symlink(
        path.parent()
            .ok_or_else(|| "project canvas active revision has no parent directory".to_string())?,
    )?;
    if path.exists() {
        ensure_no_symlink(path)?;
    }
    let bytes = serde_json::to_vec(&ActiveRevision {
        revision: revision.to_string(),
    })
    .map_err(|error| format!("encode project canvas active revision: {error}"))?;
    let mut file = AtomicWriteFile::open(path)
        .map_err(|error| format!("open project canvas active revision: {error}"))?;
    file.write_all(&bytes)
        .map_err(|error| format!("write project canvas active revision: {error}"))?;
    file.commit()
        .map_err(|error| format!("commit project canvas active revision: {error}"))
}

fn read_pending_updates_from_root(
    canvas_root: &Path,
    runtime_root: &Path,
) -> Result<PendingCanvasUpdates, String> {
    let path = runtime_root.join(UPDATES_FILE);
    if !path.exists() {
        return Ok(PendingCanvasUpdates::default());
    }
    ensure_secure_file(canvas_root, &path)?;
    let raw = read_file_with_cap(canvas_root, &path, MAX_UPDATE_STATE_BYTES)?;
    let updates: PendingCanvasUpdates = serde_json::from_slice(&raw)
        .map_err(|error| format!("invalid project canvas update state: {error}"))?;
    if updates.version != UPDATE_STATE_VERSION {
        return Err("unsupported project canvas update state version".to_string());
    }
    for update in [&updates.presentation, &updates.data].into_iter().flatten() {
        validate_notification_id(&update.notification_id)?;
        validate_widget_id(&update.widget_id)?;
        validate_revision(&update.revision)?;
        let revision_root = runtime_root.join(REVISIONS_DIR).join(&update.revision);
        ensure_secure_descendant(canvas_root, &revision_root, false)?;
    }
    Ok(updates)
}

fn write_pending_updates(
    canvas_root: &Path,
    runtime_root: &Path,
    updates: &PendingCanvasUpdates,
) -> Result<(), String> {
    let path = runtime_root.join(UPDATES_FILE);
    if path.exists() {
        ensure_secure_file(canvas_root, &path)?;
    }
    let bytes = serde_json::to_vec(updates)
        .map_err(|error| format!("encode project canvas update state: {error}"))?;
    let mut file = AtomicWriteFile::open(&path)
        .map_err(|error| format!("open project canvas update state: {error}"))?;
    file.write_all(&bytes)
        .map_err(|error| format!("write project canvas update state: {error}"))?;
    file.commit()
        .map_err(|error| format!("commit project canvas update state: {error}"))
}

fn validate_notification_id(value: &str) -> Result<(), String> {
    let parsed = uuid::Uuid::parse_str(value)
        .map_err(|_| "invalid project canvas update notification id".to_string())?;
    if parsed.simple().to_string() != value {
        return Err("invalid project canvas update notification id".to_string());
    }
    Ok(())
}

pub(super) fn validate_widget_id(value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(
            "widget id must be 1 to 128 ASCII letters, numbers, '.', '-', or '_'".to_string(),
        );
    }
    Ok(())
}

fn hash_files(files: &BTreeMap<String, Vec<u8>>) -> String {
    let mut hash = Sha256::new();
    for (path, bytes) in files {
        hash.update((path.len() as u64).to_be_bytes());
        hash.update(path.as_bytes());
        hash.update((bytes.len() as u64).to_be_bytes());
        hash.update(bytes);
    }
    hex::encode(hash.finalize())
}

fn scope_hash(value: &str) -> String {
    hex::encode(Sha256::digest(value.as_bytes()))
}

fn validate_scope_value(label: &str, value: &str, max_len: usize) -> Result<(), String> {
    if value.is_empty() || value.len() > max_len || value.chars().any(char::is_control) {
        return Err(format!("invalid project canvas {label}"));
    }
    Ok(())
}

fn validate_revision(revision: &str) -> Result<(), String> {
    if revision.len() != 64 || !revision.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("invalid project canvas revision".to_string());
    }
    Ok(())
}
