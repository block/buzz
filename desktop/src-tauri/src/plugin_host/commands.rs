//! Thin async Tauri commands over the headless [`PluginHost`] service and the
//! native `browser_webview` adapter.
//!
//! Native operations run on the main thread after checking session validity
//! and whether the initiating command is still current.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::plugin_host::browser_webview;
use crate::plugin_host::types::{
    Bounds, InstalledPlugin, OpenedSurface, PageLoad, PluginError, ResolveKind, SessionId,
};
use crate::plugin_host::PluginHost;

#[path = "command_actions.rs"]
mod command_actions;
use command_actions::CommandActions;

/// Runs `f` on the Tauri main thread and awaits its result. `browser_webview`
/// owns a `wry::WebView`, which is neither `Send` nor `Sync` and must only
/// ever be touched from the main thread; every command that reaches it goes
/// through this so no plugin command blocks the caller or the event loop
/// while doing so.
async fn on_main_thread<T, F>(app: &AppHandle, f: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.run_on_main_thread(move || {
        let _ = tx.send(f());
    })
    .map_err(|error| format!("main thread dispatch failed: {error}"))?;
    rx.await
        .map_err(|_| "main thread task was dropped before responding".to_string())
}

fn platform_supported() -> bool {
    cfg!(target_os = "macos")
}

const LOAD_DEADLINE: Duration = Duration::from_secs(30);

/// Tracks the one live page-load deadline per session: armed on open,
/// reload, and a non-fragment address submission, reset on every `Started`,
/// and cancelled on `Finished`, a failed native dispatch, session close, or
/// process termination. Elapsing emits a `load-failed` `plugin-browser-error`
/// with the current page left visible.
///
/// Callers must arm *before* the native action that starts the load, not
/// after awaiting it: `on_page_load`'s `Started`/`Finished` callbacks can run
/// synchronously inside that native call (on the main thread, before control
/// returns to this async command), so arming afterward can race a real
/// `Finished` and then install a timer for a load that already completed.
#[derive(Default)]
pub struct LoadDeadlines {
    timers: Arc<Mutex<HashMap<String, LoadTimer>>>,
    actions: CommandActions,
    committed_urls: Mutex<HashMap<String, Option<String>>>,
}

struct LoadTimer {
    token: Arc<()>,
    task: tauri::async_runtime::JoinHandle<()>,
}

impl LoadDeadlines {
    fn arm_with(
        &self,
        deadline: Duration,
        session_id: String,
        on_expire: impl FnOnce() + Send + 'static,
    ) {
        self.arm_until(
            session_id,
            async move { tokio::time::sleep(deadline).await },
            on_expire,
        );
    }

    fn arm_until(
        &self,
        session_id: String,
        expired: impl std::future::Future<Output = ()> + Send + 'static,
        on_expire: impl FnOnce() + Send + 'static,
    ) -> Arc<()> {
        let token = Arc::new(());
        let expiry_token = Arc::clone(&token);
        let timers = Arc::clone(&self.timers);
        let expiry_session = session_id.clone();
        let mut current = self
            .timers
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(previous) = current.remove(&session_id) {
            previous.task.abort();
        }
        let task = tauri::async_runtime::spawn(async move {
            expired.await;
            Self::expire_current(&timers, &expiry_session, &expiry_token, on_expire);
        });
        current.insert(
            session_id,
            LoadTimer {
                token: Arc::clone(&token),
                task,
            },
        );
        token
    }

    fn expire_current(
        timers: &Mutex<HashMap<String, LoadTimer>>,
        session_id: &str,
        token: &Arc<()>,
        on_expire: impl FnOnce(),
    ) {
        // Keep validation and emission under one lock. Cancellation cannot
        // return and then be followed by an already-ready obsolete expiry.
        let mut current = timers.lock().unwrap_or_else(|error| error.into_inner());
        if current
            .get(session_id)
            .is_some_and(|timer| Arc::ptr_eq(&timer.token, token))
        {
            current.remove(session_id);
            on_expire();
        }
    }

