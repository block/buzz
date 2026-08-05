//! Buzz Desktop provider for an already-supervised native Hermes gateway.
//!
//! This provider deliberately does not launch an ACP process locally.  It sends
//! a deployment description over SSH to the configured host, where a small
//! Python transaction updates the Hermes profile and restarts its existing
//! launchd/systemd gateway.  SSH credentials are ambient (agent/config), never
//! provider_config fields, and the identity is supplied only in the deploy
//! payload by Buzz Desktop.

use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde_json::{json, Map, Value};
use std::io::{self, Read, Write};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

const PROTOCOL_VERSION: u64 = 1;
const REMOTE_SCRIPT: &str = r###"
import fcntl, json, os, pathlib, stat, subprocess, sys, tempfile, time

request = json.load(sys.stdin)
agent = request.get("agent") or {}
cfg = request.get("provider_config") or {}
operation = str(request.get("op") or "deploy")

home = pathlib.Path(str(cfg.get("home") or "~/.hermes")).expanduser()
profile = str(cfg["profile"])
profile_home = pathlib.Path(str(cfg.get("profile_home") or (home if profile in ("", "default") else home / "profiles" / profile))).expanduser()
try:
    home_root = home.resolve()
    profile_root = profile_home.resolve()
    if home != home_root or profile_home != profile_root:
        raise RuntimeError("Hermes home and profile_home must not contain symlinks or dot segments")
    profile_root.relative_to(home_root)
except ValueError:
    raise RuntimeError("profile_home must be inside Hermes home")
profile_home = profile_root

def assert_secure_profile():
    try:
        current = os.lstat(profile_home)
    except FileNotFoundError:
        raise RuntimeError("Hermes profile_home does not exist")
    if not stat.S_ISDIR(current.st_mode):
        raise RuntimeError("Hermes profile_home is not a directory")
    if current.st_uid != os.getuid() or (stat.S_IMODE(current.st_mode) & 0o022):
        raise RuntimeError("Hermes profile_home ownership or permissions are unsafe")
    cursor = pathlib.Path(profile_home.anchor)
    for component in profile_home.parts[1:]:
        cursor /= component
        parent = os.lstat(cursor)
        if stat.S_ISLNK(parent.st_mode) or not stat.S_ISDIR(parent.st_mode):
            raise RuntimeError("Hermes profile path contains an unsafe component")
        if parent.st_uid not in (0, os.getuid()) or (stat.S_IMODE(parent.st_mode) & 0o022):
            raise RuntimeError("Hermes profile parent ownership or permissions are unsafe")
    if (current.st_dev, current.st_ino) != profile_identity:
        raise RuntimeError("Hermes profile path changed during operation")

profile_identity = (profile_root.stat().st_dev, profile_root.stat().st_ino)
assert_secure_profile()
lock_path = profile_home / ".buzz-backend-hermes.lock"
lock_fd = os.open(str(lock_path), os.O_RDWR | os.O_CREAT | getattr(os, "O_NOFOLLOW", 0), 0o600)
os.fchmod(lock_fd, 0o600)
lock_file = os.fdopen(lock_fd, "a+")
fcntl.flock(lock_file.fileno(), fcntl.LOCK_EX)
provider_marker_start = "# BEGIN BUZZ BACKEND HERMES\n"
provider_marker_end = "# END BUZZ BACKEND HERMES"

def read_secure_file(path):
    assert_secure_profile()
    try:
        file_info = os.lstat(path)
    except FileNotFoundError:
        return None
    if stat.S_ISLNK(file_info.st_mode) or file_info.st_uid != os.getuid() or (stat.S_IMODE(file_info.st_mode) & 0o022):
        raise RuntimeError(f"{path.name} ownership or permissions are unsafe")
    try:
        fd = os.open(str(path), os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0))
    except FileNotFoundError:
        return None
    with os.fdopen(fd, "rb") as stream:
        return stream.read()

