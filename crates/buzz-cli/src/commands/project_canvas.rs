use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    time::Duration,
};

use serde::{Deserialize, Serialize};

use crate::{error::CliError, CanvasChange};

const INDEX_FORMAT: &str = "buzz-project-canvas-index";
const INDEX_VERSION: u32 = 1;
const IPC_FORMAT: &str = "buzz-project-canvas-update";
const IPC_VERSION: u32 = 1;
const MAX_INDEX_BYTES: u64 = 1024 * 1024;
const MAX_INDEX_ENTRIES: usize = 4_096;
const MAX_RESPONSE_BYTES: u64 = 16 * 1024;
const SOCKET_FILE: &str = "agent-updates.sock";

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CanvasIndex {
    format: String,
    version: u32,
    canvases: Vec<CanvasIndexEntry>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CanvasIndexEntry {
    community_id: String,
    project_id: String,
    source_path: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CanvasUpdateRequest<'a> {
    format: &'static str,
    version: u32,
    notification_id: String,
    community_id: &'a str,
    project_id: &'a str,
    widget_id: &'a str,
    change: &'a CanvasChange,
}

#[derive(Deserialize)]
struct CanvasUpdateResponse {
    accepted: bool,
    message: String,
    #[serde(flatten)]
    output: serde_json::Map<String, serde_json::Value>,
}

struct ResolvedCanvas {
    canvas_root: PathBuf,
    community_id: String,
    project_id: String,
    source_path: PathBuf,
}

pub fn cmd_notify(source: &Path, widget: &str, change: &CanvasChange) -> Result<(), CliError> {
    validate_widget_id(widget)?;
    let resolved = resolve_canvas(source)?;
    notify_desktop(&resolved, widget, change)
}

fn validate_widget_id(widget: &str) -> Result<(), CliError> {
    if widget.is_empty()
        || widget.len() > 128
        || !widget
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(CliError::Usage(
            "--widget must be 1 to 128 ASCII letters, numbers, '.', '-', or '_'".into(),
        ));
    }
    Ok(())
}

fn resolve_canvas(source: &Path) -> Result<ResolvedCanvas, CliError> {
    let source_path = source
        .canonicalize()
        .map_err(|error| CliError::Usage(format!("resolve Canvas source: {error}")))?;
    if !source_path.is_dir() {
        return Err(CliError::Usage(
            "--source must identify a Canvas package directory".into(),
        ));
    }
    let canvas_root = source_path
        .ancestors()
        .find(|candidate| {
            candidate.file_name().and_then(|name| name.to_str()) == Some("CANVASES")
                && candidate.join("index.json").is_file()
        })
        .map(Path::to_path_buf)
        .ok_or_else(|| {
            CliError::Usage(
                "--source is not listed below a Buzz CANVASES directory with index.json".into(),
            )
        })?;
    let index_path = canvas_root.join("index.json");
    let metadata = fs::metadata(&index_path)
        .map_err(|error| CliError::Usage(format!("inspect Canvas index: {error}")))?;
    if metadata.len() > MAX_INDEX_BYTES {
        return Err(CliError::Usage("Canvas index exceeds 1 MiB".into()));
    }
    let mut raw = Vec::new();
    fs::File::open(&index_path)
        .and_then(|file| {
            file.take(MAX_INDEX_BYTES + 1).read_to_end(&mut raw)?;
            Ok(())
        })
        .map_err(|error| CliError::Usage(format!("read Canvas index: {error}")))?;
    if raw.len() as u64 > MAX_INDEX_BYTES {
        return Err(CliError::Usage("Canvas index exceeds 1 MiB".into()));
    }
    let index: CanvasIndex = serde_json::from_slice(&raw)
        .map_err(|error| CliError::Usage(format!("invalid Canvas index: {error}")))?;
    if index.format != INDEX_FORMAT || index.version != INDEX_VERSION {
        return Err(CliError::Usage(
            "unsupported project Canvas index format".into(),
        ));
    }
    if index.canvases.len() > MAX_INDEX_ENTRIES {
        return Err(CliError::Usage("Canvas index exceeds 4096 entries".into()));
    }

    let mut matched = None;
    for entry in index.canvases {
        let Ok(indexed_source) = PathBuf::from(&entry.source_path).canonicalize() else {
            continue;
        };
        if indexed_source == source_path {
            if matched.is_some() {
                return Err(CliError::Usage(
                    "Canvas index contains duplicate entries for --source".into(),
                ));
            }
            matched = Some((entry.community_id, entry.project_id));
        }
    }
    let (community_id, project_id) = matched.ok_or_else(|| {
        CliError::NotFound("Canvas source is not present in CANVASES/index.json".into())
    })?;
    Ok(ResolvedCanvas {
        canvas_root,
        community_id,
        project_id,
        source_path,
    })
}

