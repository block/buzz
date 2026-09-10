//! Headless browser plugin package and session service.
//!
//! `PluginHost` holds no Tauri and no wry type, so the full install → session
//! → resolve → teardown lifecycle can be driven and tested headlessly, on
//! any platform, independent of native browser code. The `browser_webview`
//! adapter and the thin Tauri `commands` layer are built on top of this
//! service, never the other way around.

pub mod browser_webview;
mod manifest;
mod mcp_stdio;
mod navigation;
mod package;
mod state;
pub mod types;

pub mod commands;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub use navigation::is_admitted;
pub use types::{
    Admitted, InstallReject, InstalledPlugin, PluginError, ResolveKind, Session, SessionId,
    StagedPackage,
};

use state::{RegistryRecord, RegistryStore};

/// Buzz's own contract version; the only value a manifest may declare and
/// the only value sent to a plugin as `context.contractVersion`.
const CONTRACT_VERSION: &str = "0.1.0-alpha";
/// Grace period between `SIGTERM` and `SIGKILL` when tearing down a plugin
/// process group.
const TERMINATE_GRACE: Duration = Duration::from_secs(2);

#[derive(Clone, Debug)]
/// Filesystem location and request deadlines for a plugin host.
pub struct PluginHostConfig {
    /// Directory containing the registry and installed packages.
    pub root: PathBuf,
    /// Maximum duration of one `tools/call` request.
    pub resolve_deadline: Duration,
    /// Maximum duration of each discovery request.
    pub discover_deadline: Duration,
}

/// Committed uninstall with separate process and package cleanup outcomes.
#[derive(Debug)]
pub struct UninstallOutcome {
    /// Invalidated session IDs whose native surfaces must close, even when
    /// process termination fails and remains owned for retry.
    pub terminated_sessions: Vec<SessionId>,
    /// Whether all pending and active processes were terminated and reaped.
    pub session_termination: Result<(), PluginError>,
    /// Whether the uninstalled package directory was removed.
    pub package_removal: Result<(), PluginError>,
}

/// Result of [`PluginHost::set_enabled`] when disabling: the registry flip is
/// unconditional and authoritative, but whether every invalidated session's
/// process group actually died is reported separately, mirroring
/// [`UninstallOutcome::session_termination`].
#[derive(Debug)]
pub struct DisableOutcome {
    /// Invalidated session IDs whose native surfaces must close.
    pub terminated_sessions: Vec<SessionId>,
    /// Whether all pending and active processes were terminated and reaped.
    pub session_termination: Result<(), PluginError>,
}

struct SessionEntry {
    plugin_id: String,
    contribution_id: String,
    generation: u64,
    closing: Arc<AtomicBool>,
    client: Arc<mcp_stdio::Client>,
}

// Pending entries remain owned during discovery and fallible cleanup. A closing
// reservation can never register, even after an identical reinstall or re-enable.
struct PendingStart {
    plugin_id: String,
    closing: bool,
    client: Arc<mcp_stdio::Client>,
}

#[derive(Default)]
struct PendingStarts {
    shutting_down: bool,
    entries: HashMap<u64, PendingStart>,
}

struct CleanupBatch {
    session_ids: Vec<SessionId>,
    pending_tokens: Vec<u64>,
    clients: Vec<Arc<mcp_stdio::Client>>,
}

/// Installs packages and manages plugin sessions.
pub struct PluginHost {
    store: RegistryStore,
    registry_lock: Mutex<()>,
    sessions: Mutex<HashMap<String, SessionEntry>>,
    pending_starts: Mutex<PendingStarts>,
    next_pending_start_id: std::sync::atomic::AtomicU64,
    #[cfg(test)]
    registration_barriers: Mutex<Option<(Arc<tokio::sync::Barrier>, Arc<tokio::sync::Barrier>)>>,
    resolve_deadline: Duration,
    discover_deadline: Duration,
}

