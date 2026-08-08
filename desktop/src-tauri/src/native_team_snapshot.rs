use std::{
    collections::VecDeque,
    fs::File,
    io::Read,
    path::{Path, PathBuf},
    sync::Mutex,
};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, Runtime, State};
use url::Url;

use crate::commands::team_snapshot::MAX_TEAM_SNAPSHOT_JSON_BYTES;

pub(crate) const NATIVE_TEAM_SNAPSHOT_OPENED_EVENT: &str = "native-team-snapshot-opened";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PendingNativeTeamSnapshot {
    id: String,
    file_name: String,
    error: Option<String>,
}

#[derive(Debug, Clone)]
struct PendingNativeTeamSnapshotEntry {
    id: String,
    path: Option<PathBuf>,
    file_name: String,
    error: Option<String>,
}

#[derive(Default)]
pub(crate) struct PendingNativeTeamSnapshots(Mutex<VecDeque<PendingNativeTeamSnapshotEntry>>);

impl PendingNativeTeamSnapshots {
    fn enqueue(&self, pending: PendingNativeTeamSnapshotEntry) {
        let mut queue = self.0.lock().expect("native team snapshot queue poisoned");
        if queue.iter().any(|entry| {
            entry.path == pending.path
                && entry.file_name == pending.file_name
                && entry.error == pending.error
        }) {
            return;
        }
        queue.push_back(pending);
    }

    fn first(&self) -> Option<PendingNativeTeamSnapshot> {
        self.0
            .lock()
            .expect("native team snapshot queue poisoned")
            .front()
            .map(|entry| PendingNativeTeamSnapshot {
                id: entry.id.clone(),
                file_name: entry.file_name.clone(),
                error: entry.error.clone(),
            })
    }

    fn read(&self, id: &str) -> Result<(String, Vec<u8>), String> {
        let queue = self.0.lock().expect("native team snapshot queue poisoned");
        let entry = queue
            .front()
            .filter(|entry| entry.id == id)
            .ok_or_else(|| "Unknown or no-longer-pending team snapshot request.".to_string())?;
        if let Some(error) = &entry.error {
            return Err(error.clone());
        }
        let path = entry
            .path
            .as_deref()
            .ok_or_else(|| "Opened team snapshot has no readable file path.".to_string())?;
        read_buzzteam(path, &entry.file_name)
    }

    fn acknowledge(&self, id: &str) -> bool {
        let mut queue = self.0.lock().expect("native team snapshot queue poisoned");
        if queue.front().is_some_and(|entry| entry.id == id) {
            queue.pop_front();
            true
        } else {
            false
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NativeTeamSnapshotBytes {
    file_name: String,
    file_bytes: Vec<u8>,
}

fn is_buzzteam_name(file_name: &str) -> bool {
    file_name.to_ascii_lowercase().ends_with(".buzzteam")
}

fn read_buzzteam(path: &Path, file_name: &str) -> Result<(String, Vec<u8>), String> {
    if !is_buzzteam_name(file_name) {
        return Err("Opened file is not a .buzzteam snapshot.".to_string());
    }
    let metadata = std::fs::metadata(path)
        .map_err(|error| format!("Failed to inspect opened team snapshot: {error}"))?;
    if !metadata.is_file() {
        return Err("Opened team snapshot is not a regular file.".to_string());
    }
    if metadata.len() > (MAX_TEAM_SNAPSHOT_JSON_BYTES as u64) {
        return Err("Opened team snapshot exceeds the 25 MiB limit.".to_string());
    }

    let mut file = File::open(path)
        .map_err(|error| format!("Failed to read opened team snapshot: {error}"))?;
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.by_ref()
        .take((MAX_TEAM_SNAPSHOT_JSON_BYTES as u64) + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("Failed to read opened team snapshot: {error}"))?;
    if bytes.len() > MAX_TEAM_SNAPSHOT_JSON_BYTES {
        return Err("Opened team snapshot exceeds the 25 MiB limit.".to_string());
    }
    Ok((file_name.to_owned(), bytes))
}

fn queue_opened_file(
    app: &AppHandle,
    path: Option<PathBuf>,
    file_name: String,
    error: Option<String>,
) {
    let pending = PendingNativeTeamSnapshotEntry {
        id: uuid::Uuid::new_v4().to_string(),
        path,
        file_name,
        error,
    };
    app.state::<PendingNativeTeamSnapshots>().enqueue(pending);
    focus_main_window(app);
    let _ = app.emit(NATIVE_TEAM_SNAPSHOT_OPENED_EVENT, ());
}

pub(crate) fn focus_main_window<R: Runtime>(app: &AppHandle<R>) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    if let Err(error) = window.unminimize() {
        eprintln!("buzz-desktop: failed to unminimize main window: {error}");
    }
    if let Err(error) = window.show() {
        eprintln!("buzz-desktop: failed to show main window: {error}");
    }
    if let Err(error) = window.set_focus() {
        eprintln!("buzz-desktop: failed to focus main window: {error}");
    }
}

pub(crate) fn handle_native_team_snapshot_url(app: &AppHandle, url: &Url) {
    match url.to_file_path() {
        Ok(path) => handle_native_team_snapshot_path(app, path),
        Err(()) => queue_opened_file(
            app,
            None,
            "opened-file".to_string(),
            Some("Opened team snapshot URL is not a local file.".to_string()),
        ),
    }
}

pub(crate) fn handle_native_team_snapshot_path(app: &AppHandle, path: PathBuf) {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("opened-file")
        .to_owned();
    if !is_buzzteam_name(&file_name) {
        queue_opened_file(
            app,
            None,
            file_name,
            Some("Opened file is not a .buzzteam snapshot.".to_string()),
        );
        return;
    }
    queue_opened_file(app, Some(path), file_name, None);
}

fn buzzteam_path_from_arg(arg: &str, cwd: &str) -> Option<PathBuf> {
    let path = Path::new(arg);
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        Path::new(cwd).join(path)
    };
    let is_buzzteam = path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(is_buzzteam_name);
    is_buzzteam.then_some(path)
}

fn startup_buzzteam_paths<I, S>(argv: I, cwd: &str) -> Vec<PathBuf>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    argv.into_iter()
        .skip(1)
        .filter_map(|arg| buzzteam_path_from_arg(arg.as_ref(), cwd))
        .collect()
}

