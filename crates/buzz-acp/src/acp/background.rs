//! What an agent process is still waiting on after its turn has ended.
//!
//! Claude Code keeps working after a turn settles. Background shells,
//! subagents and monitors run on, and session crons (`CronCreate`,
//! `ScheduleWakeup`, `/loop`) wake the session later. Each of these starts a
//! model cycle of its own, but only while the agent process is alive. The idle
//! reapers ask [`BackgroundWork::pending`] and [`StopHookDir::crons`] before
//! they tear a process down.
//!
//! Two sources report this work:
//!
//! - claude-agent-acp forwards raw SDK messages as `_claude/sdkMessage`
//!   notifications when `session/new` asks for them ([`emit_raw_filter`]).
//!   `background_tasks_changed` carries the full set of live tasks after every
//!   change, and `session_state_changed` says whether the session is running a
//!   cycle right now (for example the one a finished background task starts).
//! - Session crons appear only in the input of the Stop hook. A Stop hook that
//!   runs `buzz-acp stop-hook --dir <dir>` (see [`run_stop_hook`]) writes the
//!   cron list of each session to a file in a per-process directory.
//!
//! Adapters that send neither leave both empty, so the reapers behave as before.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde_json::Value;

/// ACP extension notification that carries one raw Claude Agent SDK message.
const SDK_MESSAGE_METHOD: &str = "_claude/sdkMessage";

/// Name of the subcommand the Stop hook runs.
pub(crate) const STOP_HOOK_SUBCOMMAND: &str = "stop-hook";

/// The raw SDK messages buzz-acp asks claude-agent-acp to forward, as the
/// value of `_meta.claudeCode.emitRawSDKMessages` on `session/new`.
pub(crate) fn emit_raw_filter() -> Value {
    serde_json::json!([
        { "type": "system", "subtype": "background_tasks_changed" },
        { "type": "system", "subtype": "session_state_changed" },
    ])
}

/// Live background work per session, fed from the agent's stdout.
#[derive(Debug, Default)]
pub(crate) struct BackgroundWork {
    sessions: HashMap<String, SessionActivity>,
    /// Set once the agent closed stdout. A dead process waits on nothing.
    exited: bool,
}

#[derive(Debug, Default)]
struct SessionActivity {
    /// Description of every live background task, from the last
    /// `background_tasks_changed`.
    tasks: Vec<String>,
    /// Whether the session is in a model cycle, from `session_state_changed`.
    running: bool,
}

impl BackgroundWork {
    /// Record `line` if it is a `_claude/sdkMessage` notification.
    ///
    /// Returns `true` when it was one. Those notifications exist only for this
    /// bookkeeping, so the caller drops them instead of passing them on.
    pub(crate) fn observe_line(&mut self, line: &str) -> bool {
        if !line.contains(SDK_MESSAGE_METHOD) {
            return false;
        }
        let Ok(msg) = serde_json::from_str::<Value>(line) else {
            return false;
        };
        if msg.get("method").and_then(Value::as_str) != Some(SDK_MESSAGE_METHOD) {
            return false;
        }
        let params = &msg["params"];
        let message = &params["message"];
        let Some(session_id) = params["sessionId"]
            .as_str()
            .or_else(|| message["session_id"].as_str())
        else {
            return true;
        };
        match message["subtype"].as_str() {
            Some("background_tasks_changed") => {
                // REPLACE semantics: the payload is the whole live set.
                let tasks = message["tasks"]
                    .as_array()
                    .map(|tasks| tasks.iter().map(describe_task).collect())
                    .unwrap_or_default();
                self.sessions
                    .entry(session_id.to_owned())
                    .or_default()
                    .tasks = tasks;
            }
            Some("session_state_changed") => {
                let running = message["state"].as_str() == Some("running");
                self.sessions
                    .entry(session_id.to_owned())
                    .or_default()
                    .running = running;
            }
            _ => {}
        }
        self.sessions
            .retain(|_, activity| activity.running || !activity.tasks.is_empty());
        true
    }

    /// Record that the agent closed stdout. Its last reports describe work
    /// that died with it.
    pub(crate) fn mark_exited(&mut self) {
        self.exited = true;
        self.sessions.clear();
    }

    /// Whether the agent closed stdout.
    pub(crate) fn exited(&self) -> bool {
        self.exited
    }

    /// What the process is still working on, one line per item. Empty when
    /// nothing is pending.
    pub(crate) fn pending(&self) -> Vec<String> {
        let mut pending = Vec::new();
        for activity in self.sessions.values() {
            if activity.running {
                pending.push("a model cycle the agent started on its own".to_owned());
            }
            pending.extend(activity.tasks.iter().cloned());
        }
        pending
    }
}

fn describe_task(task: &Value) -> String {
    let kind = task["task_type"].as_str().unwrap_or("task");
    match task["description"].as_str().filter(|d| !d.is_empty()) {
        Some(description) => format!("{kind}: {description}"),
        None => kind.to_owned(),
    }
}

