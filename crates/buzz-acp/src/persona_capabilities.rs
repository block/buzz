//! Delivery of explicitly assigned native skills and bounded Buzz lifecycle hooks.
//! This module does not claim filesystem isolation or change native hook policy.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use buzz_persona::capabilities::PersonaCapabilities;
use buzz_persona::persona::Hooks;

/// Diagnostic copies must not publish MCP credentials or token-bearing arguments.
/// The actual adapter wire value remains unchanged.
pub(crate) fn diagnostic_wire_value(value: &serde_json::Value) -> serde_json::Value {
    let mut safe = value.clone();
    if safe["method"] == "session/new" {
        if let Some(servers) = safe.pointer_mut("/params/mcpServers") {
            *servers = serde_json::json!("[redacted MCP configuration]");
        }
    }
    safe
}

pub(crate) struct Delivery {
    pub cwd: String,
    pub plugins: Vec<String>,
    pub codex_roots: Vec<String>,
    pub opencode_skill_root: Option<String>,
    pub hooks: Option<Hooks>,
    _plugin: Option<tempfile::TempDir>,
    pub started: bool,
}

impl Delivery {
    pub fn prepare(config: &PersonaCapabilities, command: &str, default: &Path) -> Result<Self> {
        config.validate().map_err(anyhow::Error::msg)?;
        let cwd = config.resolve_cwd(default).map_err(anyhow::Error::msg)?;
        let hooks = config
            .hooks
            .as_ref()
            .map(|h| -> Result<Hooks> {
                let resolve = |p: &Option<String>| -> Result<Option<String>> {
                    p.as_ref()
                        .map(|p| {
                            let path =
                                cwd.join(p).canonicalize().context("missing persona hook")?;
                            if !path.is_file() {
                                return Err(anyhow!("persona hook must be a regular file"));
                            }
                            Ok(path.to_string_lossy().into_owned())
                        })
                        .transpose()
                };
                Ok(Hooks {
                    on_start: resolve(&h.on_start)?,
                    on_stop: resolve(&h.on_stop)?,
                    on_message: resolve(&h.on_message)?,
                })
            })
            .transpose()?;
        let mut delivery = Self {
            cwd: cwd.to_string_lossy().into_owned(),
            plugins: Vec::new(),
            codex_roots: Vec::new(),
            opencode_skill_root: None,
            hooks,
            _plugin: None,
            started: false,
        };
        if config.skills.is_empty() {
            return Ok(delivery);
        }
        let runtime = crate::config::normalize_agent_command_identity(command);
        let relative_skills = match runtime.as_str() {
            "claude-agent-acp" | "opencode" => "skills",
            "codex-acp" => ".agents/skills",
            _ => {
                return Err(anyhow!(
                    "assigned skill delivery is not yet supported for this harness"
                ))
            }
        };
        let plugin = tempfile::Builder::new()
            .prefix("buzz-persona-skills-")
            .tempdir()?;
        if runtime == "claude-agent-acp" {
            std::fs::create_dir(plugin.path().join(".claude-plugin"))?;
            std::fs::write(
                plugin.path().join(".claude-plugin/plugin.json"),
                b"{\"name\":\"buzz-persona\",\"version\":\"1.0.0\"}",
            )?;
        }
        let skills_root = plugin.path().join(relative_skills);
        std::fs::create_dir_all(&skills_root)?;
        let mut names = std::collections::BTreeSet::new();
        for reference in &config.skills {
            let path = cwd
                .join(reference)
                .canonicalize()
                .with_context(|| format!("missing assigned skill: {reference}"))?;
            let name = path
                .file_name()
                .and_then(|s| s.to_str())
                .ok_or_else(|| anyhow!("skill directory has no valid name"))?;
            if !path.is_dir() || !path.join("SKILL.md").is_file() {
                return Err(anyhow!("assigned skill {reference} must contain SKILL.md"));
            }
            if !names.insert(name.to_owned()) {
                return Err(anyhow!(
                    "assigned skills have duplicate directory name: {name}"
                ));
            }
            // Native metadata (including allowed-tools, Unicode and native hooks)
            // is deliberately untouched. Referenced scripts remain beside SKILL.md.
            #[cfg(unix)]
            std::os::unix::fs::symlink(&path, skills_root.join(name))?;
            #[cfg(not(unix))]
            return Err(anyhow!(
                "native persona skill links are not supported on this platform yet"
            ));
        }
        let root = plugin.path().to_string_lossy().into_owned();
        match runtime.as_str() {
            "claude-agent-acp" => delivery.plugins.push(root),
            "codex-acp" => delivery.codex_roots.push(root),
            "opencode" => {
                delivery.opencode_skill_root = Some(skills_root.to_string_lossy().into_owned())
            }
            _ => return Err(anyhow!("unsupported native skill delivery")),
        }
        delivery._plugin = Some(plugin);
        Ok(delivery)
    }
}