    /// Arms (or re-arms) the production 30 s deadline for `session_id`,
    /// emitting the fixed `load-failed` event on expiry.
    fn arm(&self, app: AppHandle, session_id: String, generation: u64) {
        let emit_session = session_id.clone();
        self.arm_with(LOAD_DEADLINE, session_id, move || {
            let _ = app.emit(
                "plugin-browser-error",
                BrowserErrorPayload {
                    session_id: emit_session,
                    generation,
                    code: "load-failed",
                    message: "The page failed to load.",
                },
            );
        });
    }

    pub(super) fn cancel(&self, session_id: &str) {
        if let Some(timer) = self
            .timers
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(session_id)
        {
            timer.task.abort();
        }
    }

    pub(super) fn stop_loading(&self, session_id: &str) {
        let mut committed = self
            .committed_urls
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        committed.remove(session_id);
        self.cancel(session_id);
    }

    fn forget(&self, session_id: &str) {
        self.actions.remove(session_id);
        self.stop_loading(session_id);
    }

    fn register(&self, session_id: &str) {
        self.committed_urls
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .insert(session_id.to_owned(), None);
    }

    fn committed_url(&self, session_id: &str) -> Option<String> {
        self.committed_urls
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get(session_id)
            .cloned()
            .flatten()
    }

    fn begin_load(&self, app: &AppHandle, session_id: &str, generation: u64, url: String) {
        self.arm(app.clone(), session_id.to_owned(), generation);
        let _ = app.emit(
            "plugin-browser-loading",
            LoadingPayload {
                session_id: session_id.to_owned(),
                generation,
                url,
            },
        );
    }

    fn page_load(
        &self,
        app: &AppHandle,
        session_id: &str,
        generation: u64,
        load: PageLoad,
        url: String,
    ) {
        // Closing removes this entry under the same lock. A late delegate
        // callback therefore cannot restore a closed session's timer.
        let mut committed = self
            .committed_urls
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let Some(current) = committed.get_mut(session_id) else {
            return;
        };
        *current = Some(url.clone());
        match load {
            PageLoad::Started => self.begin_load(app, session_id, generation, url),
            PageLoad::Finished => {
                self.cancel(session_id);
                let _ = app.emit(
                    "plugin-browser-loaded",
                    LoadedPayload {
                        session_id: session_id.to_owned(),
                        generation,
                        url,
                    },
                );
            }
        }
    }
}

#[cfg(test)]
mod load_deadline_tests {
    use super::{address_plan, closed_surface_result, AddressPlan, LoadDeadlines};
    use crate::plugin_host::types::BrowserError;
    use std::time::Duration;

    #[tokio::test]
    async fn expiry_fires_when_nothing_cancels_it() {
        let deadlines = LoadDeadlines::default();
        let (trigger, expired) = tokio::sync::oneshot::channel();
        let (sender, receiver) = tokio::sync::oneshot::channel();
        deadlines.arm_until(
            "session".into(),
            async move {
                expired.await.unwrap();
            },
            move || {
                let _ = sender.send(());
            },
        );
        trigger.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(5), receiver)
            .await
            .expect("expiry completed")
            .expect("expiry callback fired");
        assert!(deadlines.timers.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn cancel_before_expiry_suppresses_it() {
        let deadlines = LoadDeadlines::default();
        let (sender, receiver) = tokio::sync::oneshot::channel();
        let ticket = deadlines.arm_until("session".into(), std::future::pending(), move || {
            let _ = sender.send(());
        });
        deadlines.cancel("session");
        let emitted = std::cell::Cell::new(false);
        LoadDeadlines::expire_current(&deadlines.timers, "session", &ticket, || emitted.set(true));
        assert!(
            !emitted.get(),
            "an already-ready cancelled expiry must not emit"
        );
        assert!(tokio::time::timeout(Duration::from_secs(5), receiver)
            .await
            .expect("cancelled task released callback")
            .is_err());
    }

    #[tokio::test]
    async fn rearming_cancels_the_previous_timer() {
        let deadlines = LoadDeadlines::default();
        let (first_sender, first_receiver) = tokio::sync::oneshot::channel();
        let old_ticket = deadlines.arm_until("session".into(), std::future::pending(), move || {
            let _ = first_sender.send(());
        });
        let (trigger, expired) = tokio::sync::oneshot::channel();
        let (second_sender, second_receiver) = tokio::sync::oneshot::channel();
        deadlines.arm_until(
            "session".into(),
            async move {
                expired.await.unwrap();
            },
            move || {
                let _ = second_sender.send(());
            },
        );
        let emitted = std::cell::Cell::new(false);
        LoadDeadlines::expire_current(&deadlines.timers, "session", &old_ticket, || {
            emitted.set(true)
        });
        assert!(
            !emitted.get(),
            "obsolete expiry must not remove or fire the replacement"
        );
        assert!(tokio::time::timeout(Duration::from_secs(5), first_receiver)
            .await
            .expect("superseded task released callback")
            .is_err());
        trigger.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(5), second_receiver)
            .await
            .expect("replacement expired")
            .expect("replacement callback fired");
    }