/// Per-process directory the Stop hook writes session crons into. Removed on
/// drop.
#[derive(Debug)]
pub(crate) struct StopHookDir {
    path: PathBuf,
}

impl StopHookDir {
    /// Create a fresh directory under the system temp dir, readable only by
    /// this user: the cron prompts it holds are the agent's own words.
    pub(crate) fn create() -> std::io::Result<Self> {
        let path =
            std::env::temp_dir().join(format!("buzz-acp-stop-hook-{}", uuid::Uuid::new_v4()));
        let mut builder = std::fs::DirBuilder::new();
        #[cfg(unix)]
        std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
        builder.create(&path)?;
        Ok(Self { path })
    }

    #[cfg(test)]
    fn path(&self) -> &Path {
        &self.path
    }

    /// The `_meta.claudeCode.options.settings` value that installs the Stop
    /// hook. `exe` is the buzz-acp binary the hook runs.
    ///
    /// `model_config` is the agent's `CLAUDE_MODEL_CONFIG`, if set. Older
    /// claude-agent-acp releases apply it only when the client passes no
    /// settings, so its keys are carried over here.
    pub(crate) fn settings(&self, exe: &Path, model_config: Option<&str>) -> Value {
        let command = format!(
            "{} {STOP_HOOK_SUBCOMMAND} --dir {}",
            shell_quote(&exe.to_string_lossy()),
            shell_quote(&self.path.to_string_lossy()),
        );
        let mut settings = model_config
            .and_then(|config| serde_json::from_str::<Value>(config).ok())
            .filter(Value::is_object)
            .unwrap_or_else(|| serde_json::json!({}));
        let hook = serde_json::json!({ "hooks": [{ "type": "command", "command": command }] });
        match settings["hooks"]["Stop"].as_array_mut() {
            Some(stop) => stop.push(hook),
            None => settings["hooks"]["Stop"] = Value::Array(vec![hook]),
        }
        settings
    }

    /// The scheduled wakeups of every session, one line per cron. A session
    /// whose last Stop hook listed no crons contributes nothing.
    pub(crate) fn crons(&self) -> Vec<String> {
        let Ok(entries) = std::fs::read_dir(&self.path) else {
            return Vec::new();
        };
        let mut crons = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            let Ok(record) = serde_json::from_str::<Value>(&text) else {
                continue;
            };
            for cron in record["session_crons"].as_array().into_iter().flatten() {
                crons.push(describe_cron(cron));
            }
        }
        crons
    }
}

impl Drop for StopHookDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn describe_cron(cron: &Value) -> String {
    let schedule = cron["schedule"].as_str().unwrap_or("?");
    let kind = if cron["recurring"].as_bool() == Some(true) {
        "recurring wakeup"
    } else {
        "wakeup"
    };
    match cron["prompt"].as_str().filter(|p| !p.is_empty()) {
        Some(prompt) => format!("{kind} `{schedule}`: {}", truncate(prompt, 120)),
        None => format!("{kind} `{schedule}`"),
    }
}

fn truncate(text: &str, max_chars: usize) -> String {
    let mut chars = text.chars();
    let head: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{head}…")
    } else {
        head
    }
}

/// Quote `value` for the POSIX shell Claude Code runs hook commands in.
fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', r"'\''"))
}

/// Entry point of `buzz-acp stop-hook --dir <dir>`.
///
/// Reads the Stop hook input from stdin and writes the session's crons to
/// `<dir>/<session_id>.json`. Always exits 0 and prints nothing: a failing or
/// talkative Stop hook would change what the agent does next.
pub(crate) fn run_stop_hook() -> i32 {
    let args: Vec<String> = std::env::args().collect();
    let Some(dir) = args
        .iter()
        .position(|arg| arg == "--dir")
        .and_then(|i| args.get(i + 1))
    else {
        return 0;
    };
    let mut input = String::new();
    if std::io::stdin().read_to_string(&mut input).is_err() {
        return 0;
    }
    let _ = record_stop_hook_input(Path::new(dir), &input);
    0
}

