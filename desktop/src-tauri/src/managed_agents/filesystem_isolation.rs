//! Per-run filesystem isolation for local managed-agent process trees.
//!
//! macOS Seatbelt is applied to the outer `buzz-acp` process, so every agent,
//! MCP server, shell, and background descendant inherits the same boundary.
//! The run root is fresh for every spawn and removed only after the process
//! tree has exited and the runtime entry is dropped.

use std::{
    collections::HashMap,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::Command,
    sync::{Mutex, OnceLock},
    thread,
    time::Duration,
};

#[cfg(target_os = "macos")]
use std::os::unix::{
    fs::PermissionsExt as _,
    net::{UnixListener, UnixStream},
};

use serde::{Deserialize, Serialize};

use super::FilesystemIsolationProfile;

pub const ISOLATION_ATTESTATION_ENV: &str = "BUZZ_FILESYSTEM_ISOLATION_ATTESTATION";
pub const ISOLATION_RUN_ROOT_ENV: &str = "BUZZ_FILESYSTEM_ISOLATION_RUN_ROOT";

const RUNS_DIR: &str = "buzz-agent-runs";
const RECEIPTS_DIR: &str = ".receipts";
const CONTROL_SOCKET: &str = "/private/tmp/buzz-isolation-control-v1.sock";

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct FilesystemIsolationAttestation {
    pub version: u8,
    pub enforcement: &'static str,
    pub identity_pubkey: String,
    pub run_id: String,
    pub run_root: PathBuf,
    pub allowed_read_roots: Vec<PathBuf>,
    pub allowed_write_roots: Vec<PathBuf>,
    pub denied_roots: Vec<PathBuf>,
}

#[derive(Debug)]
pub struct FilesystemIsolationRun {
    root: PathBuf,
    base: PathBuf,
    ownership_path: PathBuf,
    ownership: IsolationRunOwnership,
    control: IsolationControlPlane,
    #[allow(dead_code)] // exposed through the Desktop registry; inspected directly in tests
    pub attestation: FilesystemIsolationAttestation,
}

impl FilesystemIsolationRun {
    #[cfg(test)]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Bind the immutable control-plane receipt and durable ownership record to
    /// the exact outer harness PID after spawn, before the runtime is exposed.
    pub fn bind_pid(&mut self, pid: u32) -> Result<(), String> {
        if pid == 0 {
            return Err("filesystem isolation cannot bind an invalid process id".to_string());
        }
        self.ownership.agent_pid = Some(pid);
        write_ownership_receipt(&self.ownership_path, &self.ownership, false)?;
        self.control.bind_pid(pid)?;
        Ok(())
    }
}