#[cfg(unix)]
fn notify_desktop(
    resolved: &ResolvedCanvas,
    widget: &str,
    change: &CanvasChange,
) -> Result<(), CliError> {
    use std::os::unix::net::UnixStream;

    let socket = resolved.canvas_root.join(".runtime").join(SOCKET_FILE);
    let mut stream = UnixStream::connect(&socket).map_err(|error| {
        CliError::Other(format!(
            "connect to Buzz Desktop Canvas update socket {}: {error}; make sure Buzz Desktop is running",
            socket.display()
        ))
    })?;
    let timeout = Some(Duration::from_secs(5));
    stream
        .set_read_timeout(timeout)
        .map_err(|error| CliError::Other(format!("configure Canvas update response: {error}")))?;
    stream
        .set_write_timeout(timeout)
        .map_err(|error| CliError::Other(format!("configure Canvas update request: {error}")))?;

    let request = CanvasUpdateRequest {
        format: IPC_FORMAT,
        version: IPC_VERSION,
        notification_id: uuid::Uuid::new_v4().simple().to_string(),
        community_id: &resolved.community_id,
        project_id: &resolved.project_id,
        widget_id: widget,
        change,
    };
    serde_json::to_writer(&mut stream, &request)
        .map_err(|error| CliError::Other(format!("encode Canvas update request: {error}")))?;
    stream
        .write_all(b"\n")
        .and_then(|()| stream.flush())
        .map_err(|error| CliError::Other(format!("send Canvas update request: {error}")))?;

    let mut raw = Vec::new();
    stream
        .take(MAX_RESPONSE_BYTES + 1)
        .read_to_end(&mut raw)
        .map_err(|error| CliError::Other(format!("read Canvas update response: {error}")))?;
    if raw.len() as u64 > MAX_RESPONSE_BYTES {
        return Err(CliError::Other(
            "Buzz Desktop Canvas update response exceeds 16 KiB".into(),
        ));
    }
    let response: CanvasUpdateResponse = serde_json::from_slice(&raw)
        .map_err(|error| CliError::Other(format!("invalid Canvas update response: {error}")))?;
    if !response.accepted {
        return Err(CliError::Usage(response.message));
    }
    let mut output = response.output;
    output.insert("accepted".into(), serde_json::Value::Bool(true));
    output.insert("message".into(), response.message.into());
    output.insert(
        "sourcePath".into(),
        resolved.source_path.to_string_lossy().into_owned().into(),
    );
    println!("{}", serde_json::Value::Object(output));
    Ok(())
}

#[cfg(not(unix))]
fn notify_desktop(
    _resolved: &ResolvedCanvas,
    _widget: &str,
    _change: &CanvasChange,
) -> Result<(), CliError> {
    Err(CliError::Other(
        "sandboxed project Canvas updates are currently supported on macOS only".into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    const OWNER: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn indexed_canvas(temp: &tempfile::TempDir) -> PathBuf {
        let root = temp.path().join("CANVASES");
        let source = root.join("community").join(OWNER).join("project");
        fs::create_dir_all(&source).unwrap();
        fs::write(
            root.join("index.json"),
            serde_json::json!({
                "format": INDEX_FORMAT,
                "version": INDEX_VERSION,
                "canvases": [{
                    "communityId": "community-id",
                    "projectId": format!("30621:{OWNER}:project"),
                    "sourcePath": source,
                }],
            })
            .to_string(),
        )
        .unwrap();
        source
    }

    #[test]
    fn resolves_binding_from_the_index_for_an_exact_source_path() {
        let temp = tempfile::TempDir::new().unwrap();
        let source = indexed_canvas(&temp);
        let resolved = resolve_canvas(&source).unwrap();
        assert_eq!(resolved.community_id, "community-id");
        assert_eq!(resolved.project_id, format!("30621:{OWNER}:project"));
        assert_eq!(
            resolved.canvas_root,
            temp.path().join("CANVASES").canonicalize().unwrap()
        );
    }

    #[test]
    fn rejects_unindexed_and_invalid_widget_inputs() {
        let temp = tempfile::TempDir::new().unwrap();
        let source = indexed_canvas(&temp);
        let other = temp.path().join("CANVASES").join("other");
        fs::create_dir(&other).unwrap();
        assert!(matches!(resolve_canvas(&other), Err(CliError::NotFound(_))));
        assert!(matches!(
            validate_widget_id("bad/widget"),
            Err(CliError::Usage(_))
        ));
        assert!(resolve_canvas(&source.join("missing")).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn sends_a_bounded_local_update_request_and_accepts_the_desktop_response() {
        use std::os::unix::net::UnixListener;
        use std::thread;

        let temp = tempfile::Builder::new()
            .prefix("buzz-canvas")
            .tempdir_in("/tmp")
            .unwrap();
        let source = indexed_canvas(&temp);
        let runtime = temp.path().join("CANVASES").join(".runtime");
        fs::create_dir(&runtime).unwrap();
        let listener = UnixListener::bind(runtime.join(SOCKET_FILE)).unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut raw = Vec::new();
            loop {
                let mut byte = [0_u8; 1];
                stream.read_exact(&mut byte).unwrap();
                raw.push(byte[0]);
                if byte[0] == b'\n' {
                    break;
                }
            }
            let request: serde_json::Value = serde_json::from_slice(&raw).unwrap();
            assert_eq!(request["format"], IPC_FORMAT);
            assert_eq!(request["version"], IPC_VERSION);
            assert_eq!(request["communityId"], "community-id");
            assert_eq!(request["widgetId"], "chore-board");
            assert_eq!(request["change"], "data");
            stream
                .write_all(
                    serde_json::json!({
                        "accepted": true,
                        "change": "data",
                        "message": "Canvas update delivered",
                        "notificationId": request["notificationId"],
                        "projectId": request["projectId"],
                        "revision": "a".repeat(64),
                        "widgetId": request["widgetId"],
                    })
                    .to_string()
                    .as_bytes(),
                )
                .unwrap();
        });

        cmd_notify(&source, "chore-board", &CanvasChange::Data).unwrap();
        server.join().unwrap();
    }
}