fn record_stop_hook_input(dir: &Path, input: &str) -> std::io::Result<()> {
    let hook: Value = serde_json::from_str(input).map_err(std::io::Error::other)?;
    let session_id = hook["session_id"]
        .as_str()
        .filter(|id| !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'))
        .ok_or_else(|| std::io::Error::other("stop hook input has no usable session_id"))?;
    let record = serde_json::json!({
        "session_crons": hook.get("session_crons").cloned().unwrap_or(Value::Array(Vec::new())),
    });
    // Write then rename, so a reader never sees half a file.
    let tmp = dir.join(format!("{session_id}.json.tmp"));
    std::fs::write(&tmp, record.to_string())?;
    std::fs::rename(&tmp, dir.join(format!("{session_id}.json")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sdk_message(session: &str, message: Value) -> String {
        serde_json::json!({
            "jsonrpc": "2.0",
            "method": SDK_MESSAGE_METHOD,
            "params": { "sessionId": session, "message": message },
        })
        .to_string()
    }

    #[test]
    fn background_tasks_replace_the_live_set() {
        let mut work = BackgroundWork::default();
        assert!(work.observe_line(&sdk_message(
            "s1",
            serde_json::json!({
                "type": "system",
                "subtype": "background_tasks_changed",
                "tasks": [{ "task_id": "b1", "task_type": "local_bash", "description": "wait for CI" }],
            }),
        )));
        assert_eq!(work.pending(), vec!["local_bash: wait for CI".to_owned()]);

        assert!(work.observe_line(&sdk_message(
            "s1",
            serde_json::json!({ "type": "system", "subtype": "background_tasks_changed", "tasks": [] }),
        )));
        assert!(work.pending().is_empty());
    }

    #[test]
    fn a_running_cycle_is_pending_until_idle() {
        let mut work = BackgroundWork::default();
        let state = |state: &str| {
            sdk_message(
                "s1",
                serde_json::json!({ "type": "system", "subtype": "session_state_changed", "state": state }),
            )
        };
        work.observe_line(&state("running"));
        assert_eq!(work.pending().len(), 1);
        work.observe_line(&state("idle"));
        assert!(work.pending().is_empty());
    }

    #[test]
    fn other_lines_pass_through() {
        let mut work = BackgroundWork::default();
        let update = r#"{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"s1","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"_claude/sdkMessage"}}}}"#;
        assert!(!work.observe_line(update));
        assert!(!work.observe_line("not json _claude/sdkMessage"));
        assert!(work.pending().is_empty());
    }

    #[test]
    fn stop_hook_records_crons_per_session() {
        let dir = StopHookDir::create().unwrap();
        let input = serde_json::json!({
            "session_id": "d9367-4cd1",
            "hook_event_name": "Stop",
            "background_tasks": [],
            "session_crons": [{ "id": "7996a899", "schedule": "43 10 1 11 *", "recurring": false, "prompt": "check CI" }],
        });
        record_stop_hook_input(dir.path(), &input.to_string()).unwrap();
        assert_eq!(
            dir.crons(),
            vec!["wakeup `43 10 1 11 *`: check CI".to_owned()]
        );

        // The next Stop hook of the same session replaces the record.
        let input = serde_json::json!({ "session_id": "d9367-4cd1", "session_crons": [] });
        record_stop_hook_input(dir.path(), &input.to_string()).unwrap();
        assert!(dir.crons().is_empty());
    }

    #[test]
    fn stop_hook_rejects_a_session_id_that_is_a_path() {
        let dir = StopHookDir::create().unwrap();
        let input = serde_json::json!({ "session_id": "../escape", "session_crons": [] });
        assert!(record_stop_hook_input(dir.path(), &input.to_string()).is_err());
    }

    #[test]
    fn hook_settings_keep_the_model_config() {
        let dir = StopHookDir::create().unwrap();
        let config = r#"{"modelOverrides":{"opus":"bedrock-opus"}}"#;
        let settings = dir.settings(Path::new("/bin/buzz-acp"), Some(config));
        assert_eq!(settings["modelOverrides"]["opus"], "bedrock-opus");
        assert!(settings["hooks"]["Stop"][0]["hooks"][0]["command"].is_string());
        // An unusable config is ignored, never sent half-parsed.
        let settings = dir.settings(Path::new("/bin/buzz-acp"), Some("[1]"));
        assert_eq!(settings.as_object().map(|o| o.len()), Some(1));
    }

    #[test]
    fn an_exited_agent_waits_on_nothing() {
        let mut work = BackgroundWork::default();
        work.observe_line(&sdk_message(
            "s1",
            serde_json::json!({ "type": "system", "subtype": "session_state_changed", "state": "running" }),
        ));
        work.mark_exited();
        assert!(work.exited());
        assert!(work.pending().is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn stop_hook_dir_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = StopHookDir::create().unwrap();
        let mode = std::fs::metadata(dir.path()).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o700);
    }

    #[test]
    fn stop_hook_dir_is_removed_on_drop() {
        let dir = StopHookDir::create().unwrap();
        let path = dir.path().to_owned();
        assert!(path.exists());
        drop(dir);
        assert!(!path.exists());
    }

    #[test]
    fn hook_command_quotes_paths() {
        let dir = StopHookDir::create().unwrap();
        let settings = dir.settings(Path::new("/Applications/Agent Factory.app/buzz-acp"), None);
        let command = settings["hooks"]["Stop"][0]["hooks"][0]["command"]
            .as_str()
            .unwrap();
        assert!(command.starts_with("'/Applications/Agent Factory.app/buzz-acp' stop-hook --dir '"));
    }
}