impl Drop for FilesystemIsolationRun {
    fn drop(&mut self) {
        self.control.shutdown();
        if let Err(error) = remove_run_root(&self.base, &self.root) {
            eprintln!(
                "buzz-desktop: failed to remove isolated run root {}: {error}",
                self.root.display()
            );
            return;
        }
        let _ = fs::remove_file(&self.ownership_path);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct IsolationRunOwnership {
    version: u8,
    identity_pubkey: String,
    desktop_instance_id: String,
    run_id: String,
    run_root: PathBuf,
    desktop_pid: u32,
    agent_pid: Option<u32>,
}

#[derive(Debug)]
struct IsolationControlPlane {
    run_id: String,
    attestation: FilesystemIsolationAttestation,
    registered: bool,
}

impl IsolationControlPlane {
    fn start(attestation: &FilesystemIsolationAttestation) -> Result<Self, String> {
        ensure_control_server()?;
        Ok(Self {
            run_id: attestation.run_id.clone(),
            attestation: attestation.clone(),
            registered: false,
        })
    }

    fn bind_pid(&mut self, pid: u32) -> Result<(), String> {
        let mut registry = live_isolation_registry()
            .lock()
            .map_err(|error| format!("isolation registry lock poisoned: {error}"))?;
        if registry.contains_key(&self.run_id) {
            return Err(format!("isolation run {} is already registered", self.run_id));
        }
        registry.insert(
            self.run_id.clone(),
            LiveIsolationRun {
                root_pid: pid,
                attestation: self.attestation.clone(),
            },
        );
        self.registered = true;
        Ok(())
    }

    fn shutdown(&mut self) {
        if self.registered {
            if let Ok(mut registry) = live_isolation_registry().lock() {
                registry.remove(&self.run_id);
            }
            self.registered = false;
        }
    }
}

#[derive(Debug, Clone)]
struct LiveIsolationRun {
    root_pid: u32,
    attestation: FilesystemIsolationAttestation,
}

fn live_isolation_registry() -> &'static Mutex<HashMap<String, LiveIsolationRun>> {
    static REGISTRY: OnceLock<Mutex<HashMap<String, LiveIsolationRun>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

#[cfg(target_os = "macos")]
fn ensure_control_server() -> Result<(), String> {
    static SERVER: OnceLock<Result<(), String>> = OnceLock::new();
    SERVER
        .get_or_init(|| {
            let path = Path::new(CONTROL_SOCKET);
            if path.exists() {
                if UnixStream::connect(path).is_ok() {
                    return Err(format!(
                        "another Buzz isolation control plane already owns {CONTROL_SOCKET}"
                    ));
                }
                fs::remove_file(path).map_err(|error| {
                    format!("failed to remove stale isolation control socket: {error}")
                })?;
            }
            let listener = UnixListener::bind(path)
                .map_err(|error| format!("failed to bind isolation control socket: {error}"))?;
            fs::set_permissions(path, fs::Permissions::from_mode(0o600)).map_err(|error| {
                format!("failed to protect isolation control socket: {error}")
            })?;
            thread::Builder::new()
                .name("buzz-isolation-control".into())
                .spawn(move || {
                    for stream in listener.incoming() {
                        match stream {
                            Ok(stream) => serve_control_request(stream),
                            Err(_) => break,
                        }
                    }
                })
                .map_err(|error| format!("failed to start isolation control plane: {error}"))?;
            Ok(())
        })
        .clone()
}

#[cfg(not(target_os = "macos"))]
fn ensure_control_server() -> Result<(), String> {
    Err("filesystem isolation control plane is currently supported only on macOS".to_string())
}

#[cfg(target_os = "macos")]
fn serve_control_request(mut stream: UnixStream) {
    let _ = stream.set_read_timeout(Some(Duration::from_millis(500)));
    let mut request = [0_u8; 64];
    let Ok(read) = stream.read(&mut request) else {
        return;
    };
    if &request[..read] != b"EXPLAIN\n" {
        let _ = stream.write_all(b"{\"error\":\"invalid request\"}\n");
        return;
    }
    let Some(peer_pid) = control_peer_pid(&stream) else {
        let _ = stream.write_all(b"{\"error\":\"unverified peer\"}\n");
        return;
    };
    let receipt = live_isolation_registry().lock().ok().and_then(|registry| {
        registry
            .values()
            .find(|run| {
                process_is_live(run.root_pid)
                    && process_is_descendant_of(peer_pid, run.root_pid)
            })
            .map(|run| run.attestation.clone())
    });
    let response = match receipt {
        Some(receipt) => serde_json::to_vec(&receipt).unwrap_or_else(|_| {
            b"{\"error\":\"failed to serialize Desktop receipt\"}".to_vec()
        }),
        None => b"{\"error\":\"peer is not a tracked isolated process\"}".to_vec(),
    };
    let _ = stream.write_all(&response);
    let _ = stream.write_all(b"\n");
}

#[cfg(target_os = "macos")]
fn control_peer_pid(stream: &UnixStream) -> Option<u32> {
    use std::os::fd::AsRawFd;
    const SOL_LOCAL: libc::c_int = 0;
    const LOCAL_PEERPID: libc::c_int = 2;
    let mut pid: libc::pid_t = 0;
    let mut length = std::mem::size_of::<libc::pid_t>() as libc::socklen_t;
    let result = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            SOL_LOCAL,
            LOCAL_PEERPID,
            &mut pid as *mut _ as *mut libc::c_void,
            &mut length,
        )
    };
    (result == 0 && pid > 0).then_some(pid as u32)
}