    #[tokio::test]
    async fn stopping_a_dead_view_removes_load_tracking_and_pending_expiry() {
        let deadlines = LoadDeadlines::default();
        deadlines.register("session");
        let (sender, receiver) = tokio::sync::oneshot::channel();
        deadlines.arm_until("session".into(), std::future::pending(), move || {
            let _ = sender.send(());
        });
        deadlines.stop_loading("session");
        assert!(!deadlines
            .committed_urls
            .lock()
            .unwrap()
            .contains_key("session"));
        assert!(tokio::time::timeout(Duration::from_secs(1), receiver)
            .await
            .expect("stopped timer released callback")
            .is_err());
    }

    #[test]
    fn closing_propagates_dispatch_failures_but_accepts_an_absent_view() {
        assert_eq!(
            closed_surface_result(Err("dispatch rejected".into())),
            Err("dispatch rejected".into())
        );
        let missing = BrowserError {
            session_id: "session".into(),
            generation: 1,
            code: "plugin-unavailable".into(),
            message: "already closed".into(),
        };
        assert_eq!(closed_surface_result(Ok(Err(missing))), Ok(()));
        let failure = BrowserError {
            session_id: "session".into(),
            generation: 1,
            code: "load-failed".into(),
            message: "close failed".into(),
        };
        assert_eq!(
            closed_surface_result(Ok(Err(failure))),
            Err("close failed".into())
        );
    }