impl PluginHost {
    /// Creates a host using the supplied storage and deadlines.
    pub fn new(config: PluginHostConfig) -> Self {
        Self {
            store: RegistryStore::new(config.root),
            registry_lock: Mutex::new(()),
            sessions: Mutex::new(HashMap::new()),
            pending_starts: Mutex::new(PendingStarts::default()),
            next_pending_start_id: std::sync::atomic::AtomicU64::new(0),
            #[cfg(test)]
            registration_barriers: Mutex::new(None),
            resolve_deadline: config.resolve_deadline,
            discover_deadline: config.discover_deadline,
        }
    }

    /// Copies and validates a package before commit.
    pub fn stage(&self, directory: &std::path::Path) -> Result<StagedPackage, PluginError> {
        package::stage(self.store.root(), directory, &self.store.staging_root())
            .map_err(PluginError::Install)
    }

    /// Publishes a staged package as an installed plugin.
    ///
    /// Serialized under the registry lock: reverifies the staged executable
    /// digest, rejects an id that already has a record (enabled or
    /// disabled) without touching it, moves the package directory into its
    /// final location, then persists one atomic registry record. A failed
    /// record write rolls back the directory move.
    pub fn commit(&self, staged: StagedPackage) -> Result<InstalledPlugin, PluginError> {
        let _guard = self
            .registry_lock
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        package::reverify(&staged).map_err(PluginError::Install)?;

        let mut file = self.store.load()?;
        if file.plugins.contains_key(&staged.manifest.id) {
            let _ = package::remove_owned_directory(self.store.root(), &staged.staged_path);
            return Err(PluginError::Install(InstallReject::AlreadyInstalled));
        }

        let digest = package::package_digest(&staged).map_err(PluginError::Install)?;
        let final_path = self
            .store
            .plugins_root()
            .join(&staged.manifest.id)
            .join(&digest);
        package::finalize(self.store.root(), &staged.staged_path, &final_path)
            .map_err(PluginError::Install)?;

        let record = RegistryRecord {
            plugin_id: staged.manifest.id.clone(),
            name: staged.manifest.name.clone(),
            version: staged.manifest.version.clone(),
            publisher: staged.manifest.publisher.clone(),
            enabled: true,
            contributions: staged
                .manifest
                .contributions
                .iter()
                .map(|contribution| types::ContributionSummary {
                    contribution_id: contribution.id.clone(),
                    title: contribution.title.clone(),
                })
                .collect(),
            package_path: final_path.clone(),
            digest,
            grants: staged.manifest.grants.clone(),
        };
        let summary = record.summary();
        file.plugins.insert(staged.manifest.id.clone(), record);
        if let Err(error) = self.store.save(&file) {
            let _ = package::remove_owned_directory(self.store.root(), &final_path);
            return Err(error);
        }
        Ok(summary)
    }

    /// Stages and commits a trusted package without a consent prompt.
    pub fn install(&self, directory: &std::path::Path) -> Result<InstalledPlugin, PluginError> {
        self.commit(self.stage(directory)?)
    }

    /// Discards a staged package the caller decided not to commit (a
    /// declined consent dialog), removing its staged directory.
    pub fn discard_staged(&self, staged: &StagedPackage) {
        let _ = package::remove_owned_directory(self.store.root(), &staged.staged_path);
    }

