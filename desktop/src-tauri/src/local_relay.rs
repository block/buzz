//! Lifecycle management for the embedded single-node relay.
//!
//! The relay remains the source of truth for local-mode behavior. Desktop only
//! chooses private loopback ports, supplies its identity and durable paths, and
//! supervises the bundled `buzz-relay` binary.

use std::{
    io::Read,
    net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use nostr::Keys;
use tauri::{AppHandle, Manager};

const STARTUP_TIMEOUT: Duration = Duration::from_secs(15);
const POLL_INTERVAL: Duration = Duration::from_millis(100);
const STDERR_TAIL_BYTES: usize = 8 * 1024;

type StderrTail = Arc<Mutex<Vec<u8>>>;

pub(crate) struct LocalRelayRuntime {
    child: Child,
    url: String,
    owner_pubkey: String,
}

pub(crate) type RuntimeState = Mutex<Option<LocalRelayRuntime>>;

#[derive(Debug, PartialEq, Eq)]
struct LocalRelayConfig {
    relay_addr: SocketAddr,
    health_port: u16,
    metrics_port: u16,
    data_dir: PathBuf,
    owner_pubkey: String,
    relay_private_key: String,
}

impl LocalRelayConfig {
    fn url(&self) -> String {
        format!("ws://{}", self.relay_addr)
    }

    fn environment(&self) -> Vec<(&'static str, String)> {
        let db_path = self.data_dir.join("relay.sqlite3");
        let media_dir = self.data_dir.join("media");
        vec![
            ("BUZZ_PROFILE", "single-node".to_string()),
            ("BUZZ_BIND_ADDR", self.relay_addr.to_string()),
            ("BUZZ_HEALTH_PORT", self.health_port.to_string()),
            ("BUZZ_METRICS_PORT", self.metrics_port.to_string()),
            ("RELAY_URL", self.url()),
            ("BUZZ_LOCAL_DB", sqlite_url(&db_path)),
            ("BUZZ_LOCAL_MEDIA_DIR", media_dir.display().to_string()),
            ("BUZZ_REQUIRE_RELAY_MEMBERSHIP", "true".to_string()),
            ("BUZZ_ALLOW_NIP_OA_AUTH", "true".to_string()),
            ("RELAY_OWNER_PUBKEY", self.owner_pubkey.clone()),
            ("BUZZ_RELAY_PRIVATE_KEY", self.relay_private_key.clone()),
        ]
    }
}

impl LocalRelayRuntime {
    pub(crate) fn url(&self) -> &str {
        &self.url
    }

    fn is_running(&mut self) -> Result<bool, String> {
        self.child
            .try_wait()
            .map(|status| status.is_none())
            .map_err(|error| format!("inspect local relay: {error}"))
    }

    fn stop(&mut self) {
        // The relay handles SIGTERM with a WebSocket drain. It is our child, so
        // waiting here prevents a restart from inheriting a stale listener.
        #[cfg(unix)]
        unsafe {
            libc::kill(self.child.id() as i32, libc::SIGTERM);
        }
        #[cfg(windows)]
        let _ = self.child.kill();

        let deadline = Instant::now() + Duration::from_secs(8);
        while Instant::now() < deadline {
            if self.child.try_wait().ok().flatten().is_some() {
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Starts a relay for this desktop identity and waits for the public relay-info
/// endpoint. `Db::new_sqlite` in the relay applies its versioned SQLite
/// migrations before this endpoint can become available.
pub(crate) fn start(
    app: &AppHandle,
    keys: &Keys,
    requested_port: Option<u16>,
) -> Result<LocalRelayRuntime, String> {
    let owner_pubkey = keys.public_key().to_hex();
    let data_dir = local_data_dir(app, &owner_pubkey)?;
    let relay_keys = load_or_create_relay_keys(&data_dir)?;
    let media_dir = data_dir.join("media");
    std::fs::create_dir_all(&media_dir)
        .map_err(|e| format!("create local relay media dir: {e}"))?;
    let binary = relay_binary()?;

    // The relay owns its TCP listeners, so the desktop cannot safely retain a
    // reservation across `spawn`. Retry a complete, distinct three-port set
    // when an automatically-selected port loses that race; explicit user ports
    // fail with the child diagnostic instead of silently moving the endpoint.
    let attempts = if requested_port.is_some() { 1 } else { 3 };
    let mut last_error = None;
    for _ in 0..attempts {
        let (relay_port, health_port, metrics_port) =
            allocate_distinct_loopback_ports(requested_port)?;
        let config = LocalRelayConfig {
            relay_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), relay_port),
            health_port,
            metrics_port,
            data_dir: data_dir.clone(),
            owner_pubkey: owner_pubkey.clone(),
            relay_private_key: relay_keys.secret_key().to_secret_hex(),
        };

        match spawn_and_wait(&binary, config) {
            Ok(runtime) => return Ok(runtime),
            Err(error) if requested_port.is_none() && is_bind_conflict(&error) => {
                last_error = Some(error);
            }
            Err(error) => return Err(error),
        }
    }
    Err(last_error.expect("at least one local relay startup attempt"))
}

pub(crate) fn ensure_started(
    app: &AppHandle,
    runtime: &RuntimeState,
    keys: &Keys,
    requested_port: Option<u16>,
) -> Result<String, String> {
    let mut runtime = runtime.lock().map_err(|error| error.to_string())?;
    if let Some(running) = runtime.as_mut() {
        let same_port = requested_port.is_none_or(|port| running.url == local_relay_url(port));
        let same_owner = running.owner_pubkey == keys.public_key().to_hex();
        let is_running = running.is_running()?;
        if same_port && same_owner && is_running {
            return Ok(running.url().to_string());
        }
        if !same_owner {
            // An onboarding identity import changed the owner. Its local relay
            // must use a separate identity-scoped nest, not retain the prior
            // identity's private sidecar. `try_wait` above already reaped an
            // exited child, so never signal its potentially recycled PID.
            if let Some(mut previous) = runtime.take() {
                if is_running {
                    previous.stop();
                }
            }
        } else if !is_running {
            // `try_wait` reaped the crashed child. Drop its stale handle so the
            // replacement below receives a fresh loopback port and child.
            runtime.take();
        } else {
            return Err("a different local relay port is already running".to_string());
        }
    }
    let started = start(app, keys, requested_port)?;
    let url = started.url().to_string();
    *runtime = Some(started);
    Ok(url)
}

pub(crate) fn stop(runtime: &RuntimeState) {
    if let Ok(mut runtime) = runtime.lock() {
        if let Some(mut runtime) = runtime.take() {
            runtime.stop();
        }
    }
}

/// The persisted sentinel for the desktop-owned local workspace. It is never a
/// network endpoint: `apply_workspace` resolves it to the current supervised
/// loopback URL before any client or managed agent is restored.
pub(crate) const LOCAL_RELAY_SENTINEL: &str = "buzz-local://on-this-device";

pub(crate) fn is_local_relay_url(url: &str) -> bool {
    url == LOCAL_RELAY_SENTINEL
}

fn local_relay_url(port: u16) -> String {
    format!("ws://127.0.0.1:{port}")
}

pub(crate) fn relay_url(runtime: &RuntimeState) -> Option<String> {
    runtime
        .lock()
        .ok()
        .and_then(|runtime| runtime.as_ref().map(|runtime| runtime.url().to_string()))
}

fn local_data_dir(app: &AppHandle, owner_pubkey: &str) -> Result<PathBuf, String> {
    // Relay state is both durable and identity-scoped. A different desktop
    // identity must never inherit this identity's SQLite membership or media.
    app.path()
        .app_data_dir()
        .map(|path| path.join("local-relay").join(owner_pubkey))
        .map_err(|e| format!("resolve app data directory for local relay: {e}"))
}

fn sqlite_url(path: &Path) -> String {
    // SQLite URLs use forward slashes so this contract is canonical across platforms.
    format!("sqlite://{}", path.display().to_string().replace('\\', "/"))
}

/// Load the relay's service identity from its identity-scoped nest, or create
/// it once with the same atomic, owner-only file semantics used for private
/// desktop identities. This key signs relay-originated metadata; it is never
/// the human/owner key supplied by `apply_workspace`.
fn load_or_create_relay_keys(data_dir: &Path) -> Result<Keys, String> {
    let path = data_dir.join("relay-service.key");
    if path.exists() {
        let value = std::fs::read_to_string(&path)
            .map_err(|e| format!("read local relay service key: {e}"))?;
        return Keys::parse(value.trim())
            .map_err(|e| format!("parse local relay service key: {e}"));
    }

    std::fs::create_dir_all(data_dir).map_err(|e| format!("create local relay data dir: {e}"))?;
    let keys = Keys::generate();
    crate::app_state::save_key_file(&path, &keys)
        .map_err(|e| format!("persist local relay service key: {e}"))?;
    Ok(keys)
}

fn is_bind_conflict(error: &str) -> bool {
    error.contains("Address already in use") || error.contains("address already in use")
}

fn spawn_and_wait(binary: &Path, config: LocalRelayConfig) -> Result<LocalRelayRuntime, String> {
    let url = config.url();
    let info_url = format!("http://{}/info", config.relay_addr);
    let stderr = Arc::new(Mutex::new(Vec::new()));
    let mut command = Command::new(binary);
    command
        .envs(config.environment())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .map_err(|e| format!("start bundled buzz-relay at {}: {e}", binary.display()))?;
    let stderr_reader = capture_stderr(child.stderr.take(), stderr.clone());

    let client = reqwest::blocking::Client::new();
    let deadline = Instant::now() + STARTUP_TIMEOUT;
    while Instant::now() < deadline {
        if let Some(status) = child
            .try_wait()
            .map_err(|e| format!("inspect local relay: {e}"))?
        {
            let _ = stderr_reader.join();
            return Err(format!(
                "bundled buzz-relay exited before becoming ready ({status}){}",
                stderr_context(&stderr)
            ));
        }
        if client
            .get(&info_url)
            .header("Accept", "application/nostr+json")
            .send()
            .is_ok_and(|response| response.status().is_success())
        {
            return Ok(LocalRelayRuntime {
                child,
                url,
                owner_pubkey: config.owner_pubkey,
            });
        }
        std::thread::sleep(POLL_INTERVAL);
    }

    let _ = child.kill();
    let _ = child.wait();
    let _ = stderr_reader.join();
    Err(format!(
        "bundled buzz-relay did not become ready within 15 seconds{}",
        stderr_context(&stderr)
    ))
}

fn capture_stderr(
    stderr: Option<std::process::ChildStderr>,
    tail: StderrTail,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        if let Some(mut stderr) = stderr {
            let mut buffer = [0; 1024];
            while let Ok(read) = stderr.read(&mut buffer) {
                if read == 0 {
                    break;
                }
                let mut tail = tail.lock().expect("stderr tail mutex poisoned");
                tail.extend_from_slice(&buffer[..read]);
                if tail.len() > STDERR_TAIL_BYTES {
                    let excess = tail.len() - STDERR_TAIL_BYTES;
                    tail.drain(..excess);
                }
            }
        }
    })
}

fn stderr_context(stderr: &StderrTail) -> String {
    let tail = stderr.lock().expect("stderr tail mutex poisoned");
    let text = String::from_utf8_lossy(&tail).trim().to_string();
    if text.is_empty() {
        String::new()
    } else {
        format!("; stderr: {text}")
    }
}

fn allocate_distinct_loopback_ports(
    requested_port: Option<u16>,
) -> Result<(u16, u16, u16), String> {
    let relay_port = requested_port.unwrap_or(free_loopback_port()?);
    let mut ports = vec![relay_port];
    while ports.len() < 3 {
        let port = free_loopback_port()?;
        if !ports.contains(&port) {
            ports.push(port);
        }
    }
    Ok((ports[0], ports[1], ports[2]))
}

fn free_loopback_port() -> Result<u16, String> {
    let listener =
        TcpListener::bind("127.0.0.1:0").map_err(|e| format!("pick local relay port: {e}"))?;
    listener
        .local_addr()
        .map(|address| address.port())
        .map_err(|e| format!("read local relay port: {e}"))
}

fn relay_binary() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| format!("resolve desktop executable: {e}"))?;
    let parent = exe
        .parent()
        .ok_or("desktop executable has no parent directory")?;
    let name = if cfg!(windows) {
        "buzz-relay.exe"
    } else {
        "buzz-relay"
    };
    let binary = parent.join(name);
    if binary.is_file() {
        Ok(binary)
    } else {
        Err(format!(
            "bundled buzz-relay is missing at {}",
            binary.display()
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::{
        allocate_distinct_loopback_ports, is_local_relay_url, load_or_create_relay_keys,
        local_relay_url, sqlite_url, stderr_context, LocalRelayConfig, StderrTail,
        LOCAL_RELAY_SENTINEL,
    };
    use std::{
        collections::HashMap,
        net::{IpAddr, Ipv4Addr, SocketAddr},
        path::{Path, PathBuf},
        process::{Command, Stdio},
        sync::{Arc, Mutex},
    };

    #[test]
    fn selected_ports_are_distinct_in_one_startup_attempt() {
        let (relay, health, metrics) = allocate_distinct_loopback_ports(None).expect("ports");
        assert_ne!(relay, health);
        assert_ne!(relay, metrics);
        assert_ne!(health, metrics);
    }

    #[test]
    fn startup_error_includes_bounded_stderr_tail() {
        let tail: StderrTail = Arc::new(Mutex::new(b"bind: address already in use\n".to_vec()));
        assert_eq!(
            stderr_context(&tail),
            "; stderr: bind: address already in use"
        );
    }

    #[test]
    fn sqlite_url_is_absolute() {
        assert_eq!(
            sqlite_url(Path::new("/tmp/buzz.sqlite3")),
            "sqlite:///tmp/buzz.sqlite3"
        );
    }

    #[test]
    fn sqlite_url_normalizes_windows_separators() {
        assert_eq!(
            sqlite_url(Path::new(r"C:\tmp\buzz.sqlite3")),
            "sqlite://C:/tmp/buzz.sqlite3"
        );
    }

    #[test]
    fn local_relay_sentinel_is_the_only_managed_workspace_address() {
        assert_eq!(local_relay_url(4317), "ws://127.0.0.1:4317");
        assert!(is_local_relay_url(LOCAL_RELAY_SENTINEL));
        assert!(!is_local_relay_url("ws://127.0.0.1:4317"));
        assert!(!is_local_relay_url("ws://localhost:4317"));
        assert!(!is_local_relay_url("wss://127.0.0.1:4317"));
    }

    #[test]
    fn exited_relay_is_not_running() {
        let child = Command::new(std::env::current_exe().expect("test executable"))
            .arg("--help")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("start test child");
        let mut runtime = super::LocalRelayRuntime {
            child,
            url: "ws://127.0.0.1:4317".to_string(),
            owner_pubkey: "owner".to_string(),
        };

        runtime.child.wait().expect("wait for test child");
        assert!(!runtime.is_running().expect("inspect exited child"));
    }

    #[test]
    fn relay_service_key_is_durable_and_distinct_from_owner_identity() {
        let directory = tempfile::tempdir().expect("temp data dir");
        let owner = nostr::Keys::generate();
        let first = load_or_create_relay_keys(directory.path()).expect("create service key");
        let second = load_or_create_relay_keys(directory.path()).expect("reload service key");

        assert_ne!(first.public_key(), owner.public_key());
        assert_eq!(first.public_key(), second.public_key());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(directory.path().join("relay-service.key"))
                .expect("service key metadata")
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o600);
        }
    }

    #[test]
    fn local_relay_environment_uses_only_the_single_node_contract() {
        let config = LocalRelayConfig {
            relay_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 4317),
            health_port: 4318,
            metrics_port: 4319,
            data_dir: PathBuf::from("/tmp/buzz/local-relay"),
            owner_pubkey: "owner".to_string(),
            relay_private_key: "private".to_string(),
        };
        let env: HashMap<_, _> = config.environment().into_iter().collect();

        assert_eq!(env.get("BUZZ_PROFILE"), Some(&"single-node".to_string()));
        assert_eq!(
            env.get("BUZZ_BIND_ADDR"),
            Some(&"127.0.0.1:4317".to_string())
        );
        assert_eq!(
            env.get("RELAY_URL"),
            Some(&"ws://127.0.0.1:4317".to_string())
        );
        assert_eq!(
            env.get("BUZZ_LOCAL_DB"),
            Some(&"sqlite:///tmp/buzz/local-relay/relay.sqlite3".to_string())
        );
        assert_eq!(
            env.get("BUZZ_LOCAL_MEDIA_DIR"),
            Some(
                &PathBuf::from("/tmp/buzz/local-relay")
                    .join("media")
                    .display()
                    .to_string()
            )
        );
        assert_eq!(
            env.get("BUZZ_REQUIRE_RELAY_MEMBERSHIP"),
            Some(&"true".to_string())
        );
        assert_eq!(env.get("BUZZ_ALLOW_NIP_OA_AUTH"), Some(&"true".to_string()));
        assert_eq!(env.get("RELAY_OWNER_PUBKEY"), Some(&"owner".to_string()));
        assert_eq!(
            env.get("BUZZ_RELAY_PRIVATE_KEY"),
            Some(&"private".to_string())
        );
    }
}