    #[test]
    fn address_planning_distinguishes_live_fragments_from_provisional_documents() {
        for (committed, live, target, expected) in [
            (
                Some("https://a.test/p#one"),
                Some("https://a.test/p#two"),
                "https://a.test/p#two",
                AddressPlan::Reload,
            ),
            (
                Some("https://a.test/p#one"),
                Some("https://a.test/p#two"),
                "https://a.test/p#one",
                AddressPlan::Fragment,
            ),
            (
                Some("https://a.test/p"),
                Some("https://a.test/p"),
                "https://a.test/p#two",
                AddressPlan::Fragment,
            ),
            (
                Some("https://a.test/p#two"),
                Some("https://a.test/p#two"),
                "https://a.test/p",
                AddressPlan::Fragment,
            ),
            (
                Some("https://a.test/old"),
                Some("https://a.test/p#one"),
                "https://a.test/p#two",
                AddressPlan::Navigate,
            ),
            (
                Some("https://a.test/old"),
                Some("https://a.test/p"),
                "https://a.test/old",
                AddressPlan::Navigate,
            ),
            (
                Some("https://a.test/"),
                Some("https://a.test/"),
                "https://a.test",
                AddressPlan::Reload,
            ),
            (
                None,
                Some("https://a.test/p"),
                "https://a.test/p",
                AddressPlan::Navigate,
            ),
            (
                Some("https://a.test/p"),
                None,
                "https://a.test/p#two",
                AddressPlan::Navigate,
            ),
        ] {
            assert_eq!(
                address_plan(committed, live, target),
                expected,
                "{committed:?} / {live:?} -> {target}"
            );
        }
    }
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct BrowserErrorPayload {
    session_id: String,
    generation: u64,
    code: &'static str,
    message: &'static str,
}

const SMOKE_ENV_VAR: &str = "BUZZ_PLUGIN_BROWSER_SMOKE";
const SMOKE_DIRECTORY_ENV_VAR: &str = "BUZZ_PLUGIN_BROWSER_SMOKE_DIR";

fn smoke_enabled() -> bool {
    cfg!(debug_assertions) && std::env::var(SMOKE_ENV_VAR).as_deref() == Ok("1")
}

/// Whether the debug-only smoke path is active, so a test driver can confirm
/// production commands are answering it rather than a mocked bridge.
#[tauri::command]
pub fn plugin_browser_smoke_enabled() -> bool {
    smoke_enabled()
}

/// Debug-only sink for the main webview's own bootstrap JS errors, forwarded
/// here by an init script injected only under the smoke gate (see `lib.rs`'s
/// `on_webview_ready` for the "main" label). Exists purely so a headless
/// smoke run has a way to see a React-mount/bootstrap failure in the same
/// process stderr as the rest of the harness, instead of a silent blank
/// window and no signal beyond a timeout.
#[tauri::command]
pub fn plugin_browser_smoke_js_diagnostic(message: String) {
    if smoke_enabled() {
        eprintln!("buzz-desktop: [smoke-js-error] {message}");
    }
}

/// Opens a native directory picker and returns the chosen path, or the
/// smoke-test fixture directory when the debug smoke path is active.
#[tauri::command]
pub async fn plugin_pick_directory(app: AppHandle) -> Result<Option<String>, String> {
    if smoke_enabled() {
        eprintln!(
            "buzz-desktop: BUZZ_PLUGIN_BROWSER_SMOKE is set; skipping the native directory picker"
        );
        return Ok(std::env::var(SMOKE_DIRECTORY_ENV_VAR).ok());
    }
    use tauri_plugin_dialog::DialogExt;
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog().file().pick_folder(move |path| {
        let _ = tx.send(path);
    });
    let selected = rx
        .await
        .map_err(|_| "the directory picker closed unexpectedly".to_string())?;
    Ok(selected.and_then(|picked| {
        picked
            .as_path()
            .map(|path| path.to_string_lossy().into_owned())
    }))
}

/// Stages `directory`, shows the consent dialog naming the publisher, the
/// trusted-native-code disclosure, and the requested permission, and commits
/// only on confirmation. A decline discards the staged package and returns
/// `Ok(None)`.
#[tauri::command]
pub async fn plugin_install(
    app: AppHandle,
    host: State<'_, Arc<PluginHost>>,
    directory: String,
) -> Result<Option<InstalledPlugin>, String> {
    let host = Arc::clone(&host);
    let directory_path = PathBuf::from(directory);
    let staged = {
        let host = Arc::clone(&host);
        tokio::task::spawn_blocking(move || host.stage(&directory_path))
            .await
            .map_err(|error| format!("staging task failed: {error}"))?
            .map_err(|error| fixed_message(&error).to_string())?
    };

    let confirmed = if smoke_enabled() {
        eprintln!(
            "buzz-desktop: BUZZ_PLUGIN_BROWSER_SMOKE is set; auto-confirming the install consent dialog"
        );
        true
    } else {
        confirm_install(&app, &staged).await?
    };
    if !confirmed {
        host.discard_staged(&staged);
        return Ok(None);
    }

    let host_for_commit = Arc::clone(&host);
    let installed = tokio::task::spawn_blocking(move || host_for_commit.commit(staged))
        .await
        .map_err(|error| format!("commit task failed: {error}"))?
        .map_err(|error| fixed_message(&error).to_string())?;
    Ok(Some(installed))
}

async fn confirm_install(
    app: &AppHandle,
    staged: &crate::plugin_host::StagedPackage,
) -> Result<bool, String> {
    use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};
    let manifest = &staged.manifest;
    let grants = manifest.grants.join(", ");
    let body = format!(
        "{name} {version} by {publisher}\n\nThis plugin runs as trusted native code: it \
         executes as your own operating-system user, and process separation \
         is not a security boundary for it.\n\nRequested permission: {grants}",
        name = manifest.name,
        version = manifest.version,
        publisher = manifest.publisher,
    );
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .message(body)
        .title("Install plugin?")
        .kind(MessageDialogKind::Warning)
        .buttons(MessageDialogButtons::OkCancelCustom(
            "Install".to_string(),
            "Cancel".to_string(),
        ))
        .show(move |confirmed| {
            let _ = tx.send(confirmed);
        });
    rx.await
        .map_err(|_| "the install dialog closed unexpectedly".to_string())
}

/// Returns authoritative installed plugin records.
#[tauri::command]
pub async fn plugin_list(host: State<'_, Arc<PluginHost>>) -> Result<Vec<InstalledPlugin>, String> {
    let host = Arc::clone(&host);
    tokio::task::spawn_blocking(move || host.list())
        .await
        .map_err(|error| format!("list task failed: {error}"))?
        .map_err(|error| fixed_message(&error).to_string())
}