pub(crate) fn enqueue_startup_buzzteam_paths(app: &AppHandle, argv: &[String], cwd: &str) {
    for path in startup_buzzteam_paths(argv, cwd) {
        handle_native_team_snapshot_path(app, path);
    }
}

pub(crate) fn handle_single_instance_args(app: &AppHandle, argv: &[String], cwd: &str) {
    for arg in argv {
        if arg.starts_with("buzz://") {
            crate::deep_link::handle_deep_link_url(app, arg);
            continue;
        }
        if let Some(path) = buzzteam_path_from_arg(arg, cwd) {
            handle_native_team_snapshot_path(app, path);
        }
    }
}

pub(crate) fn handle_opened_urls(app: &AppHandle, urls: Vec<Url>) {
    for url in urls {
        handle_native_team_snapshot_url(app, &url);
    }
}

#[tauri::command]
pub(crate) fn take_pending_native_team_snapshot(
    pending: State<'_, PendingNativeTeamSnapshots>,
) -> Option<PendingNativeTeamSnapshot> {
    pending.first()
}

#[tauri::command]
pub(crate) fn read_pending_native_team_snapshot(
    id: String,
    pending: State<'_, PendingNativeTeamSnapshots>,
) -> Result<NativeTeamSnapshotBytes, String> {
    let (file_name, file_bytes) = pending.read(&id)?;
    Ok(NativeTeamSnapshotBytes {
        file_name,
        file_bytes,
    })
}

#[tauri::command]
pub(crate) fn acknowledge_pending_native_team_snapshot(
    id: String,
    pending: State<'_, PendingNativeTeamSnapshots>,
) -> bool {
    pending.acknowledge(&id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buzzteam_name_is_required() {
        assert!(is_buzzteam_name("review.buzzteam"));
        assert!(is_buzzteam_name("REVIEW.BUZZTEAM"));
        assert!(!is_buzzteam_name("review.team.json"));
    }

    #[test]
    fn pending_snapshots_are_fifo_and_acknowledged_in_order() {
        let queue = PendingNativeTeamSnapshots::default();
        queue.enqueue(PendingNativeTeamSnapshotEntry {
            id: "one".to_owned(),
            path: None,
            file_name: "one.buzzteam".to_owned(),
            error: None,
        });
        queue.enqueue(PendingNativeTeamSnapshotEntry {
            id: "two".to_owned(),
            path: None,
            file_name: "two.buzzteam".to_owned(),
            error: None,
        });
        assert_eq!(queue.first().unwrap().id, "one");
        assert!(!queue.acknowledge("two"));
        assert!(queue.acknowledge("one"));
        assert_eq!(queue.first().unwrap().id, "two");
    }

    #[test]
    fn unknown_id_does_not_read_or_acknowledge_a_pending_path() {
        let queue = PendingNativeTeamSnapshots::default();
        queue.enqueue(PendingNativeTeamSnapshotEntry {
            id: "one".to_owned(),
            path: Some(PathBuf::from("/does/not/exist.buzzteam")),
            file_name: "one.buzzteam".to_owned(),
            error: None,
        });
        assert!(queue.read("unknown").is_err());
        assert!(!queue.acknowledge("unknown"));
        assert_eq!(queue.first().unwrap().id, "one");
    }

    #[test]
    fn relative_single_instance_buzzteam_path_resolves_against_callback_cwd() {
        assert_eq!(
            buzzteam_path_from_arg("imports/review.buzzteam", "/tmp/caller"),
            Some(PathBuf::from("/tmp/caller/imports/review.buzzteam"))
        );
        assert_eq!(buzzteam_path_from_arg("--flag", "/tmp/caller"), None);
    }

    #[test]
    fn startup_buzzteam_paths_skips_the_executable_and_uses_the_launch_cwd() {
        assert_eq!(
            startup_buzzteam_paths(
                [
                    "/Applications/Buzz.app/Contents/MacOS/buzz-desktop",
                    "imports/review.buzzteam",
                    "--safe-mode",
                    "/tmp/second.BUZZTEAM",
                ],
                "/tmp/caller",
            ),
            vec![
                PathBuf::from("/tmp/caller/imports/review.buzzteam"),
                PathBuf::from("/tmp/second.BUZZTEAM"),
            ],
        );
    }
}