    /// Recovers from an interrupted install: removes any package directory
    /// under the plugins root, or staged directory under the staging root,
    /// that the registry no longer references. Never follows symlinks and
    /// never activates anything it finds; run once at startup, before any
    /// concurrent `stage`/`commit` could be in flight, since it clears the
    /// entire staging root.
    pub fn reconcile(&self) -> Result<(), PluginError> {
        let _guard = self
            .registry_lock
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let file = self.store.load()?;
        let plugins_root = self.store.plugins_root();
        let staging_root = self.store.staging_root();

        let top_level_referenced: std::collections::HashSet<PathBuf> = file
            .plugins
            .values()
            .map(|record| plugins_root.join(&record.plugin_id))
            .collect();
        package::reconcile(
            self.store.root(),
            &plugins_root,
            &top_level_referenced,
            &staging_root,
        )
        .map_err(|error| PluginError::Registry(format!("reconcile plugins root: {error}")))?;

        for record in file.plugins.values() {
            let id_root = plugins_root.join(&record.plugin_id);
            let mut referenced = std::collections::HashSet::new();
            referenced.insert(record.package_path.clone());
            package::reconcile(self.store.root(), &id_root, &referenced, &staging_root).map_err(
                |error| {
                    PluginError::Registry(format!(
                        "reconcile package directory for {}: {error}",
                        record.plugin_id
                    ))
                },
            )?;
        }
        Ok(())
    }

    /// Returns authoritative installed plugin records.
    pub fn list(&self) -> Result<Vec<InstalledPlugin>, PluginError> {
        let _guard = self
            .registry_lock
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let file = self.store.load()?;
        Ok(file.plugins.values().map(RegistryRecord::summary).collect())
    }

    /// Enables or disables an installed plugin. Disable invalidates sessions
    /// before cleanup; failed cleanup remains owned for retry or shutdown.
    pub fn set_enabled(
        &self,
        plugin_id: &str,
        enabled: bool,
    ) -> Result<DisableOutcome, PluginError> {
        let batch = {
            let _guard = self
                .registry_lock
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            let mut file = self.store.load()?;
            let record = file
                .plugins
                .get_mut(plugin_id)
                .ok_or(PluginError::UnknownPlugin)?;
            record.enabled = enabled;
            self.store.save(&file)?;
            if enabled {
                return Ok(DisableOutcome {
                    terminated_sessions: Vec::new(),
                    session_termination: Ok(()),
                });
            }
            self.invalidate_owned(Some(plugin_id))
        };
        let session_termination = self.terminate_batch(&batch);
        Ok(DisableOutcome {
            terminated_sessions: batch.session_ids,
            session_termination,
        })
    }

    /// Removes the registry record, package, and grant, and invalidates all
    /// plugin sessions. Reports package and process cleanup failures separately.
    pub fn uninstall(&self, plugin_id: &str) -> Result<UninstallOutcome, PluginError> {
        let (batch, package_removal) = {
            let _guard = self
                .registry_lock
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            let mut file = self.store.load()?;
            let record = file
                .plugins
                .remove(plugin_id)
                .ok_or(PluginError::UnknownPlugin)?;
            self.store.save(&file)?;
            let batch = self.invalidate_owned(Some(plugin_id));
            // Remove before releasing the registry lock: a concurrent identical
            // reinstall must not have its new package deleted by this uninstall.
            let package_removal =
                package::remove_owned_directory(self.store.root(), &record.package_path).map_err(
                    |error| {
                        PluginError::Registry(format!(
                            "failed to remove uninstalled plugin package directory {}: {error}",
                            record.package_path.display()
                        ))
                    },
                );
            (batch, package_removal)
        };
        let session_termination = self.terminate_batch(&batch);
        Ok(UninstallOutcome {
            terminated_sessions: batch.session_ids,
            session_termination,
            package_removal,
        })
    }