/// Enables or disables an installed plugin, closing the native surface of
/// any session this invalidates.
#[tauri::command]
pub async fn plugin_set_enabled(
    app: AppHandle,
    host: State<'_, Arc<PluginHost>>,
    deadlines: State<'_, LoadDeadlines>,
    plugin_id: String,
    enabled: bool,
) -> Result<(), String> {
    let host_for_task = Arc::clone(&host);
    let outcome =
        tokio::task::spawn_blocking(move || host_for_task.set_enabled(&plugin_id, enabled))
            .await
            .map_err(|error| format!("set_enabled task failed: {error}"))?
            .map_err(|error| fixed_message(&error).to_string())?;
    close_terminated_surfaces(&app, &deadlines, outcome.terminated_sessions).await?;
    outcome
        .session_termination
        .map_err(|error| fixed_message(&error).to_string())
}

/// Removes an installed plugin, its package, and its grant, closing the
/// native surface of any session this invalidates. Session termination and
/// the registry removal always happen and are never rolled back by a
/// package-directory removal failure, but that failure is still reported as
/// an `Err` rather than silently treated as full success — the orphaned
/// directory is recovered by the next startup reconciliation, but the caller
/// must know cleanup didn't fully complete.
#[tauri::command]
pub async fn plugin_uninstall(
    app: AppHandle,
    host: State<'_, Arc<PluginHost>>,
    deadlines: State<'_, LoadDeadlines>,
    plugin_id: String,
) -> Result<(), String> {
    let host_for_task = Arc::clone(&host);
    let outcome = tokio::task::spawn_blocking(move || host_for_task.uninstall(&plugin_id))
        .await
        .map_err(|error| format!("uninstall task failed: {error}"))?
        .map_err(|error| fixed_message(&error).to_string())?;
    close_terminated_surfaces(&app, &deadlines, outcome.terminated_sessions).await?;
    outcome
        .session_termination
        .map_err(|error| fixed_message(&error).to_string())?;
    outcome
        .package_removal
        .map_err(|error| fixed_message(&error).to_string())
}

/// Attempts every native close; a failed dispatch remains visible to the caller.
async fn close_terminated_surfaces(
    app: &AppHandle,
    deadlines: &LoadDeadlines,
    terminated: Vec<SessionId>,
) -> Result<(), String> {
    let mut first_error = None;
    for session_id in terminated {
        deadlines.forget(&session_id.0);
        let close_app = app.clone();
        let result =
            on_main_thread(app, move || browser_webview::close(&close_app, session_id)).await;
        if let Err(error) = closed_surface_result(result) {
            first_error.get_or_insert(error);
        }
    }
    first_error.map_or(Ok(()), Err)
}

fn closed_surface_result(
    result: Result<Result<(), crate::plugin_host::types::BrowserError>, String>,
) -> Result<(), String> {
    match result {
        Ok(Ok(())) => Ok(()),
        Ok(Err(error)) if error.code == "plugin-unavailable" => Ok(()),
        Ok(Err(error)) => Err(error.message),
        Err(error) => Err(error),
    }
}

// The action lock precedes the host lifecycle lock. Delegate callbacks use
// only LoadDeadlines, never either of these locks.
fn with_current_action<T>(
    app: &AppHandle,
    host: &PluginHost,
    session_id: &str,
    generation: u64,
    ticket: &Arc<()>,
    operation: impl FnOnce() -> T,
) -> Option<T> {
    app.state::<LoadDeadlines>()
        .actions
        .run(session_id, ticket, || {
            host.with_current_session(&SessionId(session_id.to_owned()), generation, operation)
        })
        .flatten()
}