/// Append persona skill roots to OpenCode's native inline configuration, retaining
/// the operator's other fields and both supported skill configuration forms.
pub(crate) fn opencode_config_with_skills(base: Option<&str>, root: &str) -> Result<String> {
    let mut value: serde_json::Value = match base {
        Some(text) => serde_json::from_str(text).context("invalid OPENCODE_CONFIG_CONTENT JSON")?,
        None => serde_json::json!({}),
    };
    let object = value
        .as_object_mut()
        .ok_or_else(|| anyhow!("OpenCode inline config must be an object"))?;
    let skills = object
        .entry("skills")
        .or_insert_with(|| serde_json::json!({"paths": []}));
    let paths = if skills.is_array() {
        skills.as_array_mut()
    } else if let Some(object) = skills.as_object_mut() {
        object
            .entry("paths")
            .or_insert_with(|| serde_json::json!([]))
            .as_array_mut()
    } else {
        None
    }
    .ok_or_else(|| anyhow!("OpenCode skills must be an array or an object with paths"))?;
    if paths.iter().any(|v| !v.is_string()) {
        return Err(anyhow!("OpenCode skill paths must be strings"));
    }
    if !paths.iter().any(|v| v.as_str() == Some(root)) {
        paths.push(serde_json::json!(root));
    }
    Ok(serde_json::to_string(&value)?)
}

/// Script execution is bounded and does not interpret stdout as prompt text.
pub(crate) fn run_hook(path: &str, event: &str, cwd: &str, input: &[u8]) -> Result<()> {
    if input.len() > 1024 * 1024 {
        return Err(anyhow!("persona hook input exceeds 1 MiB"));
    }
    let mut command = Command::new("/bin/sh");
    command
        .arg(path)
        .current_dir(cwd)
        .env("AGENT_CWD", cwd)
        .env("BUZZ_HOOK_EVENT", event)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = command.spawn().context("cannot start persona hook")?;
    let pid = child.id();
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| anyhow!("missing hook stdin"))?;
    let input = input.to_vec();
    let writer = std::thread::spawn(move || {
        let _ = stdin.write_all(&input);
    });
    fn drain(stream: Option<impl Read + Send + 'static>) -> std::thread::JoinHandle<Vec<u8>> {
        std::thread::spawn(move || {
            let mut out = Vec::new();
            let mut buffer = [0; 4096];
            if let Some(mut stream) = stream {
                while let Ok(n) = stream.read(&mut buffer) {
                    if n == 0 {
                        break;
                    }
                    let keep = n.min(16384usize.saturating_sub(out.len()));
                    out.extend_from_slice(&buffer[..keep]);
                }
            }
            out
        })
    }
    let stdout = drain(child.stdout.take());
    let stderr = drain(child.stderr.take());
    let deadline = Instant::now() + Duration::from_secs(5);
    let result = loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                break if status.success() {
                    Ok(())
                } else {
                    Err(anyhow!("persona {event} hook exited unsuccessfully"))
                }
            }
            Err(error) => break Err(error.into()),
            Ok(None) if Instant::now() >= deadline => {
                break Err(anyhow!("persona {event} hook exceeded 5 seconds"))
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
        }
    };
    #[cfg(unix)]
    if let Ok(pid) = i32::try_from(pid) {
        let _ = nix::sys::signal::killpg(
            nix::unistd::Pid::from_raw(pid),
            nix::sys::signal::Signal::SIGKILL,
        );
    }
    let _ = child.kill();
    let _ = child.wait();
    // Do not join a reader held open by a deliberately detached descendant.
    // Hook execution is at the owner's trust level, not an OS sandbox.
    for reader in [stdout, stderr] {
        if reader.is_finished() {
            if let Ok(bytes) = reader.join() {
                tracing::debug!(
                    event,
                    captured_bytes = bytes.len(),
                    "persona hook output discarded (not injected)"
                );
            }
        }
    }
    if writer.is_finished() {
        let _ = writer.join();
    }
    result
}

impl Drop for Delivery {
    fn drop(&mut self) {
        if !self.started {
            return;
        }
        if let Some(path) = self.hooks.as_ref().and_then(|h| h.on_stop.as_deref()) {
            if let Err(error) = run_hook(path, "on_stop", &self.cwd, b"") {
                tracing::warn!(%error,"persona cleanup hook failed");
            }
        }
    }
}