    /// Spawns a contribution process, discovers its contract, and resolves home.
    /// Reservations serialize with disable, uninstall, and shutdown; failed or
    /// cancelled startup retains process ownership until cleanup succeeds.
    pub async fn start_session(
        &self,
        plugin_id: &str,
        contribution_id: &str,
    ) -> Result<Session, PluginError> {
        let pending_token = self.next_pending_start_id.fetch_add(1, Ordering::Relaxed);
        let (package_path, client) = {
            let _guard = self
                .registry_lock
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            let mut pending = self
                .pending_starts
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            if pending.shutting_down {
                return Err(PluginError::NoSession);
            }
            let file = self.store.load()?;
            let record = file.plugins.get(plugin_id).ok_or(PluginError::NoSession)?;
            if !record.enabled
                || !record
                    .contributions
                    .iter()
                    .any(|contribution| contribution.contribution_id == contribution_id)
            {
                return Err(PluginError::NoSession);
            }
            let package_path = record.package_path.clone();
            let executable_path = resolve_executable_path(&package_path)?;
            let client = mcp_stdio::Client::spawn(&executable_path, plugin_id, CONTRACT_VERSION)?;
            pending.entries.insert(
                pending_token,
                PendingStart {
                    plugin_id: plugin_id.to_string(),
                    closing: false,
                    client: Arc::clone(&client),
                },
            );
            (package_path, client)
        };
        let session_id = uuid::Uuid::new_v4().to_string();
        let generation = 0;
        let discovery: Result<Admitted, PluginError> = async {
            client.discover(self.discover_deadline).await?;
            client.tools_list(self.discover_deadline).await?;
            client
                .resolve(
                    plugin_id,
                    contribution_id,
                    &session_id,
                    generation,
                    CONTRACT_VERSION,
                    ResolveKind::Home,
                    None,
                    self.resolve_deadline,
                )
                .await
        }
        .await;
        let home = match discovery {
            Ok(home) => home,
            Err(error) => {
                self.cleanup_pending(pending_token).await?;
                return Err(error);
            }
        };
        #[cfg(test)]
        {
            let barriers = self
                .registration_barriers
                .lock()
                .unwrap_or_else(|poison| poison.into_inner())
                .take();
            if let Some((reached, resume)) = barriers {
                reached.wait().await;
                resume.wait().await;
            }
        }
        let registration = {
            let _guard = self
                .registry_lock
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            let mut pending = self
                .pending_starts
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            if pending.shutting_down
                || pending
                    .entries
                    .get(&pending_token)
                    .is_none_or(|entry| entry.closing)
            {
                Err(PluginError::NoSession)
            } else {
                match self.store.load() {
                    Err(error) => Err(error),
                    Ok(file)
                        if !file.plugins.get(plugin_id).is_some_and(|record| {
                            record.enabled && record.package_path == package_path
                        }) =>
                    {
                        Err(PluginError::NoSession)
                    }
                    Ok(_) => {
                        self.sessions
                            .lock()
                            .unwrap_or_else(|poison| poison.into_inner())
                            .insert(
                                session_id.clone(),
                                SessionEntry {
                                    plugin_id: plugin_id.to_string(),
                                    contribution_id: contribution_id.to_string(),
                                    generation,
                                    closing: Arc::new(AtomicBool::new(false)),
                                    client,
                                },
                            );
                        pending.entries.remove(&pending_token);
                        Ok(())
                    }
                }
            }
        };
        if let Err(error) = registration {
            self.cleanup_pending(pending_token).await?;
            return Err(error);
        }
        Ok(Session {
            id: SessionId(session_id),
            generation,
            home_url: home.url,
        })
    }

    async fn cleanup_pending(&self, token: u64) -> Result<(), PluginError> {
        let client = {
            let _guard = self
                .registry_lock
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            let mut pending = self
                .pending_starts
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            let Some(entry) = pending.entries.get_mut(&token) else {
                return Ok(());
            };
            entry.closing = true;
            Arc::clone(&entry.client)
        };
        client.shutdown(TERMINATE_GRACE).await?;
        let _guard = self
            .registry_lock
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        self.pending_starts
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .entries
            .remove(&token);
        Ok(())
    }