/// Starts a contribution session and opens its native browser surface.
/// Rejects an unsupported platform before spawning the plugin process at
/// all, since opening the wry webview is the only platform-dependent step.
#[tauri::command]
pub async fn plugin_browser_open(
    app: AppHandle,
    host: State<'_, Arc<PluginHost>>,
    deadlines: State<'_, LoadDeadlines>,
    plugin_id: String,
    contribution_id: String,
    bounds: Bounds,
) -> Result<OpenedSurface, String> {
    if !platform_supported() {
        return Err(fixed_message(&PluginError::Unsupported).to_string());
    }

    let session = host
        .start_session(&plugin_id, &contribution_id)
        .await
        .map_err(|error| fixed_message(&error).to_string())?;

    let ticket = deadlines.actions.begin(&session.id.0);
    let event_app = app.clone();
    let event_session = session.id.0.clone();
    let generation = session.generation;
    let on_page_load: Arc<dyn Fn(PageLoad, String) + Send + Sync> = Arc::new(move |load, url| {
        event_app.state::<LoadDeadlines>().page_load(
            &event_app,
            &event_session,
            generation,
            load,
            url,
        );
    });
    let open_app = app.clone();
    let open_host = Arc::clone(&host);
    let open_session = session.id.clone();
    let open_home_url = session.home_url.clone();
    let result = on_main_thread(&app, move || {
        with_current_action(
            &open_app,
            &open_host,
            &open_session.0,
            generation,
            &ticket,
            || {
                let state = open_app.state::<LoadDeadlines>();
                state.register(&open_session.0);
                state.begin_load(
                    &open_app,
                    &open_session.0,
                    generation,
                    open_home_url.clone(),
                );
                browser_webview::open(
                    &open_app,
                    open_session.clone(),
                    generation,
                    &open_home_url,
                    bounds,
                    Arc::new(crate::plugin_host::is_admitted),
                    on_page_load,
                )
            },
        )
    })
    .await;
    let error = match result {
        Ok(Some(Ok(()))) => None,
        Ok(Some(Err(error))) => Some(error.message),
        Ok(None) => Some(fixed_message(&PluginError::StaleGeneration).to_owned()),
        Err(error) => Some(error),
    };
    if let Some(error) = error {
        deadlines.forget(&session.id.0);
        host.end_session(session.id)
            .await
            .map_err(|cleanup| fixed_message(&cleanup).to_owned())?;
        return Err(error);
    }
    Ok(OpenedSurface {
        session_id: session.id.0,
        generation: session.generation,
        home_url: session.home_url,
    })
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct LoadingPayload {
    session_id: String,
    generation: u64,
    url: String,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct LoadedPayload {
    session_id: String,
    generation: u64,
    url: String,
}

/// Resolves an address and applies it only while its command and session remain current.
#[tauri::command]
pub async fn plugin_browser_navigate(
    app: AppHandle,
    host: State<'_, Arc<PluginHost>>,
    deadlines: State<'_, LoadDeadlines>,
    session_id: String,
    input: String,
) -> Result<(), String> {
    let generation = host
        .session_generation(&SessionId(session_id.clone()))
        .ok_or_else(|| fixed_message(&PluginError::NoSession).to_owned())?;
    let ticket = deadlines.actions.begin(&session_id);
    let result = host
        .resolve(
            SessionId(session_id.clone()),
            ResolveKind::Address,
            Some(&input),
        )
        .await;
    let admitted = match result {
        Ok(admitted) => admitted,
        Err(PluginError::StaleGeneration) => return Ok(()),
        Err(error) => {
            return with_current_action(&app, &host, &session_id, generation, &ticket, || {
                emit_browser_error(&app, &session_id, generation, &error);
                Err(fixed_message(&error).to_owned())
            })
            .unwrap_or(Ok(()));
        }
    };
    dispatch_navigation(
        &app,
        Arc::clone(&host),
        session_id,
        generation,
        ticket,
        NativeNavigation::Address(admitted.url),
    )
    .await
}

enum NativeNavigation {
    Address(String),
    Back,
    Forward,
    Reload,
}

async fn dispatch_navigation(
    app: &AppHandle,
    host: Arc<PluginHost>,
    session_id: String,
    generation: u64,
    ticket: Arc<()>,
    navigation: NativeNavigation,
) -> Result<(), String> {
    let dispatch_app = app.clone();
    let dispatch_host = Arc::clone(&host);
    let dispatch_session = session_id.clone();
    let dispatch_ticket = Arc::clone(&ticket);
    let result = on_main_thread(app, move || {
        with_current_action(
            &dispatch_app,
            &dispatch_host,
            &dispatch_session,
            generation,
            &dispatch_ticket,
            || {
                let state = dispatch_app.state::<LoadDeadlines>();
                let current = state.committed_url(&dispatch_session);
                state.cancel(&dispatch_session);
                let session = SessionId(dispatch_session.clone());
                let result = match navigation {
                    NativeNavigation::Address(url) => {
                        let live =
                            browser_webview::current_url(&dispatch_app, session.clone()).ok();
                        let plan = address_plan(current.as_deref(), live.as_deref(), &url);
                        if plan != AddressPlan::Fragment {
                            state.begin_load(
                                &dispatch_app,
                                &dispatch_session,
                                generation,
                                url.clone(),
                            );
                        }
                        if plan == AddressPlan::Reload {
                            browser_webview::reload(&dispatch_app, session)
                        } else {
                            let result = browser_webview::navigate(&dispatch_app, session, &url);
                            if result.is_ok() && plan == AddressPlan::Fragment {
                                if let Some(committed) = state
                                    .committed_urls
                                    .lock()
                                    .unwrap_or_else(|error| error.into_inner())
                                    .get_mut(&dispatch_session)
                                {
                                    *committed = Some(url);
                                }
                            }
                            result
                        }
                    }
                    NativeNavigation::Back => browser_webview::back(&dispatch_app, session),
                    NativeNavigation::Forward => browser_webview::forward(&dispatch_app, session),
                    NativeNavigation::Reload => {
                        state.begin_load(
                            &dispatch_app,
                            &dispatch_session,
                            generation,
                            current.unwrap_or_default(),
                        );
                        browser_webview::reload(&dispatch_app, session)
                    }
                };
                result.map_err(|error| {
                    state.cancel(&dispatch_session);
                    let _ = dispatch_app.emit("plugin-browser-error", error.clone());
                    error.message
                })
            },
        )
        .unwrap_or(Ok(()))
    })
    .await;
    match result {
        Ok(result) => result,
        Err(error) => with_current_action(app, &host, &session_id, generation, &ticket, || {
            app.state::<LoadDeadlines>().cancel(&session_id);
            emit_browser_error(
                app,
                &session_id,
                generation,
                &PluginError::PluginUnavailable,
            );
            Err(error)
        })
        .unwrap_or(Ok(())),
    }
}

#[derive(Debug, PartialEq, Eq)]
enum AddressPlan {
    Reload,
    Fragment,
    Navigate,
}

fn address_plan(committed: Option<&str>, live: Option<&str>, target: &str) -> AddressPlan {
    let (Some(mut committed), Some(mut live), Ok(mut target)) = (
        committed.and_then(|value| url::Url::parse(value).ok()),
        live.and_then(|value| url::Url::parse(value).ok()),
        url::Url::parse(target),
    ) else {
        return AddressPlan::Navigate;
    };
    let same_address = live == target;
    committed.set_fragment(None);
    live.set_fragment(None);
    target.set_fragment(None);
    // A polled URL can belong to an uncommitted provisional load. It only
    // supplies fragment state when its document URL matches a known commit.
    if committed != live || committed != target {
        return AddressPlan::Navigate;
    }
    if same_address {
        AddressPlan::Reload
    } else {
        AddressPlan::Fragment
    }
}

/// Emits the fixed-code `plugin-browser-error` event for a host-side
/// (plugin/network, not native-webview) failure. Silently skipped for
/// `StaleGeneration`/`NoSession`/`Install`/`Registry`/`UnknownPlugin`: those
/// mean the surface is already gone or never had a live generation to
/// attach, and aren't in the closed event `code` set.
fn emit_browser_error(app: &AppHandle, session_id: &str, generation: u64, error: &PluginError) {
    let Some(code) = browser_error_code(error) else {
        return;
    };
    let _ = app.emit(
        "plugin-browser-error",
        BrowserErrorPayload {
            session_id: session_id.to_string(),
            generation,
            code,
            message: fixed_message(error),
        },
    );
}

fn browser_error_code(error: &PluginError) -> Option<&'static str> {
    match error {
        PluginError::NavigationDenied { .. } => Some("navigation-denied"),
        PluginError::PluginTimeout => Some("plugin-timeout"),
        PluginError::PluginUnavailable => Some("plugin-unavailable"),
        PluginError::PluginProtocol(_) => Some("plugin-protocol-error"),
        PluginError::ContractMismatch(_) => Some("plugin-contract-mismatch"),
        PluginError::Unsupported => Some("unsupported-platform"),
        PluginError::StaleGeneration
        | PluginError::NoSession
        | PluginError::Install(_)
        | PluginError::Registry(_)
        | PluginError::UnknownPlugin => None,
    }
}

/// Traverses native history without assuming a full-document load will occur.
#[tauri::command]
pub async fn plugin_browser_back(
    app: AppHandle,
    host: State<'_, Arc<PluginHost>>,
    session_id: String,
) -> Result<(), String> {
    navigation_command(&app, &host, session_id, NativeNavigation::Back).await
}

/// Traverses forward using WebKit's history.
#[tauri::command]
pub async fn plugin_browser_forward(
    app: AppHandle,
    host: State<'_, Arc<PluginHost>>,
    session_id: String,
) -> Result<(), String> {
    navigation_command(&app, &host, session_id, NativeNavigation::Forward).await
}

/// Reloads the current document, including pre-commit failure detection.
#[tauri::command]
pub async fn plugin_browser_reload(
    app: AppHandle,
    host: State<'_, Arc<PluginHost>>,
    session_id: String,
) -> Result<(), String> {
    navigation_command(&app, &host, session_id, NativeNavigation::Reload).await
}

async fn navigation_command(
    app: &AppHandle,
    host: &Arc<PluginHost>,
    session_id: String,
    navigation: NativeNavigation,
) -> Result<(), String> {
    let generation = host
        .session_generation(&SessionId(session_id.clone()))
        .ok_or_else(|| fixed_message(&PluginError::NoSession).to_owned())?;
    let ticket = app.state::<LoadDeadlines>().actions.begin(&session_id);
    dispatch_navigation(
        app,
        Arc::clone(host),
        session_id,
        generation,
        ticket,
        navigation,
    )
    .await
}

#[tauri::command]
pub async fn plugin_browser_set_bounds(
    app: AppHandle,
    session_id: String,
    bounds: Bounds,
) -> Result<(), String> {
    let dispatch_app = app.clone();
    on_main_thread(&app, move || {
        browser_webview::set_bounds(&dispatch_app, SessionId(session_id), bounds)
    })
    .await?
    .map_err(|error| error.message)
}

#[tauri::command]
pub async fn plugin_browser_set_visible(
    app: AppHandle,
    session_id: String,
    visible: bool,
) -> Result<(), String> {
    let dispatch_app = app.clone();
    on_main_thread(&app, move || {
        browser_webview::set_visible(&dispatch_app, SessionId(session_id), visible)
    })
    .await?
    .map_err(|error| error.message)
}

#[tauri::command]
pub async fn plugin_browser_close(
    app: AppHandle,
    host: State<'_, Arc<PluginHost>>,
    deadlines: State<'_, LoadDeadlines>,
    session_id: String,
) -> Result<(), String> {
    deadlines.forget(&session_id);
    let close_app = app.clone();
    let close_session = SessionId(session_id.clone());
    let native = closed_surface_result(
        on_main_thread(&app, move || {
            browser_webview::close(&close_app, close_session)
        })
        .await,
    );
    let process = host
        .end_session(SessionId(session_id))
        .await
        .map_err(|error| fixed_message(&error).to_owned());
    native.and(process)
}

/// Maps a [`PluginError`] to the fixed user-facing sentence for its closed
/// error code. Never includes plugin- or filesystem-supplied text.
fn fixed_message(error: &PluginError) -> &'static str {
    match error {
        PluginError::Install(_) => "The plugin could not be installed.",
        PluginError::NavigationDenied { .. } => "That address isn't allowed.",
        PluginError::PluginTimeout => "The plugin didn't respond in time.",
        PluginError::PluginUnavailable => "The plugin is unavailable.",
        PluginError::PluginProtocol(_) => "The plugin returned an invalid response.",
        PluginError::ContractMismatch(_) => {
            "The plugin's response didn't match the expected contract."
        }
        PluginError::StaleGeneration => "That session is no longer active.",
        PluginError::NoSession => "That browser session no longer exists.",
        PluginError::Unsupported => "This feature isn't available on your platform.",
        PluginError::Registry(_) => "The plugin registry could not be read.",
        PluginError::UnknownPlugin => "No installed plugin has that id.",
    }
}
