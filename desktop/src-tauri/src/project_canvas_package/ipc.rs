#[cfg(unix)]
use std::{
    fs,
    os::unix::{
        fs::{FileTypeExt, PermissionsExt},
        net::{UnixListener as StdUnixListener, UnixStream as StdUnixStream},
    },
    path::PathBuf,
    time::Duration,
};

#[cfg(unix)]
use tauri::{AppHandle, Emitter, Manager};

#[cfg(unix)]
use super::path_security::{canonical_canvas_root, ensure_secure_descendant};
use super::ProjectCanvasAgentUpdateRequest;

pub(super) const UPDATE_FORMAT: &str = "buzz-project-canvas-update";
pub(super) const UPDATE_VERSION: u32 = 1;
pub(crate) const UPDATE_EVENT: &str = "project-canvas-source-updated";

#[cfg(unix)]
const SOCKET_FILE: &str = "agent-updates.sock";
#[cfg(unix)]
const MAX_REQUEST_BYTES: usize = 16 * 1024;

#[cfg(unix)]
pub(super) fn start(app: AppHandle) -> Result<(), String> {
    let canvas_root = crate::managed_agents::nest_dir()
        .ok_or_else(|| "cannot resolve the nest directory for Canvas updates".to_string())?
        .join("CANVASES");
    let canvas_root = canonical_canvas_root(&canvas_root, true)?
        .ok_or_else(|| "project canvas root was not created".to_string())?;
    let runtime_root = canvas_root.join(".runtime");
    ensure_secure_descendant(&canvas_root, &runtime_root, true)?;
    let socket_path = runtime_root.join(SOCKET_FILE);
    if socket_path.exists() {
        let metadata = fs::symlink_metadata(&socket_path)
            .map_err(|error| format!("inspect project canvas update socket: {error}"))?;
        if !metadata.file_type().is_socket() {
            return Err("project canvas update socket path is not a socket".to_string());
        }
        if StdUnixStream::connect(&socket_path).is_ok() {
            return Err("project canvas update socket is already in use".to_string());
        }
        fs::remove_file(&socket_path)
            .map_err(|error| format!("remove stale project canvas update socket: {error}"))?;
    }

    let std_listener = StdUnixListener::bind(&socket_path)
        .map_err(|error| format!("bind project canvas update socket: {error}"))?;
    fs::set_permissions(&socket_path, fs::Permissions::from_mode(0o600))
        .map_err(|error| format!("secure project canvas update socket: {error}"))?;
    std_listener
        .set_nonblocking(true)
        .map_err(|error| format!("configure project canvas update socket: {error}"))?;
    let serve_path = socket_path.clone();
    spawn_serving(std_listener, socket_path, move |listener| {
        run(listener, app, serve_path)
    });
    Ok(())
}

/// Hand a bound socket to the async runtime and serve it there.
///
/// `tokio::net::UnixListener::from_std` registers the socket with the Tokio
/// reactor, so it only works from inside the runtime. `start` runs on the main
/// thread from Tauri's `setup` hook, which is not in runtime context — doing
/// the conversion there panics, and the panic crosses the non-unwinding
/// `did_finish_launching` boundary, aborting the app before it opens a window.
/// So the conversion has to happen in the spawned task, not at the call site.
#[cfg(unix)]
pub(super) fn spawn_serving<F, Fut>(std_listener: StdUnixListener, socket_path: PathBuf, serve: F)
where
    F: FnOnce(tokio::net::UnixListener) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = ()> + Send + 'static,
{
    tauri::async_runtime::spawn(async move {
        match tokio::net::UnixListener::from_std(std_listener) {
            Ok(listener) => serve(listener).await,
            Err(error) => {
                eprintln!("buzz-desktop: project Canvas update socket stopped: {error}");
                let _ = fs::remove_file(&socket_path);
            }
        }
    });
}

#[cfg(not(unix))]
pub(super) fn start(_app: tauri::AppHandle) -> Result<(), String> {
    Ok(())
}

#[cfg(unix)]
async fn run(listener: tokio::net::UnixListener, app: AppHandle, socket_path: PathBuf) {
    loop {
        let (stream, _) = match listener.accept().await {
            Ok(connection) => connection,
            Err(error) => {
                eprintln!("buzz-desktop: project Canvas update socket stopped: {error}");
                let _ = fs::remove_file(&socket_path);
                return;
            }
        };
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            if let Err(error) = handle_connection(stream, app).await {
                eprintln!("buzz-desktop: project Canvas update rejected: {error}");
            }
        });
    }
}

#[cfg(unix)]
async fn handle_connection(stream: tokio::net::UnixStream, app: AppHandle) -> Result<(), String> {
    use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

    let (read, mut write) = stream.into_split();
    let mut reader = BufReader::new(read).take((MAX_REQUEST_BYTES + 1) as u64);
    let mut raw = Vec::new();
    let read_result =
        tokio::time::timeout(Duration::from_secs(5), reader.read_until(b'\n', &mut raw)).await;
    let result = match read_result {
        Ok(Ok(0)) => Err("empty project canvas update request".to_string()),
        Ok(Ok(_)) if raw.len() > MAX_REQUEST_BYTES => {
            Err("project canvas update request exceeds 16 KiB".to_string())
        }
        Ok(Ok(_)) if raw.last() != Some(&b'\n') => {
            Err("incomplete project canvas update request".to_string())
        }
        Ok(Ok(_)) => {
            let request: Result<ProjectCanvasAgentUpdateRequest, _> = serde_json::from_slice(&raw);
            match request {
                Ok(request) => {
                    let runtime = app.state::<super::ProjectCanvasRuntime>().inner().clone();
                    match super::run_blocking(move || runtime.accept_agent_update(request)).await {
                        Ok(accepted) => app
                            .emit(
                                UPDATE_EVENT,
                                serde_json::json!({
                                    "communityId": accepted.community_id,
                                    "projectId": accepted.project_id,
                                }),
                            )
                            .map(|()| accepted)
                            .map_err(|error| format!("emit project canvas update: {error}")),
                        Err(error) => Err(error),
                    }
                }
                Err(error) => Err(format!("invalid project canvas update request: {error}")),
            }
        }
        Ok(Err(error)) => Err(format!("read project canvas update request: {error}")),
        Err(_) => Err("project canvas update request timed out".to_string()),
    };

    let response = match &result {
        Ok(accepted) => {
            let mut value = serde_json::to_value(accepted)
                .map_err(|error| format!("encode project canvas update response: {error}"))?;
            let object = value
                .as_object_mut()
                .ok_or_else(|| "invalid project canvas update response shape".to_string())?;
            object.insert("accepted".into(), true.into());
            object.insert("message".into(), "Canvas update delivered".into());
            value
        }
        Err(error) => serde_json::json!({
            "accepted": false,
            "message": error,
        }),
    };
    let mut bytes = serde_json::to_vec(&response)
        .map_err(|error| format!("encode project canvas update response: {error}"))?;
    bytes.push(b'\n');
    tokio::time::timeout(Duration::from_secs(5), write.write_all(&bytes))
        .await
        .map_err(|_| "project canvas update response timed out".to_string())?
        .map_err(|error| format!("write project canvas update response: {error}"))?;
    result.map(|_| ())
}
