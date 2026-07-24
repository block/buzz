use base64::Engine;
use rustix::fs::{fstat, openat, FileType, Mode, OFlags, Stat};
use rustix::process::geteuid;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs::File;
use std::io::{self, Read, Seek};
use std::net::{IpAddr, Shutdown, TcpListener, TcpStream};
use std::os::fd::{AsRawFd, OwnedFd};
use std::path::{Component, Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

const MAXIMUM_PIN_FILE_BYTES: u64 = 64 * 1024;
const MAXIMUM_ACTIVE_PROXY_CONNECTIONS: usize = 8;

#[derive(Clone, Debug)]
pub(super) struct SshTunnelConfig {
    pub home_host_alias: String,
    pub home_user: String,
    pub pinned_host_fingerprint: String,
    pub known_hosts_path: PathBuf,
    pub identity_file: PathBuf,
    pub remote_loopback_port: u16,
    pub local_forward_port: u16,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct PinnedHostEvidence {
    pub host_alias: String,
    pub fingerprint: String,
    pub key_type: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum SshError {
    InvalidHostAlias,
    InvalidConfiguration,
    UnprotectedFile,
    InvalidKnownHosts,
    HostFingerprintMismatch,
    Spawn,
    EarlyExit,
    Teardown,
}

fn valid_name(value: &str, maximum: usize) -> bool {
    !value.is_empty()
        && value.len() <= maximum
        && value.is_ascii()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
}

pub(super) fn validate_host_target(alias: &str, user: &str) -> Result<(), SshError> {
    let value = alias;
    if !valid_name(value, 253)
        || value.parse::<IpAddr>().is_ok()
        || value.eq_ignore_ascii_case("localhost")
        || value.starts_with('.')
        || value.ends_with('.')
        || !valid_name(user, 64)
    {
        return Err(SshError::InvalidHostAlias);
    }
    Ok(())
}

#[derive(Debug)]
pub(super) struct ProtectedFile {
    file: File,
    stat: Stat,
    maximum: u64,
    ancestors: Vec<(File, Stat)>,
}

impl ProtectedFile {
    pub(super) fn open(path: &Path, maximum: u64) -> Result<Self, SshError> {
        if !path.is_absolute() || maximum == 0 {
            return Err(SshError::UnprotectedFile);
        }
        let mut directory = File::open("/").map_err(|_| SshError::UnprotectedFile)?;
        let mut ancestors = Vec::new();
        validate_directory(&directory)?;
        ancestors.push((
            directory
                .try_clone()
                .map_err(|_| SshError::UnprotectedFile)?,
            fstat(&directory).map_err(|_| SshError::UnprotectedFile)?,
        ));
        let mut components = path.components().peekable();
        if !matches!(components.next(), Some(Component::RootDir)) {
            return Err(SshError::UnprotectedFile);
        }
        let mut final_name = None;
        while let Some(component) = components.next() {
            let Component::Normal(name) = component else {
                return Err(SshError::UnprotectedFile);
            };
            if components.peek().is_none() {
                final_name = Some(name.to_owned());
                break;
            }
            let owned = openat(
                &directory,
                name,
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(|_| SshError::UnprotectedFile)?;
            directory = File::from(owned);
            validate_directory(&directory)?;
            ancestors.push((
                directory
                    .try_clone()
                    .map_err(|_| SshError::UnprotectedFile)?,
                fstat(&directory).map_err(|_| SshError::UnprotectedFile)?,
            ));
        }
        let name = final_name.ok_or(SshError::UnprotectedFile)?;
        // Intentionally omit CLOEXEC: SSH and the replication CLI consume this
        // exact verified inode through /dev/fd or /proc/self/fd.
        let owned: OwnedFd = openat(
            &directory,
            name,
            OFlags::RDONLY | OFlags::NOFOLLOW,
            Mode::empty(),
        )
        .map_err(|_| SshError::UnprotectedFile)?;
        let file = File::from(owned);
        let stat = fstat(&file).map_err(|_| SshError::UnprotectedFile)?;
        if FileType::from_raw_mode(stat.st_mode) != FileType::RegularFile
            || stat.st_uid != geteuid().as_raw()
            || stat.st_size <= 0
            || stat.st_size as u64 > maximum
            || stat.st_mode & 0o077 != 0
        {
            return Err(SshError::UnprotectedFile);
        }
        Ok(Self {
            file,
            stat,
            maximum,
            ancestors,
        })
    }

    pub(super) fn read_all(&self) -> Result<Vec<u8>, SshError> {
        let before = fstat(&self.file).map_err(|_| SshError::UnprotectedFile)?;
        if !same_inode(&self.stat, &before) {
            return Err(SshError::UnprotectedFile);
        }
        let mut file = self
            .file
            .try_clone()
            .map_err(|_| SshError::UnprotectedFile)?;
        file.seek(std::io::SeekFrom::Start(0))
            .map_err(|_| SshError::UnprotectedFile)?;
        let mut bytes = Vec::new();
        (&mut file)
            .take(self.maximum + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| SshError::UnprotectedFile)?;
        file.seek(std::io::SeekFrom::Start(0))
            .map_err(|_| SshError::UnprotectedFile)?;
        let after = fstat(&self.file).map_err(|_| SshError::UnprotectedFile)?;
        if !same_inode(&self.stat, &after)
            || bytes.is_empty()
            || bytes.len() as u64 > self.maximum
            || bytes.len() as i64 != self.stat.st_size
        {
            return Err(SshError::UnprotectedFile);
        }
        Ok(bytes)
    }

    pub(super) fn read_prefix(&self, maximum: usize) -> Result<Vec<u8>, SshError> {
        let mut file = self
            .file
            .try_clone()
            .map_err(|_| SshError::UnprotectedFile)?;
        file.seek(std::io::SeekFrom::Start(0))
            .map_err(|_| SshError::UnprotectedFile)?;
        let mut bytes = Vec::new();
        (&mut file)
            .take(maximum as u64)
            .read_to_end(&mut bytes)
            .map_err(|_| SshError::UnprotectedFile)?;
        file.seek(std::io::SeekFrom::Start(0))
            .map_err(|_| SshError::UnprotectedFile)?;
        if self.ancestors.iter().any(|(file, stat)| {
            fstat(file)
                .ok()
                .is_none_or(|value| !same_directory(stat, &value))
        }) || fstat(&self.file)
            .ok()
            .is_none_or(|value| !same_inode(&self.stat, &value))
        {
            return Err(SshError::UnprotectedFile);
        }
        Ok(bytes)
    }

    pub(super) fn descriptor_path(&self) -> PathBuf {
        #[cfg(target_os = "linux")]
        let prefix = "/proc/self/fd";
        #[cfg(not(target_os = "linux"))]
        let prefix = "/dev/fd";
        PathBuf::from(prefix).join(self.file.as_raw_fd().to_string())
    }

    pub(super) fn mode(&self) -> u32 {
        self.stat.st_mode.into()
    }

    pub(super) fn try_clone_file(&self) -> Result<File, SshError> {
        let mut file = self
            .file
            .try_clone()
            .map_err(|_| SshError::UnprotectedFile)?;
        file.seek(std::io::SeekFrom::Start(0))
            .map_err(|_| SshError::UnprotectedFile)?;
        Ok(file)
    }

    pub(super) fn matches_path(&self, path: &Path) -> bool {
        if self.ancestors.iter().any(|(file, stat)| {
            fstat(file)
                .map(|observed| !same_directory(stat, &observed))
                .unwrap_or(true)
        }) {
            return false;
        }
        Self::open(path, self.maximum)
            .ok()
            .is_some_and(|candidate| same_inode(&self.stat, &candidate.stat))
    }
}

fn validate_directory(directory: &File) -> Result<(), SshError> {
    let stat = fstat(directory).map_err(|_| SshError::UnprotectedFile)?;
    let uid = geteuid().as_raw();
    if FileType::from_raw_mode(stat.st_mode) != FileType::Directory
        || (stat.st_uid != uid && stat.st_uid != 0)
        || stat.st_mode & 0o022 != 0
    {
        return Err(SshError::UnprotectedFile);
    }
    Ok(())
}

fn same_inode(expected: &Stat, observed: &Stat) -> bool {
    expected.st_dev == observed.st_dev
        && expected.st_ino == observed.st_ino
        && expected.st_uid == observed.st_uid
        && expected.st_mode == observed.st_mode
        && expected.st_size == observed.st_size
}

fn same_directory(expected: &Stat, observed: &Stat) -> bool {
    expected.st_dev == observed.st_dev
        && expected.st_ino == observed.st_ino
        && expected.st_uid == observed.st_uid
        && expected.st_mode == observed.st_mode
}

pub(super) fn sha256_fingerprint(key_blob: &[u8]) -> String {
    let digest = Sha256::digest(key_blob);
    format!(
        "SHA256:{}",
        base64::engine::general_purpose::STANDARD_NO_PAD.encode(digest)
    )
}

#[cfg(test)]
pub(super) fn validate_host_pin(config: &SshTunnelConfig) -> Result<PinnedHostEvidence, SshError> {
    validate_host_material(config).map(|material| material.evidence)
}

struct VerifiedHostMaterial {
    evidence: PinnedHostEvidence,
    known_hosts: ProtectedFile,
    identity: ProtectedFile,
}

fn validate_host_material(config: &SshTunnelConfig) -> Result<VerifiedHostMaterial, SshError> {
    validate_host_target(&config.home_host_alias, &config.home_user)?;
    if config.remote_loopback_port == 0
        || config.local_forward_port == 0
        || config.pinned_host_fingerprint.len() > 128
    {
        return Err(SshError::InvalidConfiguration);
    }
    let known_hosts_file = ProtectedFile::open(&config.known_hosts_path, MAXIMUM_PIN_FILE_BYTES)?;
    let identity = ProtectedFile::open(&config.identity_file, MAXIMUM_PIN_FILE_BYTES)?;
    let known_hosts = known_hosts_file.read_all()?;
    let known_hosts = std::str::from_utf8(&known_hosts).map_err(|_| SshError::InvalidKnownHosts)?;
    let mut lines = known_hosts.lines();
    let line = lines.next().ok_or(SshError::InvalidKnownHosts)?;
    if lines.any(|line| !line.trim().is_empty()) {
        return Err(SshError::InvalidKnownHosts);
    }
    let fields = line.split_ascii_whitespace().collect::<Vec<_>>();
    if fields.len() != 3
        || fields[0] != config.home_host_alias
        || fields[1].contains("-cert-")
        || !(fields[1].starts_with("ssh-") || fields[1].starts_with("ecdsa-sha2-"))
    {
        return Err(SshError::InvalidKnownHosts);
    }
    let key_blob = base64::engine::general_purpose::STANDARD
        .decode(fields[2])
        .map_err(|_| SshError::InvalidKnownHosts)?;
    if key_blob.is_empty() || key_blob.len() > 16 * 1024 {
        return Err(SshError::InvalidKnownHosts);
    }
    let fingerprint = sha256_fingerprint(&key_blob);
    if fingerprint != config.pinned_host_fingerprint {
        return Err(SshError::HostFingerprintMismatch);
    }
    Ok(VerifiedHostMaterial {
        evidence: PinnedHostEvidence {
            host_alias: config.home_host_alias.clone(),
            fingerprint,
            key_type: fields[1].to_string(),
        },
        known_hosts: known_hosts_file,
        identity,
    })
}

pub(super) struct ReservedLoopbackPort {
    listener: TcpListener,
}

impl ReservedLoopbackPort {
    pub(super) fn new() -> io::Result<Self> {
        TcpListener::bind(("127.0.0.1", 0)).map(|listener| Self { listener })
    }

    pub(super) fn port(&self) -> u16 {
        self.listener
            .local_addr()
            .map(|address| address.port())
            .unwrap_or(0)
    }

    #[cfg(test)]
    fn at(port: u16) -> io::Result<Self> {
        TcpListener::bind(("127.0.0.1", port)).map(|listener| Self { listener })
    }
}

#[cfg(test)]
pub(super) fn reserve_loopback_port() -> io::Result<u16> {
    ReservedLoopbackPort::new().map(|reservation| reservation.port())
}

pub(super) struct SshTunnel {
    cancel: Arc<AtomicBool>,
    failed: Arc<AtomicBool>,
    streams: Arc<Mutex<HashMap<u64, TcpStream>>>,
    supervisor: Option<JoinHandle<()>>,
    _material: Arc<VerifiedHostMaterial>,
    pub evidence: PinnedHostEvidence,
    pub local_forward_port: u16,
}

fn terminate(child: &mut Child) -> Result<(), SshError> {
    match child.try_wait() {
        Ok(Some(_)) => Ok(()),
        Ok(None) => {
            child.kill().map_err(|_| SshError::Teardown)?;
            child.wait().map_err(|_| SshError::Teardown)?;
            Ok(())
        }
        Err(_) => Err(SshError::Teardown),
    }
}

impl Drop for SshTunnel {
    fn drop(&mut self) {
        let _ = self.close_inner();
    }
}

impl SshTunnel {
    pub(super) fn ensure_running(&self) -> Result<(), SshError> {
        if self.failed.load(Ordering::SeqCst) || self.cancel.load(Ordering::SeqCst) {
            Err(SshError::EarlyExit)
        } else {
            Ok(())
        }
    }

    pub(super) fn close(mut self) -> Result<(), SshError> {
        self.close_inner()
    }

    #[cfg(test)]
    fn active_connection_count(&self) -> usize {
        self.streams.lock().map(|value| value.len()).unwrap_or(0)
    }

    fn close_inner(&mut self) -> Result<(), SshError> {
        self.cancel.store(true, Ordering::SeqCst);
        if let Ok(streams) = self.streams.lock() {
            for stream in streams.values() {
                let _ = stream.shutdown(Shutdown::Both);
            }
        }
        let _ = TcpStream::connect(("127.0.0.1", self.local_forward_port));
        let joined = self
            .supervisor
            .take()
            .map(|thread| thread.join().is_ok())
            .unwrap_or(true);
        if joined && !self.failed.load(Ordering::SeqCst) {
            Ok(())
        } else {
            Err(SshError::Teardown)
        }
    }
}

#[cfg(test)]
pub(super) fn start_tunnel_with_binary(
    ssh_binary: &Path,
    config: &SshTunnelConfig,
) -> Result<SshTunnel, SshError> {
    let reservation =
        ReservedLoopbackPort::at(config.local_forward_port).map_err(|_| SshError::Spawn)?;
    let mut config = config.clone();
    start_tunnel_with_reservation(ssh_binary, &mut config, reservation)
}

pub(super) fn start_tunnel_with_reservation(
    ssh_binary: &Path,
    config: &mut SshTunnelConfig,
    reservation: ReservedLoopbackPort,
) -> Result<SshTunnel, SshError> {
    if reservation.port() == 0 || config.local_forward_port != reservation.port() {
        return Err(SshError::InvalidConfiguration);
    }
    let material = Arc::new(validate_host_material(config)?);
    let host_key_algorithms = format!("HostKeyAlgorithms={}", material.evidence.key_type);
    let known_hosts = format!(
        "UserKnownHostsFile={}",
        material.known_hosts.descriptor_path().to_string_lossy()
    );
    let identity = format!(
        "IdentityFile={}",
        material.identity.descriptor_path().to_string_lossy()
    );
    let host_alias = format!("HostKeyAlias={}", config.home_host_alias);
    let target = format!("{}@{}", config.home_user, config.home_host_alias);
    let direct = format!("127.0.0.1:{}", config.remote_loopback_port);
    let args = [
        "-F",
        "/dev/null",
        "-T",
        "-o",
        "BatchMode=yes",
        "-o",
        "StrictHostKeyChecking=yes",
        "-o",
        "GlobalKnownHostsFile=/dev/null",
        "-o",
        known_hosts.as_str(),
        "-o",
        host_alias.as_str(),
        "-o",
        host_key_algorithms.as_str(),
        "-o",
        "ForwardAgent=no",
        "-o",
        "IdentityAgent=none",
        "-o",
        "IdentitiesOnly=yes",
        "-o",
        identity.as_str(),
        "-o",
        "PasswordAuthentication=no",
        "-o",
        "KbdInteractiveAuthentication=no",
        "-o",
        "GSSAPIAuthentication=no",
        "-o",
        "PermitLocalCommand=no",
        "-o",
        "ProxyCommand=none",
        "-o",
        "ProxyJump=none",
        "-o",
        "RequestTTY=no",
        "-o",
        "ExitOnForwardFailure=yes",
        "-o",
        "ConnectTimeout=5",
        "-o",
        "ServerAliveInterval=15",
        "-o",
        "ServerAliveCountMax=1",
        "-W",
        direct.as_str(),
        target.as_str(),
    ]
    .map(str::to_string)
    .to_vec();
    let listener = reservation.listener;
    listener
        .set_nonblocking(true)
        .map_err(|_| SshError::Spawn)?;
    let cancel = Arc::new(AtomicBool::new(false));
    let failed = Arc::new(AtomicBool::new(false));
    let streams = Arc::new(Mutex::new(HashMap::new()));
    let thread_cancel = Arc::clone(&cancel);
    let thread_failed = Arc::clone(&failed);
    let thread_streams = Arc::clone(&streams);
    let binary = ssh_binary.to_path_buf();
    let supervisor = std::thread::spawn(move || {
        let mut workers: Vec<JoinHandle<()>> = Vec::new();
        let mut next_connection_id = 0_u64;
        while !thread_cancel.load(Ordering::SeqCst) {
            let mut index = 0;
            while index < workers.len() {
                if workers[index].is_finished() {
                    let worker = workers.swap_remove(index);
                    if worker.join().is_err() {
                        thread_failed.store(true, Ordering::SeqCst);
                    }
                } else {
                    index += 1;
                }
            }
            match listener.accept() {
                Ok((stream, _)) => {
                    let connection_id = next_connection_id;
                    next_connection_id = next_connection_id.wrapping_add(1);
                    let admitted = match thread_streams.lock() {
                        Ok(mut active) if active.len() < MAXIMUM_ACTIVE_PROXY_CONNECTIONS => {
                            match stream.try_clone() {
                                Ok(clone) => {
                                    active.insert(connection_id, clone);
                                    true
                                }
                                Err(_) => false,
                            }
                        }
                        _ => false,
                    };
                    if !admitted {
                        let _ = stream.shutdown(Shutdown::Both);
                        continue;
                    }
                    let worker_cancel = Arc::clone(&thread_cancel);
                    let worker_failed = Arc::clone(&thread_failed);
                    let worker_streams = Arc::clone(&thread_streams);
                    let worker_binary = binary.clone();
                    let worker_args = args.clone();
                    workers.push(std::thread::spawn(move || {
                        if proxy_connection(stream, &worker_binary, &worker_args, &worker_cancel)
                            .is_err()
                        {
                            worker_failed.store(true, Ordering::SeqCst);
                        }
                        if let Ok(mut active) = worker_streams.lock() {
                            active.remove(&connection_id);
                        }
                    }));
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(_) => {
                    thread_failed.store(true, Ordering::SeqCst);
                    break;
                }
            }
        }
        for worker in workers {
            if worker.join().is_err() {
                thread_failed.store(true, Ordering::SeqCst);
            }
        }
    });
    Ok(SshTunnel {
        cancel,
        failed,
        streams,
        supervisor: Some(supervisor),
        _material: Arc::clone(&material),
        evidence: material.evidence.clone(),
        local_forward_port: config.local_forward_port,
    })
}

fn proxy_connection(
    stream: TcpStream,
    ssh_binary: &Path,
    args: &[String],
    cancel: &AtomicBool,
) -> Result<(), SshError> {
    let mut child = Command::new(ssh_binary)
        .env_clear()
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| SshError::Spawn)?;
    let mut stdin = child.stdin.take().ok_or(SshError::Spawn)?;
    let mut stdout = child.stdout.take().ok_or(SshError::Spawn)?;
    let mut inbound = stream.try_clone().map_err(|_| SshError::Spawn)?;
    let mut outbound = stream.try_clone().map_err(|_| SshError::Spawn)?;
    let input = std::thread::spawn(move || io::copy(&mut inbound, &mut stdin));
    let output = std::thread::spawn(move || io::copy(&mut stdout, &mut outbound));
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let _ = stream.shutdown(Shutdown::Both);
                let _ = input.join();
                let _ = output.join();
                return if status.success() {
                    Ok(())
                } else {
                    Err(SshError::EarlyExit)
                };
            }
            Ok(None) if cancel.load(Ordering::SeqCst) => {
                let result = terminate(&mut child);
                let _ = stream.shutdown(Shutdown::Both);
                let _ = input.join();
                let _ = output.join();
                return result;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(5)),
            Err(_) => return terminate(&mut child).and(Err(SshError::Teardown)),
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use base64::Engine;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;
    use std::time::Duration;

    fn protected_file(directory: &std::path::Path, name: &str, contents: &[u8]) -> PathBuf {
        let path = directory.join(name);
        fs::write(&path, contents).expect("write protected fixture");
        let mut permissions = fs::metadata(&path).expect("fixture metadata").permissions();
        permissions.set_mode(0o600);
        fs::set_permissions(&path, permissions).expect("protect fixture");
        path
    }

    fn tempdir() -> tempfile::TempDir {
        let base = std::env::current_dir().expect("current directory");
        tempfile::Builder::new()
            .tempdir_in(&base)
            .expect("temporary directory")
    }

    fn tunnel_config(directory: &std::path::Path, local_forward_port: u16) -> SshTunnelConfig {
        let key_blob = b"bounded fake OpenSSH public key blob";
        let fingerprint = sha256_fingerprint(key_blob);
        let encoded = base64::engine::general_purpose::STANDARD.encode(key_blob);
        let known_hosts_path = protected_file(
            directory,
            "memory_known_hosts",
            format!("memory-home ssh-ed25519 {encoded}\n").as_bytes(),
        );
        let identity_file =
            protected_file(directory, "memory_identity", b"fake private key fixture\n");
        SshTunnelConfig {
            home_host_alias: "memory-home".to_string(),
            home_user: "memory-sync".to_string(),
            pinned_host_fingerprint: fingerprint,
            known_hosts_path,
            identity_file,
            remote_loopback_port: 8006,
            local_forward_port,
        }
    }

    #[test]
    fn validates_exact_dedicated_known_host_fingerprint() {
        let directory = tempdir();
        let config = tunnel_config(directory.path(), 43123);

        let evidence = validate_host_pin(&config).expect("valid pin");

        assert_eq!(evidence.host_alias, "memory-home");
        assert_eq!(
            evidence.fingerprint,
            config.pinned_host_fingerprint.as_str()
        );
        assert_eq!(evidence.key_type, "ssh-ed25519");
    }

    #[test]
    fn rejects_ip_alias_mismatch_symlinks_and_unprotected_credentials() {
        let directory = tempdir();
        let mut ip_alias = tunnel_config(directory.path(), 43124);
        ip_alias.home_host_alias = "192.168.1.26".to_string();
        assert_eq!(
            validate_host_pin(&ip_alias).expect_err("IP aliases are forbidden"),
            SshError::InvalidHostAlias
        );

        let mut mismatched = tunnel_config(directory.path(), 43125);
        mismatched.pinned_host_fingerprint = "SHA256:AAAAAAAA".to_string();
        assert_eq!(
            validate_host_pin(&mismatched).expect_err("pin mismatch must fail"),
            SshError::HostFingerprintMismatch
        );

        let real_known_hosts = mismatched.known_hosts_path.clone();
        let symlink = directory.path().join("known-hosts-link");
        std::os::unix::fs::symlink(&real_known_hosts, &symlink).expect("create fixture symlink");
        mismatched.known_hosts_path = symlink;
        assert_eq!(
            validate_host_pin(&mismatched).expect_err("known-hosts symlink must fail"),
            SshError::UnprotectedFile
        );

        let open_identity = tunnel_config(directory.path(), 43126);
        let mut permissions = fs::metadata(&open_identity.identity_file)
            .expect("identity metadata")
            .permissions();
        permissions.set_mode(0o644);
        fs::set_permissions(&open_identity.identity_file, permissions)
            .expect("make identity unprotected");
        assert_eq!(
            validate_host_pin(&open_identity).expect_err("open identity must fail"),
            SshError::UnprotectedFile
        );
    }

    #[test]
    fn owned_listener_blocks_squatter_token_capture_and_uses_hardened_direct_ssh() {
        std::env::set_var("BUZZ_MEMORY_INHERITED_SENTINEL", "secret");
        let directory = tempdir();
        let arguments_log = directory.path().join("ssh-arguments");
        let environment_log = directory.path().join("ssh-environment");
        let fake_ssh = directory.path().join("fake-ssh");
        let fixture = format!(
            r#"#!/bin/sh
set -eu
printf '%s\n' "$@" > '{}'
if env | grep -q '^BUZZ_MEMORY_INHERITED_SENTINEL='; then
  printf 'inherited\n' > '{}'
else
  printf 'cleared\n' > '{}'
fi
exec /bin/cat
"#,
            arguments_log.display(),
            environment_log.display(),
            environment_log.display(),
        );
        fs::write(&fake_ssh, fixture).expect("write fake ssh");
        let mut permissions = fs::metadata(&fake_ssh)
            .expect("fixture metadata")
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&fake_ssh, permissions).expect("make fixture executable");
        let local_port = reserve_loopback_port().expect("reserve loopback port");
        let config = tunnel_config(directory.path(), local_port);

        let tunnel = start_tunnel_with_binary(&fake_ssh, &config).expect("start fake tunnel");
        assert!(
            TcpListener::bind(("127.0.0.1", local_port)).is_err(),
            "a squatter can never replace Buzz's bearer-facing listener"
        );
        let mut client =
            TcpStream::connect(("127.0.0.1", local_port)).expect("connect owned proxy");
        use std::io::Write;
        client
            .write_all(b"Authorization: Bearer never-to-squatter\n")
            .expect("write bearer through owned proxy");
        client
            .shutdown(Shutdown::Write)
            .expect("finish proxy request");
        let mut response = String::new();
        client
            .read_to_string(&mut response)
            .expect("read proxy response");
        assert_eq!(response, "Authorization: Bearer never-to-squatter\n");

        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while (!arguments_log.exists() || !environment_log.exists())
            && std::time::Instant::now() < deadline
        {
            std::thread::sleep(Duration::from_millis(5));
        }
        let arguments = fs::read_to_string(&arguments_log).expect("read arguments log");
        assert!(arguments.contains("StrictHostKeyChecking=yes"));
        assert!(arguments.contains("ForwardAgent=no"));
        assert!(arguments.contains("IdentityAgent=none"));
        assert!(arguments.contains("ExitOnForwardFailure=yes"));
        assert!(arguments.contains("memory-sync@memory-home"));
        assert!(arguments.contains("-W\n127.0.0.1:8006"));
        assert!(!arguments.contains("-L"));
        assert!(!arguments.contains("-R"));
        assert!(!arguments.contains("-D"));
        assert_eq!(
            fs::read_to_string(&environment_log).expect("read environment log"),
            "cleared\n"
        );
        drop(tunnel);
        std::env::remove_var("BUZZ_MEMORY_INHERITED_SENTINEL");
    }

    #[test]
    fn spawn_handoff_does_not_treat_an_arbitrary_listener_as_authenticated_readiness() {
        let directory = tempdir();
        let fake_ssh = directory.path().join("hung-ssh");
        fs::write(&fake_ssh, "#!/bin/sh\nexec /bin/sleep 30\n").expect("write hung fake");
        let mut permissions = fs::metadata(&fake_ssh)
            .expect("fixture metadata")
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&fake_ssh, permissions).expect("make fixture executable");
        let config = tunnel_config(
            directory.path(),
            reserve_loopback_port().expect("reserve loopback port"),
        );
        let tunnel = start_tunnel_with_binary(&fake_ssh, &config)
            .expect("spawn handoff succeeds without claiming HTTP readiness");
        let _client =
            TcpStream::connect(("127.0.0.1", tunnel.local_forward_port)).expect("start hung child");
        std::thread::sleep(Duration::from_millis(50));
        let started = std::time::Instant::now();
        tunnel
            .close()
            .expect("hung SSH is explicitly killed and reaped");
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn reserved_loopback_listener_is_held_until_ssh_spawn_handoff() {
        let reservation = ReservedLoopbackPort::new().expect("reserve loopback listener");
        let port = reservation.port();
        assert!(TcpListener::bind(("127.0.0.1", port)).is_err());

        let directory = tempdir();
        let fake_ssh = directory.path().join("fake-ssh");
        fs::write(&fake_ssh, "#!/bin/sh\nexec /bin/cat\n").expect("write fake ssh");
        let mut permissions = fs::metadata(&fake_ssh)
            .expect("fixture metadata")
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&fake_ssh, permissions).expect("make executable");
        let mut config = tunnel_config(directory.path(), port);

        let tunnel = start_tunnel_with_reservation(&fake_ssh, &mut config, reservation)
            .expect("spawn owns reservation handoff");

        assert_eq!(tunnel.local_forward_port, port);
        assert!(
            TcpListener::bind(("127.0.0.1", port)).is_err(),
            "Buzz must retain exclusive ownership for the tunnel lifetime"
        );
        tunnel.close().expect("explicitly reap ssh");
    }

    #[test]
    fn local_connection_flood_is_bounded_and_cannot_spawn_unlimited_ssh_children() {
        let directory = tempdir();
        let fake_ssh = directory.path().join("hung-flood-ssh");
        fs::write(&fake_ssh, "#!/bin/sh\nexec /bin/sleep 30\n").expect("write fake ssh");
        let mut permissions = fs::metadata(&fake_ssh)
            .expect("fixture metadata")
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&fake_ssh, permissions).expect("make executable");
        let reservation = ReservedLoopbackPort::new().expect("reserve owned listener");
        let port = reservation.port();
        let mut config = tunnel_config(directory.path(), port);
        let tunnel = start_tunnel_with_reservation(&fake_ssh, &mut config, reservation)
            .expect("start bounded proxy");

        let clients = (0..32)
            .filter_map(|_| TcpStream::connect(("127.0.0.1", port)).ok())
            .collect::<Vec<_>>();
        std::thread::sleep(Duration::from_millis(150));

        assert!(clients.len() > MAXIMUM_ACTIVE_PROXY_CONNECTIONS);
        assert!(tunnel.active_connection_count() <= MAXIMUM_ACTIVE_PROXY_CONNECTIONS);
        tunnel.close().expect("cancel bounded proxy children");
    }

    #[test]
    fn protected_file_open_rejects_symlinked_ancestor_and_pins_inode() {
        let directory = tempdir();
        let real = directory.path().join("real");
        fs::create_dir(&real).expect("create real directory");
        let file = protected_file(&real, "known_hosts", b"original\n");
        let alias = directory.path().join("alias");
        std::os::unix::fs::symlink(&real, &alias).expect("create ancestor symlink");

        assert_eq!(
            ProtectedFile::open(&alias.join("known_hosts"), 1024)
                .expect_err("symlinked ancestor must fail"),
            SshError::UnprotectedFile
        );

        let protected = ProtectedFile::open(&file, 1024).expect("open protected inode");
        fs::rename(&file, real.join("old")).expect("replace protected path");
        protected_file(&real, "known_hosts", b"swapped\n");
        assert_eq!(
            protected.read_all().expect("read pinned descriptor"),
            b"original\n"
        );

        let writable = directory.path().join("writable");
        fs::create_dir(&writable).expect("create writable ancestor");
        let writable_file = protected_file(&writable, "identity", b"secret\n");
        let mut permissions = fs::metadata(&writable)
            .expect("ancestor metadata")
            .permissions();
        permissions.set_mode(0o777);
        fs::set_permissions(&writable, permissions).expect("make ancestor writable");
        assert_eq!(
            ProtectedFile::open(&writable_file, 1024).expect_err("writable ancestor must fail"),
            SshError::UnprotectedFile
        );
    }
}