#[cfg(target_os = "macos")]
fn process_is_descendant_of(mut pid: u32, root_pid: u32) -> bool {
    for _ in 0..64 {
        if pid == root_pid {
            return true;
        }
        let Some(parent) = process_parent_pid(pid) else {
            return false;
        };
        if parent == 0 || parent == pid {
            return false;
        }
        pid = parent;
    }
    false
}

#[cfg(target_os = "macos")]
fn process_parent_pid(pid: u32) -> Option<u32> {
    let output = Command::new("/bin/ps")
        .args(["-o", "ppid=", "-p", &pid.to_string()])
        .output()
        .ok()?;
    output.status.success().then(|| {
        String::from_utf8_lossy(&output.stdout)
            .trim()
            .parse::<u32>()
            .ok()
    })?
}

fn process_is_live(pid: u32) -> bool {
    #[cfg(unix)]
    {
        let result = unsafe { libc::kill(pid as i32, 0) };
        result == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        false
    }
}

/// Build the outer command and fresh run-root guard for one isolated spawn.
///
/// The returned `Command` launches the existing ACP harness through the host
/// boundary. Callers configure stdio and environment on it exactly as they do
/// for an unisolated harness, then retain the guard for the process lifetime.
pub fn isolated_agent_command(
    profile: &FilesystemIsolationProfile,
    identity_pubkey: &str,
    desktop_instance_id: &str,
    acp_command: &Path,
) -> Result<(Command, FilesystemIsolationRun), String> {
    let FilesystemIsolationProfile::Ephemeral { read_only_roots } = profile;
    validate_identity(identity_pubkey)?;

    #[cfg(not(target_os = "macos"))]
    {
        let _ = (read_only_roots, desktop_instance_id, acp_command);
        return Err(
            "ephemeral filesystem isolation is currently supported only on macOS".to_string(),
        );
    }

    #[cfg(target_os = "macos")]
    {
        // Reconcile crash residue before every isolated spawn as well as at
        // startup. A startup warning must never silently degrade the next run.
        recover_abandoned_isolation_runs()?;
        let sandbox_exec = Path::new("/usr/bin/sandbox-exec");
        if !sandbox_exec.is_file() {
            return Err("macOS filesystem isolation requires /usr/bin/sandbox-exec".to_string());
        }

        let (base, run_id, root) = create_run_root(identity_pubkey)?;
        let result = (|| {
            let home = root.join("home");
            let temp = root.join("tmp");
            create_private_dir(&home)?;
            create_private_dir(&temp)?;

            let denied_roots = denied_roots()?;
            let mut allowed_read_roots = system_read_roots();
            allowed_read_roots.extend(validate_read_only_roots(
                read_only_roots,
                &protected_data_roots()?,
            )?);
            allowed_read_roots.extend(executable_read_roots(acp_command)?);
            allowed_read_roots.push(root.clone());
            normalize_paths(&mut allowed_read_roots);

            let allowed_write_roots = vec![root.clone()];
            let attestation = FilesystemIsolationAttestation {
                version: 1,
                enforcement: "macos_seatbelt_process_tree_control_plane_v1",
                identity_pubkey: identity_pubkey.to_ascii_lowercase(),
                run_id: run_id.clone(),
                run_root: root.clone(),
                allowed_read_roots: allowed_read_roots.clone(),
                allowed_write_roots: allowed_write_roots.clone(),
                denied_roots,
            };
            let profile_text = seatbelt_profile(&attestation)?;
            let control = IsolationControlPlane::start(&attestation)?;
            let receipts = receipts_dir(&base)?;
            let ownership_path = receipts.join(format!("{run_id}.json"));
            let ownership = IsolationRunOwnership {
                version: 1,
                identity_pubkey: identity_pubkey.to_ascii_lowercase(),
                desktop_instance_id: desktop_instance_id.to_string(),
                run_id,
                run_root: root.clone(),
                desktop_pid: std::process::id(),
                agent_pid: None,
            };
            write_ownership_receipt(&ownership_path, &ownership, true)?;

            let mut command = Command::new(sandbox_exec);
            command
                .arg("-p")
                .arg(profile_text)
                .arg(acp_command)
                .current_dir(&root)
                .env("HOME", &home)
                .env("TMPDIR", &temp)
                .env("XDG_CACHE_HOME", home.join(".cache"))
                .env("XDG_CONFIG_HOME", home.join(".config"))
                .env("XDG_DATA_HOME", home.join(".local/share"))
                .env(ISOLATION_RUN_ROOT_ENV, &root)
                .env(
                    ISOLATION_ATTESTATION_ENV,
                    serde_json::to_string(&attestation).map_err(|error| {
                        format!("failed to serialize isolation receipt: {error}")
                    })?,
                );

            Ok((
                command,
                FilesystemIsolationRun {
                    root: root.clone(),
                    base: base.clone(),
                    ownership_path,
                    ownership,
                    control,
                    attestation,
                },
            ))
        })();

        if result.is_err() {
            let _ = remove_run_root(&base, &root);
        }
        result
    }
}