    /// Resolves a home or user-supplied address through the plugin.
    ///
    /// A missing or closing session returns [`PluginError::NoSession`].
    /// Invalidation during a call returns [`PluginError::StaleGeneration`],
    /// including when successful cleanup has already removed the entry.
    pub async fn resolve(
        &self,
        session: SessionId,
        kind: ResolveKind,
        input: Option<&str>,
    ) -> Result<Admitted, PluginError> {
        let (plugin_id, contribution_id, generation, closing, client) = {
            let sessions = self
                .sessions
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            let entry = sessions.get(&session.0).ok_or(PluginError::NoSession)?;
            if entry.closing.load(Ordering::Acquire) {
                return Err(PluginError::NoSession);
            }
            (
                entry.plugin_id.clone(),
                entry.contribution_id.clone(),
                entry.generation,
                Arc::clone(&entry.closing),
                Arc::clone(&entry.client),
            )
        };

        let result = client
            .resolve(
                &plugin_id,
                &contribution_id,
                &session.0,
                generation,
                CONTRACT_VERSION,
                kind,
                input,
                self.resolve_deadline,
            )
            .await;

        let sessions = self
            .sessions
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if closing.load(Ordering::Acquire) {
            return Err(PluginError::StaleGeneration);
        }
        match sessions.get(&session.0) {
            None => Err(PluginError::NoSession),
            Some(entry) if entry.generation != generation => Err(PluginError::StaleGeneration),
            Some(_) => result,
        }
    }

    /// Returns the generation of a session that is still accepting commands.
    pub fn session_generation(&self, session_id: &SessionId) -> Option<u64> {
        self.sessions
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .get(&session_id.0)
            .filter(|entry| !entry.closing.load(Ordering::Acquire))
            .map(|entry| entry.generation)
    }

    /// Runs a synchronous operation only while the session is current and open.
    /// The operation holds the lifecycle lock and must not reenter the host.
    pub fn with_current_session<T>(
        &self,
        session_id: &SessionId,
        generation: u64,
        operation: impl FnOnce() -> T,
    ) -> Option<T> {
        let _guard = self
            .registry_lock
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let current = self
            .sessions
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .get(&session_id.0)
            .is_some_and(|entry| {
                entry.generation == generation && !entry.closing.load(Ordering::Acquire)
            });
        current.then(operation)
    }

    /// Ends a session. Already-closed sessions succeed; failed or cancelled
    /// cleanup retains an invalidated owner for a later close or shutdown.
    pub async fn end_session(&self, session: SessionId) -> Result<(), PluginError> {
        let client = {
            let _guard = self
                .registry_lock
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            let mut sessions = self
                .sessions
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            let Some(entry) = sessions.get_mut(&session.0) else {
                return Ok(());
            };
            Self::mark_closing(entry);
            Arc::clone(&entry.client)
        };
        client.shutdown(TERMINATE_GRACE).await?;
        let _guard = self
            .registry_lock
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        self.sessions
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .remove(&session.0);
        Ok(())
    }

    /// Rejects future starts, invalidates all sessions, and synchronously
    /// terminates owned processes without requiring an async runtime.
    /// Failed cleanup remains owned so another shutdown can retry it.
    pub fn shutdown(&self) -> Result<(), PluginError> {
        let batch = {
            let _guard = self
                .registry_lock
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            self.pending_starts
                .lock()
                .unwrap_or_else(|poison| poison.into_inner())
                .shutting_down = true;
            self.invalidate_owned(None)
        };
        self.terminate_batch(&batch)
    }

    fn mark_closing(entry: &mut SessionEntry) {
        if !entry.closing.swap(true, Ordering::AcqRel) {
            entry.generation += 1;
        }
    }

    // Caller holds registry_lock. IDs stay in their maps until the batch's
    // process cleanup succeeds; only their eligibility changes here.
    fn invalidate_owned(&self, plugin_id: Option<&str>) -> CleanupBatch {
        let mut batch = CleanupBatch {
            session_ids: Vec::new(),
            pending_tokens: Vec::new(),
            clients: Vec::new(),
        };
        {
            let mut sessions = self
                .sessions
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            for (id, entry) in sessions.iter_mut() {
                if plugin_id.is_none_or(|plugin_id| entry.plugin_id == plugin_id) {
                    Self::mark_closing(entry);
                    batch.session_ids.push(SessionId(id.clone()));
                    batch.clients.push(Arc::clone(&entry.client));
                }
            }
        }
        let mut pending = self
            .pending_starts
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        for (token, entry) in pending.entries.iter_mut() {
            if plugin_id.is_none_or(|plugin_id| entry.plugin_id == plugin_id) {
                entry.closing = true;
                batch.pending_tokens.push(*token);
                batch.clients.push(Arc::clone(&entry.client));
            }
        }
        batch
    }

