use base64::Engine;
use sha2::{Digest, Sha256};
use std::io;
use std::net::{IpAddr, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

const MAXIMUM_PIN_FILE_BYTES: u64 = 64 * 1024;

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
    StartupTimeout,
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

#[cfg(unix)]
fn validate_protected_file(path: &Path, maximum: u64) -> Result<Vec<u8>, SshError> {
    if !path.is_absolute() {
        return Err(SshError::UnprotectedFile);
    }
    let metadata = std::fs::symlink_metadata(path).map_err(|_| SshError::UnprotectedFile)?;
    if metadata.file_type().is_symlink()
        || !metadata.is_file()
        || metadata.len() == 0
        || metadata.len() > maximum
        || metadata.permissions().mode() & 0o077 != 0
    {
        return Err(SshError::UnprotectedFile);
    }
    std::fs::read(path).map_err(|_| SshError::UnprotectedFile)
}

#[cfg(not(unix))]
fn validate_protected_file(_path: &Path, _maximum: u64) -> Result<Vec<u8>, SshError> {
    Err(SshError::UnprotectedFile)
}

pub(super) fn sha256_fingerprint(key_blob: &[u8]) -> String {
    let digest = Sha256::digest(key_blob);
    format!(
        "SHA256:{}",
        base64::engine::general_purpose::STANDARD_NO_PAD.encode(digest)
    )
}

pub(super) fn validate_host_pin(config: &SshTunnelConfig) -> Result<PinnedHostEvidence, SshError> {
    validate_host_target(&config.home_host_alias, &config.home_user)?;
    if config.remote_loopback_port == 0
        || config.local_forward_port == 0
        || config.pinned_host_fingerprint.len() > 128
    {
        return Err(SshError::InvalidConfiguration);
    }
    let known_hosts = validate_protected_file(&config.known_hosts_path, MAXIMUM_PIN_FILE_BYTES)?;
    let _identity = validate_protected_file(&config.identity_file, MAXIMUM_PIN_FILE_BYTES)?;
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
    Ok(PinnedHostEvidence {
        host_alias: config.home_host_alias.clone(),
        fingerprint,
        key_type: fields[1].to_string(),
    })
}

pub(super) fn reserve_loopback_port() -> io::Result<u16> {
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    listener.local_addr().map(|address| address.port())
}

#[derive(Debug)]
pub(super) struct SshTunnel {
    child: Child,
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
        let _ = terminate(&mut self.child);
    }
}

pub(super) fn start_tunnel_with_binary(
    ssh_binary: &Path,
    config: &SshTunnelConfig,
    startup_timeout: Duration,
) -> Result<SshTunnel, SshError> {
    if startup_timeout.is_zero() || startup_timeout > Duration::from_secs(30) {
        return Err(SshError::InvalidConfiguration);
    }
    let evidence = validate_host_pin(config)?;
    let forward = format!(
        "127.0.0.1:{}:127.0.0.1:{}",
        config.local_forward_port, config.remote_loopback_port
    );
    let host_key_algorithms = format!("HostKeyAlgorithms={}", evidence.key_type);
    let known_hosts = format!(
        "UserKnownHostsFile={}",
        config.known_hosts_path.to_string_lossy()
    );
    let identity = format!("IdentityFile={}", config.identity_file.to_string_lossy());
    let host_alias = format!("HostKeyAlias={}", config.home_host_alias);
    let target = format!("{}@{}", config.home_user, config.home_host_alias);
    let mut child = Command::new(ssh_binary)
        .env_clear()
        .args([
            "-F",
            "/dev/null",
            "-N",
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
            "-L",
            forward.as_str(),
            target.as_str(),
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| SshError::Spawn)?;

    let deadline = Instant::now() + startup_timeout;
    let address = SocketAddr::from(([127, 0, 0, 1], config.local_forward_port));
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return Err(SshError::EarlyExit),
            Ok(None) => {}
            Err(_) => {
                let _ = terminate(&mut child);
                return Err(SshError::Teardown);
            }
        }
        if TcpStream::connect_timeout(&address, Duration::from_millis(25)).is_ok() {
            return Ok(SshTunnel {
                child,
                evidence,
                local_forward_port: config.local_forward_port,
            });
        }
        if Instant::now() >= deadline {
            let teardown = terminate(&mut child);
            return teardown.and(Err(SshError::StartupTimeout));
        }
        std::thread::sleep(Duration::from_millis(10));
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
        let directory = tempfile::tempdir().expect("temporary directory");
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
        let directory = tempfile::tempdir().expect("temporary directory");
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
    fn spawns_direct_ssh_with_fixed_hardened_arguments_and_cleared_environment() {
        std::env::set_var("BUZZ_MEMORY_INHERITED_SENTINEL", "secret");
        let directory = tempfile::tempdir().expect("temporary directory");
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
forward=''
previous=''
for argument in "$@"; do
  if [ "$previous" = '-L' ]; then forward="$argument"; break; fi
  previous="$argument"
done
port=$(printf '%s' "$forward" | cut -d: -f2)
exec /usr/bin/python3 -c 'import socket,sys,time
s=socket.socket()
s.setsockopt(socket.SOL_SOCKET,socket.SO_REUSEADDR,1)
s.bind(("127.0.0.1",int(sys.argv[1])))
s.listen()
while True:
 c,_=s.accept()
 c.close()' "$port"
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

        let tunnel = start_tunnel_with_binary(&fake_ssh, &config, Duration::from_secs(2))
            .expect("start fake tunnel");

        let arguments = fs::read_to_string(&arguments_log).expect("read arguments log");
        assert!(arguments.contains("StrictHostKeyChecking=yes"));
        assert!(arguments.contains("ForwardAgent=no"));
        assert!(arguments.contains("IdentityAgent=none"));
        assert!(arguments.contains("ExitOnForwardFailure=yes"));
        assert!(arguments.contains("memory-sync@memory-home"));
        assert!(arguments.contains(&format!("127.0.0.1:{local_port}:127.0.0.1:8006")));
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
    fn startup_timeout_kills_and_reaps_fake_ssh() {
        let directory = tempfile::tempdir().expect("temporary directory");
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
        let started = std::time::Instant::now();

        let error = start_tunnel_with_binary(&fake_ssh, &config, Duration::from_millis(100))
            .expect_err("hung tunnel must time out");

        assert_eq!(error, SshError::StartupTimeout);
        assert!(started.elapsed() < Duration::from_secs(2));
    }
}