/// Validate an owner-authored profile without creating a run root.
pub fn validate_filesystem_isolation_profile(
    profile: &FilesystemIsolationProfile,
) -> Result<(), String> {
    let FilesystemIsolationProfile::Ephemeral { read_only_roots } = profile;
    validate_read_only_roots(read_only_roots, &protected_data_roots()?).map(|_| ())
}

fn validate_identity(identity_pubkey: &str) -> Result<(), String> {
    if identity_pubkey.len() == 64 && identity_pubkey.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        Ok(())
    } else {
        Err("filesystem isolation requires an exact 64-character agent pubkey".to_string())
    }
}

fn create_run_root(identity_pubkey: &str) -> Result<(PathBuf, String, PathBuf), String> {
    let temp = std::env::temp_dir()
        .canonicalize()
        .map_err(|error| format!("failed to resolve temporary directory: {error}"))?;
    let base = temp.join(RUNS_DIR);
    reject_protected_overlap(&base)?;
    if base.exists() {
        let metadata = base
            .symlink_metadata()
            .map_err(|error| format!("failed to inspect isolation root: {error}"))?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(format!("refusing unsafe isolation base {}", base.display()));
        }
    } else {
        create_private_dir(&base)?;
    }
    let _ = receipts_dir(&base)?;

    let run_id = uuid::Uuid::new_v4().simple().to_string();
    let root = base.join(format!(
        "{}-{run_id}",
        &identity_pubkey.to_ascii_lowercase()[..16]
    ));
    create_private_dir(&root)?;
    Ok((base, run_id, root))
}

fn create_private_dir(path: &Path) -> Result<(), String> {
    fs::create_dir(path).map_err(|error| {
        format!(
            "failed to create isolated directory {}: {error}",
            path.display()
        )
    })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(|error| {
            format!(
                "failed to protect isolated directory {}: {error}",
                path.display()
            )
        })?;
    }
    Ok(())
}

fn protected_data_roots() -> Result<Vec<PathBuf>, String> {
    let home = dirs::home_dir().ok_or_else(|| {
        "filesystem isolation requires a resolvable home directory for fail-closed denial"
            .to_string()
    })?;
    let mut roots = vec![home.join(".buzz")];
    if let Some(nest) = super::nest_dir() {
        roots.push(nest);
    }
    normalize_paths(&mut roots);
    Ok(roots)
}

fn denied_roots() -> Result<Vec<PathBuf>, String> {
    let mut roots = vec![
        PathBuf::from("/Users"),
        PathBuf::from("/Volumes"),
        PathBuf::from("/Network"),
        PathBuf::from("/cores"),
        PathBuf::from("/private"),
    ];
    normalize_paths(&mut roots);
    Ok(roots)
}

fn reject_protected_overlap(path: &Path) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("isolation base has no parent: {}", path.display()))?;
    let canonical_parent = parent.canonicalize().map_err(|error| {
        format!(
            "failed to resolve isolation base parent {}: {error}",
            parent.display()
        )
    })?;
    let canonical = canonical_parent.join(
        path.file_name()
            .ok_or_else(|| format!("isolation base has no name: {}", path.display()))?,
    );
    if protected_data_roots()?.iter().any(|protected| {
        canonical.starts_with(protected) || protected.starts_with(&canonical)
    }) {
        return Err(format!(
            "filesystem isolation base overlaps protected Buzz data: {}",
            canonical.display()
        ));
    }
    Ok(())
}