    fn terminate_batch(&self, batch: &CleanupBatch) -> Result<(), PluginError> {
        mcp_stdio::terminate_clients_blocking(&batch.clients, TERMINATE_GRACE)?;
        let _guard = self
            .registry_lock
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let mut sessions = self
            .sessions
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        for id in &batch.session_ids {
            sessions.remove(&id.0);
        }
        let mut pending = self
            .pending_starts
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        for token in &batch.pending_tokens {
            pending.entries.remove(token);
        }
        Ok(())
    }
}

/// Default `tools/call` deadline: 2 seconds.
const DEFAULT_RESOLVE_DEADLINE: Duration = Duration::from_secs(2);
/// Default `server/discover`/`tools/list` deadline: 5 seconds.
const DEFAULT_DISCOVER_DEADLINE: Duration = Duration::from_secs(5);

/// Environment variable a debug-only smoke test script sets to redirect the
/// registry/package root at a fresh temporary directory it owns, so a smoke
/// run's repeated install/uninstall cannot touch a real user's plugins.
/// Only honored when [`commands::plugin_browser_smoke_enabled`] is `true`.
const SMOKE_ROOT_ENV_VAR: &str = "BUZZ_PLUGIN_BROWSER_SMOKE_ROOT";

/// Builds the managed `PluginHost` for the running app: its storage root is
/// the app's own data directory unless the debug-only smoke path overrides
/// it with an isolated temporary directory. Runs one startup reconciliation
/// pass to recover any package or staging directory left behind by an
/// interrupted install, and fails loudly (rather than silently falling back
/// to a relative, easy-to-lose directory) if the app data directory or that
/// reconciliation can't be resolved.
pub fn build_plugin_host(app: &tauri::AppHandle) -> Result<Arc<PluginHost>, String> {
    let root = if commands::plugin_browser_smoke_enabled() {
        std::env::var(SMOKE_ROOT_ENV_VAR)
            .ok()
            .map(PathBuf::from)
            .map(Ok)
            .unwrap_or_else(|| default_plugins_root(app))?
    } else {
        default_plugins_root(app)?
    };
    let host = Arc::new(PluginHost::new(PluginHostConfig {
        root,
        resolve_deadline: DEFAULT_RESOLVE_DEADLINE,
        discover_deadline: DEFAULT_DISCOVER_DEADLINE,
    }));
    host.reconcile()
        .map_err(|error| format!("plugin host startup reconciliation failed: {error:?}"))?;
    Ok(host)
}

fn default_plugins_root(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    use tauri::Manager;
    app.path()
        .app_data_dir()
        .map(|dir| dir.join("plugins"))
        .map_err(|error| format!("resolve app data directory for plugin host: {error}"))
}

fn resolve_executable_path(package_path: &std::path::Path) -> Result<PathBuf, PluginError> {
    let manifest_bytes = std::fs::read(package_path.join("manifest.json")).map_err(|error| {
        PluginError::Install(InstallReject::InvalidPackage(format!(
            "read installed manifest: {error}"
        )))
    })?;
    let manifest = manifest::parse_and_validate(&manifest_bytes).map_err(PluginError::Install)?;
    let triple = env!("BUZZ_PLUGIN_TARGET_TRIPLE");
    let target = manifest
        .runtime
        .targets
        .get(triple)
        .ok_or(PluginError::Install(InstallReject::UnknownTarget))?;
    Ok(package_path.join(&target.path))
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;

#[cfg(all(test, unix))]
#[path = "lifecycle_tests.rs"]
mod lifecycle_tests;