/// Resolve the configured path without reading a home directory. Also implements
/// the explicit AGENT_CWD override proposed by rsaulo in upstream PR #4558.
pub(crate) fn resolve_cwd(config: &PersonaCapabilities) -> Result<PathBuf> {
    let default = match std::env::var("AGENT_CWD") {
        Ok(value) if !value.trim().is_empty() => PathBuf::from(value.trim()),
        _ => std::env::current_dir()?,
    };
    config.resolve_cwd(&default).map_err(anyhow::Error::msg)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn unassigned_unsupported_harness_keeps_its_existing_working_directory() {
        let root = tempfile::tempdir().unwrap();
        for command in ["goose", "custom-acp"] {
            let delivery =
                Delivery::prepare(&PersonaCapabilities::default(), command, root.path()).unwrap();
            assert_eq!(Path::new(&delivery.cwd), root.path());
            assert!(delivery.plugins.is_empty());
            assert!(delivery.codex_roots.is_empty());
            assert!(delivery.opencode_skill_root.is_none());
        }
    }

    #[test]
    fn native_plugin_preserves_crlf_unicode_metadata_scripts_and_cleans_up() {
        let root = tempfile::tempdir().unwrap();
        let skill = root.path().join("priorities");
        std::fs::create_dir(&skill).unwrap();
        let body = "---\r\nname: priorities\r\ndescription: Résumé 日本語\r\nallowed-tools: Read, Bash\r\nuser-invocable: true\r\n---\r\nUse ./helper.sh.\r\n";
        std::fs::write(skill.join("SKILL.md"), body).unwrap();
        std::fs::write(skill.join("helper.sh"), "printf hello").unwrap();
        let config = PersonaCapabilities {
            skills: vec!["priorities".into()],
            ..Default::default()
        };
        let delivery = Delivery::prepare(&config, "claude-agent-acp", root.path()).unwrap();
        let plugin = PathBuf::from(&delivery.plugins[0]);
        assert_eq!(
            std::fs::read_to_string(plugin.join("skills/priorities/SKILL.md")).unwrap(),
            body
        );
        assert_eq!(
            std::fs::read_to_string(plugin.join("skills/priorities/helper.sh")).unwrap(),
            "printf hello"
        );
        assert!(!root.path().join(".claude").exists());
        drop(delivery);
        assert!(!plugin.exists());
        assert!(skill.join("SKILL.md").exists());
    }

    #[test]
    fn missing_skill_hook_cwd_and_unsupported_harness_are_explicit_errors() {
        let root = tempfile::tempdir().unwrap();
        let mut config = PersonaCapabilities {
            skills: vec!["missing".into()],
            ..Default::default()
        };
        assert!(Delivery::prepare(&config, "claude-agent-acp", root.path())
            .err()
            .unwrap()
            .to_string()
            .contains("missing assigned skill"));
        assert!(Delivery::prepare(&config, "unsupported-acp", root.path())
            .err()
            .unwrap()
            .to_string()
            .contains("not yet supported"));
        config.skills.clear();
        config.hooks = Some(Hooks {
            on_start: Some("absent.sh".into()),
            on_stop: None,
            on_message: None,
        });
        assert!(Delivery::prepare(&config, "claude-agent-acp", root.path()).is_err());
        config.hooks = None;
        config.working_directory = Some(root.path().join("absent").to_string_lossy().into_owned());
        assert!(Delivery::prepare(&config, "claude-agent-acp", root.path()).is_err());
    }

    #[test]
    fn codex_and_opencode_deliver_native_skill_roots_without_changing_cwd() {
        let root = tempfile::tempdir().unwrap();
        let skill = root.path().join("assignment");
        std::fs::create_dir(&skill).unwrap();
        std::fs::write(
            skill.join("SKILL.md"),
            "---\nname: assignment\ndescription: Assigned skill\n---\nUse MCP.",
        )
        .unwrap();
        let config = PersonaCapabilities {
            skills: vec!["assignment".into()],
            ..Default::default()
        };
        for command in ["codex-acp", "opencode"] {
            let delivery = Delivery::prepare(&config, command, root.path()).unwrap();
            // No working directory is assigned, so the launch directory is unchanged.
            assert_eq!(Path::new(&delivery.cwd), root.path());
            assert!(delivery.plugins.is_empty());
            let path = if command == "codex-acp" {
                PathBuf::from(&delivery.codex_roots[0]).join(".agents/skills")
            } else {
                PathBuf::from(delivery.opencode_skill_root.as_ref().unwrap())
            };
            assert_eq!(
                path.join("assignment/SKILL.md").canonicalize().unwrap(),
                skill.join("SKILL.md").canonicalize().unwrap()
            );
            drop(delivery);
            assert!(!path.exists());
            assert!(skill.join("SKILL.md").exists());
        }
        assert!(!root.path().join(".agents").exists());
        assert!(!root.path().join("opencode.json").exists());
    }

    #[test]
    fn opencode_inline_config_preserves_settings_and_appends_skills() {
        for skills in [
            serde_json::json!(["/global"]),
            serde_json::json!({"paths":["/global"],"urls":["https://example.test/skills"]}),
        ] {
            let base = serde_json::json!({"model":"openrouter/deepseek/deepseek-v3.2","skills":skills,"permission":{"bash":"ask"}});
            let encoded = opencode_config_with_skills(Some(&base.to_string()), "/persona").unwrap();
            let twice = opencode_config_with_skills(Some(&encoded), "/persona").unwrap();
            assert_eq!(encoded, twice);
            let value: serde_json::Value = serde_json::from_str(&encoded).unwrap();
            assert_eq!(value["model"], base["model"]);
            assert_eq!(value["permission"], base["permission"]);
            let paths = value["skills"]
                .as_array()
                .or_else(|| value["skills"]["paths"].as_array())
                .unwrap();
            assert_eq!(
                paths,
                &vec![serde_json::json!("/global"), serde_json::json!("/persona")]
            );
        }
        for bad in [
            "[]",
            "null",
            "{",
            "{\"skills\":false}",
            "{\"skills\":{\"paths\":42}}",
            "{\"skills\":[7]}",
        ] {
            assert!(opencode_config_with_skills(Some(bad), "/persona").is_err());
        }
    }

    #[test]
    fn lifecycle_has_exact_input_and_event_without_injecting_output() {
        let root = tempfile::tempdir().unwrap();
        let hook = root.path().join("hook.sh");
        std::fs::write(
            &hook,
            "printf '%s\\n' \"$BUZZ_HOOK_EVENT\" >> events\ncat > input\nprintf 'not injected'\n",
        )
        .unwrap();
        let config = PersonaCapabilities {
            hooks: Some(Hooks {
                on_start: Some("hook.sh".into()),
                on_stop: Some("hook.sh".into()),
                on_message: Some("hook.sh".into()),
            }),
            ..Default::default()
        };
        let mut delivery = Delivery::prepare(&config, "claude-agent-acp", root.path()).unwrap();
        run_hook(hook.to_str().unwrap(), "on_start", &delivery.cwd, b"").unwrap();
        delivery.started = true;
        run_hook(
            hook.to_str().unwrap(),
            "on_message",
            &delivery.cwd,
            "hello 日本語".as_bytes(),
        )
        .unwrap();
        assert_eq!(
            std::fs::read_to_string(root.path().join("input")).unwrap(),
            "hello 日本語"
        );
        drop(delivery);
        assert_eq!(
            std::fs::read_to_string(root.path().join("events")).unwrap(),
            "on_start\non_message\non_stop\n"
        );
    }

    #[test]
    fn failed_and_timed_out_hooks_do_not_succeed() {
        let root = tempfile::tempdir().unwrap();
        let hook = root.path().join("hook.sh");
        std::fs::write(&hook, "exit 7\n").unwrap();
        assert!(run_hook(
            hook.to_str().unwrap(),
            "on_message",
            root.path().to_str().unwrap(),
            b"prompt"
        )
        .unwrap_err()
        .to_string()
        .contains("unsuccessfully"));
        std::fs::write(
            &hook,
            "trap '' TERM\nsleep 30 &\necho $! > child.pid\nwait\n",
        )
        .unwrap();
        let start = Instant::now();
        assert!(run_hook(
            hook.to_str().unwrap(),
            "on_message",
            root.path().to_str().unwrap(),
            b"prompt"
        )
        .unwrap_err()
        .to_string()
        .contains("exceeded 5 seconds"));
        assert!(start.elapsed() < Duration::from_secs(8));
        let pid = std::fs::read_to_string(root.path().join("child.pid")).unwrap();
        let child = nix::unistd::Pid::from_raw(pid.trim().parse().unwrap());
        let deadline = Instant::now() + Duration::from_secs(2);
        while nix::sys::signal::kill(child, None).is_ok() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(
            nix::sys::signal::kill(child, None),
            Err(nix::errno::Errno::ESRCH)
        );
    }
}