fn receipts_dir(base: &Path) -> Result<PathBuf, String> {
    let path = base.join(RECEIPTS_DIR);
    match fs::create_dir(&path) {
        Ok(()) => {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).map_err(
                    |error| format!("failed to protect isolation receipts: {error}"),
                )?;
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => {
            return Err(format!("failed to create isolation receipts: {error}"));
        }
    }
    let metadata = path
        .symlink_metadata()
        .map_err(|error| format!("failed to inspect isolation receipts: {error}"))?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(format!("refusing unsafe isolation receipts {}", path.display()));
    }
    Ok(path)
}

fn write_ownership_receipt(
    path: &Path,
    receipt: &IsolationRunOwnership,
    create_new: bool,
) -> Result<(), String> {
    let payload = serde_json::to_vec(receipt)
        .map_err(|error| format!("failed to serialize isolation ownership: {error}"))?;
    if create_new {
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        let mut file = options.open(path).map_err(|error| {
            format!(
                "failed to create isolation ownership {}: {error}",
                path.display()
            )
        })?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(fs::Permissions::from_mode(0o600))
                .map_err(|error| format!("failed to protect isolation ownership: {error}"))?;
        }
        return file
            .write_all(&payload)
            .and_then(|_| file.sync_all())
            .map_err(|error| format!("failed to persist isolation ownership: {error}"));
    }

    let temp = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4().simple()));
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    let mut file = options.open(&temp).map_err(|error| {
        format!("failed to create isolation ownership {}: {error}", temp.display())
    })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(0o600))
            .map_err(|error| format!("failed to protect isolation ownership: {error}"))?;
    }
    file.write_all(&payload)
        .and_then(|_| file.sync_all())
        .map_err(|error| format!("failed to persist isolation ownership: {error}"))?;
    fs::rename(&temp, path)
        .map_err(|error| format!("failed to install isolation ownership: {error}"))
}

fn validate_read_only_roots(
    roots: &[PathBuf],
    denied_roots: &[PathBuf],
) -> Result<Vec<PathBuf>, String> {
    let home = dirs::home_dir();
    let mut validated = Vec::with_capacity(roots.len());
    for root in roots {
        if !root.is_absolute() {
            return Err(format!(
                "filesystem isolation read root must be absolute: {}",
                root.display()
            ));
        }
        let canonical = root.canonicalize().map_err(|error| {
            format!(
                "filesystem isolation read root must exist ({}): {error}",
                root.display()
            )
        })?;
        let metadata = canonical.symlink_metadata().map_err(|error| {
            format!(
                "failed to inspect read root {}: {error}",
                canonical.display()
            )
        })?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(format!(
                "filesystem isolation read root must be a real directory: {}",
                canonical.display()
            ));
        }
        if canonical == Path::new("/") || home.as_ref().is_some_and(|path| canonical == *path) {
            return Err(format!(
                "filesystem isolation refuses broad read root {}",
                canonical.display()
            ));
        }
        if denied_roots
            .iter()
            .any(|denied| canonical.starts_with(denied) || denied.starts_with(&canonical))
        {
            return Err(format!(
                "filesystem isolation read root overlaps protected Buzz data: {}",
                canonical.display()
            ));
        }
        validated.push(canonical);
    }
    normalize_paths(&mut validated);
    Ok(validated)
}

#[cfg(target_os = "macos")]
fn system_read_roots() -> Vec<PathBuf> {
    [
        "/System",
        "/usr",
        "/bin",
        "/sbin",
        "/Library",
        "/Applications",
        "/private/etc",
        "/private/var/db",
        "/private/var/run",
        "/dev",
        "/opt",
    ]
    .into_iter()
    .map(PathBuf::from)
    .filter(|path| path.exists())
    .collect()
}