def secure_file_mode(path, default=0o600):
    try:
        return stat.S_IMODE(os.lstat(path).st_mode)
    except FileNotFoundError:
        return default

def remove_provider_blocks(content):
    while provider_marker_start in content and provider_marker_end in content:
        prefix, marked = content.split(provider_marker_start, 1)
        _, suffix = marked.split(provider_marker_end, 1)
        content = prefix + suffix
    return content

supervisor = str(cfg["supervisor"])
unit = str(cfg["unit"])
if operation in ("stop", "cleanup"):
    if supervisor == "systemd":
        result = subprocess.run(["systemctl", "--user", "stop", unit], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        if result.returncode:
            raise RuntimeError("Hermes gateway stop failed")
    elif supervisor == "launchd":
        target = f"gui/{os.getuid()}/{unit}"
        result = subprocess.run(["launchctl", "bootout", target], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        if result.returncode:
            status = subprocess.run(["launchctl", "print", target], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            if status.returncode == 0:
                raise RuntimeError("Hermes launchd gateway bootout failed")

        def launchd_has_pid():
            status = subprocess.run(["launchctl", "print", target], capture_output=True, text=True)
            return status.returncode == 0 and any(
                line.strip().startswith("pid = ") for line in status.stdout.splitlines()
            )

        deadline = time.monotonic() + 10
        while launchd_has_pid() and time.monotonic() < deadline:
            time.sleep(0.2)
        if launchd_has_pid():
            raise RuntimeError("Hermes launchd gateway did not stop")
    else:
        raise RuntimeError("unsupported supervisor")
    if operation == "stop":
        print(json.dumps({"ok": True, "agent_id": f"ssh://{cfg['host']}/{profile}"}, separators=(",", ":")))
        raise SystemExit(0)

    env_path = profile_home / ".env"
    old_bytes = read_secure_file(env_path)
    if old_bytes is not None:
        old = old_bytes.decode("utf-8", "surrogateescape")
        cleaned = remove_provider_blocks(old)
        if cleaned != old:
            assert_secure_profile()
            fd, tmp_name = tempfile.mkstemp(prefix=".env.", dir=str(profile_home))
            os.chmod(tmp_name, 0o600)
            with os.fdopen(fd, "wb") as stream:
                stream.write(cleaned.encode("utf-8", "surrogateescape"))
                stream.flush(); os.fsync(stream.fileno())
            assert_secure_profile()
            os.replace(tmp_name, env_path)
            os.chmod(env_path, 0o600)
    print(json.dumps({"ok": True, "agent_id": f"ssh://{cfg['host']}/{profile}"}, separators=(",", ":")))
    raise SystemExit(0)
if operation != "deploy":
    raise RuntimeError("unsupported provider operation")

private_key = str(agent.get("private_key_nsec") or "").strip()
auth_tag = str(agent.get("auth_tag") or "").strip()
relay_url = str(agent.get("relay_url") or "").strip()
if not private_key or not auth_tag or not relay_url:
    raise RuntimeError("identity payload is incomplete")
if str(agent.get("agent_command") or "").strip() != "hermes":
    raise RuntimeError("hermes provider refuses a non-Hermes agent command")
if any(any(char in value for char in ("\r", "\n")) for value in (private_key, auth_tag, relay_url)):
    raise RuntimeError("identity payload contains a newline")

def dotenv_value(name, value):
    if any(char in value for char in ("\x00", "\r", "\n")):
        raise RuntimeError(f"{name} contains a newline")
    return '"' + value.replace("\\", "\\\\").replace('"', '\\"') + '"'

channels = str(cfg.get("channels") or "a492f811-492b-5d55-b03f-81f9ff6107ea")
home_channel = str(cfg.get("home_channel") or channels.split(",", 1)[0])
cli_path = str(cfg.get("cli_path") or "buzz")
allowed_users = str(cfg.get("allowed_users") or "")
allow_all_users = cfg.get("allow_all_users") is True
env_path = profile_home / ".env"
old_env_bytes = read_secure_file(env_path)
old_env_mode = secure_file_mode(env_path)
old = old_env_bytes.decode("utf-8", "surrogateescape") if old_env_bytes is not None else ""
for marker_start, marker_end in (
    (provider_marker_start, provider_marker_end),
    ("# BEGIN RACKTAQ HERMES BUZZ\n", "# END RACKTAQ HERMES BUZZ"),
):
    while marker_start in old and marker_end in old:
        prefix, marked = old.split(marker_start, 1)
        _, suffix = marked.split(marker_end, 1)
        old = prefix + suffix
old = old.replace(provider_marker_end, "")
old = old.replace("# END RACKTAQ HERMES BUZZ", "")
block = provider_marker_start + "\n".join([
    f"BUZZ_PRIVATE_KEY={dotenv_value('private_key', private_key)}",
    f"BUZZ_AUTH_TAG={dotenv_value('auth_tag', auth_tag)}",
    f"BUZZ_RELAY_URL={dotenv_value('relay_url', relay_url)}",
    "BUZZ_TRANSPORT=websocket",
    f"BUZZ_CHANNELS={dotenv_value('channels', channels)}",
    f"BUZZ_HOME_CHANNEL={dotenv_value('home_channel', home_channel)}",
    f"BUZZ_ALLOWED_USERS={dotenv_value('allowed_users', allowed_users)}",
    f"BUZZ_ALLOW_ALL_USERS={'true' if allow_all_users else 'false'}",
    "BUZZ_REQUIRE_MENTION=true",
    f"BUZZ_CLI_PATH={dotenv_value('cli_path', cli_path)}",
    provider_marker_end,
]) + "\n"
config_path = profile_home / "config.yaml"
old_config_bytes = read_secure_file(config_path)
old_config_mode = secure_file_mode(config_path)

def restore_file(path, data, mode):
    assert_secure_profile()
    if data is None:
        try:
            path.unlink()
        except FileNotFoundError:
            pass
        return
    fd, tmp_name = tempfile.mkstemp(prefix=f".{path.name}.", dir=str(path.parent))
    os.chmod(tmp_name, mode)
    with os.fdopen(fd, "wb") as stream:
        stream.write(data)
        stream.flush(); os.fsync(stream.fileno())
    assert_secure_profile()
    os.replace(tmp_name, path)
    os.chmod(path, mode)

def restart_supervisor():
    if supervisor == "systemd":
        command = ["systemctl", "--user", "restart", unit]
    elif supervisor == "launchd":
        target = f"gui/{os.getuid()}/{unit}"
        enabled = subprocess.run(["launchctl", "enable", target], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        if enabled.returncode:
            raise RuntimeError("Hermes launchd gateway enable failed")
        if subprocess.run(["launchctl", "print", target], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL).returncode != 0:
            plist = pathlib.Path(str(cfg.get("plist") or f"~/Library/LaunchAgents/{unit}.plist")).expanduser()
            bootstrapped = subprocess.run(["launchctl", "bootstrap", f"gui/{os.getuid()}", str(plist)], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            if bootstrapped.returncode:
                raise RuntimeError("Hermes launchd gateway bootstrap failed")
        command = ["launchctl", "kickstart", "-k", target]
    else:
        raise RuntimeError("unsupported supervisor")
    return subprocess.run(command, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)

hermes = str(cfg.get("program") or "hermes")
hermes_env = {**os.environ, "HOME": str(home), "HERMES_HOME": str(profile_home)}
model = str(agent.get("model") or "").strip()
provider = str(agent.get("provider") or "").strip()
if not model or not provider:
    raise RuntimeError("model and provider are required")
restart_attempted = False
try:
    assert_secure_profile()
    fd, tmp_name = tempfile.mkstemp(prefix=".env.", dir=str(profile_home))
    os.chmod(tmp_name, 0o600)
    with os.fdopen(fd, "wb") as stream:
        prefix = old.rstrip()
        stream.write(((prefix + "\n" if prefix else "") + block).encode("utf-8", "surrogateescape"))
        stream.flush(); os.fsync(stream.fileno())
    assert_secure_profile()
    os.replace(tmp_name, env_path)
    os.chmod(env_path, 0o600)

    for key, value in (("model.default", model), ("model.provider", provider)):
        if value:
            assert_secure_profile()
            result = subprocess.run([hermes, "config", "set", key, value], env=hermes_env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            if result.returncode:
                raise RuntimeError(f"Hermes config update failed for {key}")

    assert_secure_profile()
    restart_attempted = True
    result = restart_supervisor()
    if result.returncode:
        raise RuntimeError("Hermes gateway restart failed")
except Exception:
    restore_file(env_path, old_env_bytes, old_env_mode)
    restore_file(config_path, old_config_bytes, old_config_mode)
    if restart_attempted:
        try:
            restart_supervisor()
        except Exception:
            pass
    raise

print(json.dumps({"ok": True, "agent_id": f"ssh://{cfg['host']}/{profile}"}, separators=(",", ":")))
"###;

fn main() {
    let mut input = String::new();
    if io::stdin().read_to_string(&mut input).is_err() {
        emit_error("could not read provider request");
        return;
    }
    let request: Value = match serde_json::from_str(&input) {
        Ok(value) => value,
        Err(_) => {
            emit_error("invalid JSON request");
            return;
        }
    };

    match request.get("op").and_then(Value::as_str) {
        Some("info") => emit_json(info_response()),
        Some("deploy") => match deploy(&request, "deploy") {
            Ok(response) => emit_json(response),
            Err(error) => emit_error(&error),
        },
        Some("stop") => match deploy(&request, "stop") {
            Ok(response) => emit_json(response),
            Err(error) => emit_error(&error),
        },
        Some("cleanup") => match deploy(&request, "cleanup") {
            Ok(response) => emit_json(response),
            Err(error) => emit_error(&error),
        },
        _ => emit_error("unsupported provider operation"),
    }
}

fn info_response() -> Value {
    json!({
        "ok": true,
        "name": "Native Hermes over SSH",
        "version": env!("CARGO_PKG_VERSION"),
        "protocol_version": PROTOCOL_VERSION,
        "description": "Updates an existing launchd/systemd Hermes gateway over SSH; never launches a local ACP runtime.",
        "config_schema": {
            "type": "object",
            "required": ["host", "user", "profile", "supervisor", "unit"],
            "properties": {
                "host": {"type": "string", "title": "Remote host"},
                "user": {"type": "string", "title": "SSH user"},
                "profile": {"type": "string", "title": "Hermes profile"},
                "supervisor": {"type": "string", "enum": ["launchd", "systemd"], "title": "Supervisor"},
                "unit": {"type": "string", "title": "Supervisor unit/label"},
                "plist": {"type": "string", "title": "launchd plist path"},
                "home": {"type": "string", "default": "~/.hermes", "title": "Hermes home"},
                "profile_home": {"type": "string", "title": "Profile home override"},
                "program": {"type": "string", "default": "hermes", "title": "Hermes executable"},
                "cli_path": {"type": "string", "default": "buzz", "title": "Buzz CLI path"},
                "channels": {"type": "string", "default": "a492f811-492b-5d55-b03f-81f9ff6107ea", "title": "Buzz channel UUIDs"},
                "home_channel": {"type": "string", "default": "a492f811-492b-5d55-b03f-81f9ff6107ea", "title": "Buzz home channel"},
                "allowed_users": {"type": "string", "default": "", "title": "Allowed relay users"},
                "allow_all_users": {"type": "boolean", "default": false, "title": "Allow all relay users"}
            }
        }
    })
}

fn deploy(request: &Value, operation: &str) -> Result<Value, String> {
    let cfg = request
        .get("provider_config")
        .and_then(Value::as_object)
        .ok_or_else(|| "provider_config must be an object".to_string())?;
    let host = required_string(cfg, "host")?;
    let profile = required_string(cfg, "profile")?;
    let supervisor = required_string(cfg, "supervisor")?;
    let unit = required_string(cfg, "unit")?;
    let user = required_string(cfg, "user")?;
    if !["launchd", "systemd"].contains(&supervisor.as_str()) {
        return Err("supervisor must be launchd or systemd".to_string());
    }
    validate_ssh_component("host", &host, ":%[]")?;
    if profile == "." || profile == ".." {
        return Err("provider_config.profile contains unsafe characters".to_string());
    }
    validate_ssh_component("profile", &profile, "")?;
    validate_ssh_component("user", &user, "")?;
    for (field, value) in [("profile", &profile), ("unit", &unit)] {
        if value.is_empty()
            || value.starts_with('-')
            || value.chars().any(|c| c.is_control() || c.is_whitespace())
        {
            return Err(format!(
                "provider_config.{field} contains unsafe characters"
            ));
        }
    }
    let empty_agent = Value::Object(Map::new());
    let agent = request.get("agent").unwrap_or(&empty_agent);
    if operation == "deploy" {
        if agent
            .get("private_key_nsec")
            .and_then(Value::as_str)
            .unwrap_or("")
            .is_empty()
        {
            return Err("agent identity is missing".to_string());
        }
        if agent
            .get("auth_tag")
            .and_then(Value::as_str)
            .unwrap_or("")
            .is_empty()
        {
            return Err("agent auth tag is missing".to_string());
        }
        if agent
            .get("relay_url")
            .and_then(Value::as_str)
            .unwrap_or("")
            .is_empty()
        {
            return Err("agent relay URL is missing".to_string());
        }
        for field in ["model", "provider"] {
            if agent
                .get(field)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .is_none()
            {
                return Err(format!("agent {field} is missing"));
            }
        }
    }

    let mut remote_config = cfg.clone();
    remote_config.insert("host".to_string(), Value::String(host.clone()));
    remote_config.insert("profile".to_string(), Value::String(profile.clone()));
    remote_config.insert("supervisor".to_string(), Value::String(supervisor));
    remote_config.insert("unit".to_string(), Value::String(unit));
    remote_config.insert("user".to_string(), Value::String(user.clone()));
    let mut remote_request = json!({
        "op": operation,
        "provider_config": remote_config,
    });
    if operation == "deploy" {
        remote_request["agent"] = agent.clone();
    }
    run_ssh(&host, Some(&user), &remote_request)
}

fn run_ssh(host: &str, user: Option<&str>, request: &Value) -> Result<Value, String> {
    let target = match user.filter(|value| !value.is_empty()) {
        Some(user) => format!("{user}@{host}"),
        None => host.to_string(),
    };
    let mut command = Command::new("ssh");
    let encoded_script = STANDARD.encode(REMOTE_SCRIPT);
    let remote_command =
        format!("python3 -c 'import base64;exec(base64.b64decode(\"{encoded_script}\"))'");
    command.args([
        "-o",
        "BatchMode=yes",
        "-o",
        "ConnectTimeout=10",
        "-o",
        "ServerAliveInterval=5",
        "-o",
        "ServerAliveCountMax=2",
        "--",
        &target,
        &remote_command,
    ]);
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| "could not start ssh".to_string())?;
    let body =
        serde_json::to_vec(request).map_err(|_| "could not encode remote request".to_string())?;
    child
        .stdin
        .take()
        .ok_or_else(|| "ssh stdin unavailable".to_string())?
        .write_all(&body)
        .map_err(|_| "could not send remote request".to_string())?;
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(50)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("remote Hermes deployment timed out".to_string());
            }
            Err(_) => return Err("ssh execution failed".to_string()),
        }
    }
    let output = child
        .wait_with_output()
        .map_err(|_| "ssh execution failed".to_string())?;
    if !output.status.success() {
        return Err("remote Hermes deployment failed".to_string());
    }
    let response: Value = serde_json::from_slice(&output.stdout)
        .map_err(|_| "remote Hermes returned invalid provider output".to_string())?;
    if response.get("ok") != Some(&Value::Bool(true)) {
        return Err("remote Hermes deployment was rejected".to_string());
    }
    Ok(response)
}

fn validate_ssh_component(field: &str, value: &str, extra: &str) -> Result<(), String> {
    if value.is_empty()
        || value.starts_with('-')
        || value.chars().any(|character| {
            character.is_control()
                || character.is_whitespace()
                || !(character.is_ascii_alphanumeric()
                    || ".-_".contains(character)
                    || extra.contains(character))
        })
    {
        return Err(format!(
            "provider_config.{field} contains unsafe characters"
        ));
    }
    Ok(())
}

fn required_string(config: &Map<String, Value>, key: &str) -> Result<String, String> {
    config
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .ok_or_else(|| format!("provider_config.{key} is required"))
}

fn emit_json(value: Value) {
    println!(
        "{}",
        serde_json::to_string(&value).unwrap_or_else(|_| "{\"ok\":false}".to_string())
    );
}

fn emit_error(message: &str) {
    emit_json(json!({"ok": false, "error": message}));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn info_schema_has_required_non_secret_fields() {
        let response = info_response();
        let properties = response["config_schema"]["properties"].as_object().unwrap();
        assert!(properties.contains_key("host"));
        assert!(properties.contains_key("profile"));
        assert!(properties.contains_key("allow_all_users"));
        assert!(!properties
            .keys()
            .any(|key| key.contains("key") || key.contains("secret")));
    }

    #[test]
    fn missing_provider_config_is_rejected_before_ssh() {
        let error = deploy(&json!({"op": "deploy", "agent": {}}), "deploy").unwrap_err();
        assert!(error.contains("provider_config"));
    }

    #[test]
    fn unsafe_ssh_target_is_rejected() {
        let error = deploy(
            &json!({
                "op": "stop",
                "provider_config": {
                    "host": "-oProxyCommand=touch /tmp/pwned",
                    "user": "karsten",
                    "profile": "default",
                    "supervisor": "launchd",
                    "unit": "ai.hermes.gateway"
                }
            }),
            "stop",
        )
        .unwrap_err();
        assert!(error.contains("host") && error.contains("unsafe"));
    }

    #[test]
    fn unsafe_profile_is_rejected() {
        let error = deploy(
            &json!({
                "op": "stop",
                "provider_config": {
                    "host": "example",
                    "user": "karsten",
                    "profile": "../.ssh",
                    "supervisor": "launchd",
                    "unit": "ai.hermes.gateway"
                }
            }),
            "stop",
        )
        .unwrap_err();
        assert!(error.contains("profile") && error.contains("unsafe"));
    }

    #[test]
    fn deploy_requires_model_and_provider() {
        let error = deploy(
            &json!({
                "op": "deploy",
                "agent": {
                    "private_key_nsec": "nsec1x",
                    "auth_tag": "[\"auth\"]",
                    "relay_url": "wss://relay"
                },
                "provider_config": {
                    "host": "example",
                    "user": "karsten",
                    "profile": "default",
                    "supervisor": "launchd",
                    "unit": "ai.hermes.gateway"
                }
            }),
            "deploy",
        )
        .unwrap_err();
        assert!(error.contains("model"));
    }

    #[test]
    fn unsafe_unit_is_rejected() {
        let error = deploy(&json!({
            "op": "deploy",
            "agent": {"private_key_nsec": "nsec1x", "auth_tag": "[\"auth\"]", "relay_url": "wss://relay"},
            "provider_config": {"host": "example", "user": "karsten", "profile": "default", "supervisor": "launchd", "unit": "bad unit"}
        }), "deploy").unwrap_err();
        assert!(error.contains("unsafe"));
    }
}