#[cfg(target_os = "macos")]
fn executable_read_roots(command: &Path) -> Result<Vec<PathBuf>, String> {
    let canonical = command.canonicalize().map_err(|error| {
        format!(
            "failed to resolve ACP command {} for isolation: {error}",
            command.display()
        )
    })?;
    let parent = canonical.parent().ok_or_else(|| {
        format!(
            "ACP command has no parent directory: {}",
            canonical.display()
        )
    })?;
    Ok(vec![parent.to_path_buf()])
}

#[cfg(target_os = "macos")]
fn seatbelt_profile(attestation: &FilesystemIsolationAttestation) -> Result<String, String> {
    // A global file deny causes macOS libSystem to abort before the harness can
    // start. Preserve the platform bootstrap default, close every user-data and
    // mutable-private namespace, then reopen only the attested runtime roots
    // and this run's fresh write root. Specific allows override parent denies.
    let mut profile = String::from("(version 1)\n(allow default)\n");
    for root in &attestation.denied_roots {
        profile.push_str(&format!(
            "(deny file-read* file-write* (subpath \"{}\"))\n",
            escape_seatbelt_path(root)?
        ));
    }
    for root in &attestation.allowed_read_roots {
        profile.push_str(&format!(
            "(allow file-read* (subpath \"{}\"))\n",
            escape_seatbelt_path(root)?
        ));
    }
    for root in &attestation.allowed_write_roots {
        profile.push_str(&format!(
            "(allow file-write* (subpath \"{}\"))\n",
            escape_seatbelt_path(root)?
        ));
    }
    for device in ["/dev/null", "/dev/zero", "/dev/random", "/dev/urandom"] {
        profile.push_str(&format!(
            "(allow file-read* file-write* (literal \"{device}\"))\n"
        ));
    }
    profile.push_str(&format!(
        "(allow network-outbound (literal \"{}\"))\n",
        CONTROL_SOCKET
    ));
    Ok(profile)
}

/// Remove only crash residue whose durable receipt proves the exact run root
/// and whose Desktop owner and outer harness are both no longer live. Live or
/// ambiguous receipts are preserved so concurrent app instances cannot lose a
/// workspace underneath them.
pub fn recover_abandoned_isolation_runs() -> Result<Vec<PathBuf>, String> {
    let temp = std::env::temp_dir()
        .canonicalize()
        .map_err(|error| format!("failed to resolve temporary directory: {error}"))?;
    recover_abandoned_isolation_runs_in(&temp.join(RUNS_DIR), process_is_live)
}

fn recover_abandoned_isolation_runs_in(
    base: &Path,
    is_live: impl Fn(u32) -> bool,
) -> Result<Vec<PathBuf>, String> {
    reject_protected_overlap(base)?;
    if !base.exists() {
        return Ok(Vec::new());
    }
    let base_metadata = base
        .symlink_metadata()
        .map_err(|error| format!("failed to inspect isolation base: {error}"))?;
    if !base_metadata.is_dir() || base_metadata.file_type().is_symlink() {
        return Err(format!("refusing unsafe isolation base {}", base.display()));
    }
    let receipts = receipts_dir(base)?;
    let mut removed = Vec::new();
    let entries = fs::read_dir(&receipts)
        .map_err(|error| format!("failed to read isolation receipts: {error}"))?;
    for entry in entries.flatten() {
        let receipt_path = entry.path();
        if receipt_path.extension().is_none_or(|extension| extension != "json") {
            continue;
        }
        let Ok(metadata) = receipt_path.symlink_metadata() else {
            continue;
        };
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            continue;
        }
        let Ok(bytes) = fs::read(&receipt_path) else {
            continue;
        };
        let Ok(receipt) = serde_json::from_slice::<IsolationRunOwnership>(&bytes) else {
            continue;
        };
        if receipt.version != 1
            || receipt.identity_pubkey.len() != 64
            || !receipt
                .identity_pubkey
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
            || receipt.desktop_instance_id.trim().is_empty()
            || receipt.run_id.len() != 32
            || !receipt.run_id.bytes().all(|byte| byte.is_ascii_hexdigit())
            || receipt.run_root.parent() != Some(base)
            || !receipt.run_root.starts_with(base)
            || receipt_path.file_stem().and_then(|stem| stem.to_str())
                != Some(receipt.run_id.as_str())
            || !receipt.run_root.file_name().is_some_and(|name| {
                name.to_string_lossy().ends_with(&receipt.run_id)
            })
        {
            continue;
        }
        let owner_live = is_live(receipt.desktop_pid);
        let agent_live = receipt.agent_pid.is_some_and(&is_live);
        if owner_live || agent_live {
            continue;
        }
        remove_run_root(base, &receipt.run_root)?;
        fs::remove_file(&receipt_path).map_err(|error| {
            format!(
                "failed to remove recovered isolation receipt {}: {error}",
                receipt_path.display()
            )
        })?;
        removed.push(receipt.run_root);
    }
    Ok(removed)
}

#[cfg(target_os = "macos")]
fn escape_seatbelt_path(path: &Path) -> Result<String, String> {
    let raw = path
        .to_str()
        .ok_or_else(|| format!("isolation path is not valid UTF-8: {}", path.display()))?;
    if raw.contains(['\n', '\r', '\0']) {
        return Err(format!(
            "isolation path contains invalid bytes: {}",
            path.display()
        ));
    }
    Ok(raw.replace('\\', "\\\\").replace('"', "\\\""))
}

fn normalize_paths(paths: &mut Vec<PathBuf>) {
    paths.sort();
    paths.dedup();
}

fn remove_run_root(base: &Path, root: &Path) -> Result<(), String> {
    if root.parent() != Some(base) || !root.starts_with(base) {
        return Err(format!(
            "refusing to remove unscoped path {}",
            root.display()
        ));
    }
    match root.symlink_metadata() {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {
            fs::remove_dir_all(root)
                .map_err(|error| format!("failed to remove {}: {error}", root.display()))
        }
        Ok(_) => Err(format!(
            "refusing to remove non-directory run root {}",
            root.display()
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("failed to inspect {}: {error}", root.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn broad_and_buzz_overlapping_read_roots_fail_closed() {
        let protected = protected_data_roots().unwrap();
        assert!(validate_read_only_roots(&[PathBuf::from("/")], &protected).is_err());
        if let Some(home) = dirs::home_dir() {
            assert!(validate_read_only_roots(std::slice::from_ref(&home), &protected).is_err());
            let buzz = home.join(".buzz");
            if buzz.is_dir() {
                assert!(validate_read_only_roots(&[buzz], &protected).is_err());
            }
        }
    }

    #[test]
    fn invalid_identity_is_rejected_before_creating_a_run_root() {
        let profile = FilesystemIsolationProfile::Ephemeral {
            read_only_roots: Vec::new(),
        };
        let error =
            isolated_agent_command(&profile, "not-a-pubkey", "test", Path::new("/bin/sh"))
                .unwrap_err();
        assert!(error.contains("exact 64-character"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn seatbelt_receipt_never_allows_home_or_shared_buzz_root() {
        let profile = FilesystemIsolationProfile::Ephemeral {
            read_only_roots: Vec::new(),
        };
        let (_command, run) =
            isolated_agent_command(&profile, &"ab".repeat(32), "test", Path::new("/bin/sh"))
                .unwrap();
        let home = dirs::home_dir().unwrap();
        assert!(!run.attestation.allowed_read_roots.contains(&home));
        assert!(!run
            .attestation
            .allowed_read_roots
            .iter()
            .any(|root| root == &home.join(".buzz")));
        assert!(run.attestation.denied_roots.contains(&PathBuf::from("/Users")));
        assert!(run
            .root()
            .starts_with(std::env::temp_dir().canonicalize().unwrap()));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn seatbelt_denies_sibling_markers_and_nested_children_across_fresh_runs() {
        let operator = tempfile::tempdir().unwrap();
        let outside = operator.path().join("outside.txt");
        let outside_write = operator.path().join("outside-write.txt");
        fs::write(&outside, "OUTSIDE-TOKEN").unwrap();
        assert!(outside.metadata().unwrap().is_file());

        let profile = FilesystemIsolationProfile::Ephemeral {
            read_only_roots: Vec::new(),
        };
        let mut previous_root = None;
        for _ in 0..2 {
            let (mut command, mut run) = isolated_agent_command(
                &profile,
                &"ab".repeat(32),
                "test",
                Path::new("/bin/sh"),
            )
            .unwrap();
            let run_root = run.root().to_path_buf();
            if let Some(previous) = &previous_root {
                assert_ne!(previous, &run_root);
            }
            let inside = run_root.join("inside.txt");
            fs::write(&inside, "INSIDE-TOKEN").unwrap();

            command
                .arg("-c")
                .arg(
                    r#"
cat "$1" > "$3/inside.out"
inside_status=$?
cat "$2" > "$3/outside.out" 2>&1
outside_status=$?
printf x > "$3/inside-write.txt"
inside_write_status=$?
printf x > "$4" 2>/dev/null
outside_write_status=$?
/bin/sh -c 'cat "$1"' probe "$2" > "$3/nested.out" 2>&1
nested_status=$?
printf 'EXPLAIN\n' | /usr/bin/nc -U /private/tmp/buzz-isolation-control-v1.sock > "$3/receipt.json" 2>&1
control_status=$?
printf '%s %s %s %s %s %s' "$inside_status" "$outside_status" "$inside_write_status" "$outside_write_status" "$nested_status" "$control_status"
"#,
                )
                .arg("probe")
                .arg(&inside)
                .arg(&outside)
                .arg(&run_root)
                .arg(&outside_write)
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped());
            let child = command.spawn().unwrap();
            run.bind_pid(child.id()).unwrap();
            let output = child.wait_with_output().unwrap();
            assert!(
                output.status.success(),
                "status={:?} stdout={:?} stderr={:?}",
                output.status,
                output.stdout,
                output.stderr
            );
            assert_eq!(String::from_utf8_lossy(&output.stdout), "0 1 0 1 1 0");
            assert_eq!(
                fs::read_to_string(run_root.join("inside.out")).unwrap(),
                "INSIDE-TOKEN"
            );
            assert!(!fs::read_to_string(run_root.join("outside.out"))
                .unwrap()
                .contains("OUTSIDE-TOKEN"));
            assert!(!fs::read_to_string(run_root.join("nested.out"))
                .unwrap()
                .contains("OUTSIDE-TOKEN"));
            assert!(!outside_write.exists());
            let receipt: serde_json::Value =
                serde_json::from_slice(&fs::read(run_root.join("receipt.json")).unwrap()).unwrap();
            assert_eq!(receipt["identity_pubkey"], "ab".repeat(32));
            assert_eq!(receipt["run_root"], run_root.to_string_lossy().as_ref());

            drop(run);
            assert!(!run_root.exists());
            previous_root = Some(run_root);
        }
    }

    #[test]
    fn startup_recovery_removes_only_proven_abandoned_roots() {
        let operator = tempfile::tempdir().unwrap();
        let base = operator.path().join(RUNS_DIR);
        create_private_dir(&base).unwrap();
        let receipts = receipts_dir(&base).unwrap();

        let make_run = |run_id: &str, desktop_pid: u32, agent_pid: Option<u32>| {
            let root = base.join(format!("abababababababab-{run_id}"));
            create_private_dir(&root).unwrap();
            fs::write(root.join("residue"), "test").unwrap();
            let receipt = IsolationRunOwnership {
                version: 1,
                identity_pubkey: "ab".repeat(32),
                desktop_instance_id: "test".into(),
                run_id: run_id.into(),
                run_root: root.clone(),
                desktop_pid,
                agent_pid,
            };
            write_ownership_receipt(
                &receipts.join(format!("{run_id}.json")),
                &receipt,
                true,
            )
            .unwrap();
            root
        };

        let abandoned = make_run(&"a".repeat(32), 10, Some(11));
        let live_desktop = make_run(&"b".repeat(32), 20, None);
        let live_agent = make_run(&"c".repeat(32), 10, Some(30));
        let removed = recover_abandoned_isolation_runs_in(&base, |pid| pid == 20 || pid == 30)
            .unwrap();

        assert_eq!(removed, vec![abandoned.clone()]);
        assert!(!abandoned.exists());
        assert!(live_desktop.exists());
        assert!(live_agent.exists());
    }
}
