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

/// One change a `_claude/sdkMessage` notification reports.
#[derive(Debug)]
pub(crate) struct SdkUpdate {
    session_id: String,
    change: Change,
}

#[derive(Debug)]
enum Change {
    /// The full set of live background tasks, by kind.
    Tasks(Vec<String>),
    /// Whether the session is in a model cycle.
    Running(bool),
    /// An SDK message buzz-acp did not ask for.
    None,
}

/// Parse `line` if it is a `_claude/sdkMessage` notification.
///
/// Returns `None` for every other line. Those notifications exist only for
/// [`BackgroundWork`], so the caller drops them instead of passing them on.
/// Parsing happens here, outside any lock.
pub(crate) fn parse_sdk_message(line: &str) -> Option<SdkUpdate> {
    if !line.contains(SDK_MESSAGE_METHOD) {
        return None;
    }
    let msg = serde_json::from_str::<Value>(line).ok()?;
    if msg.get("method").and_then(Value::as_str) != Some(SDK_MESSAGE_METHOD) {
        return None;
    }
    let params = &msg["params"];
    let message = &params["message"];
    let session_id = params["sessionId"]
        .as_str()
        .or_else(|| message["session_id"].as_str())
        .unwrap_or_default()
        .to_owned();
    let change = match message["subtype"].as_str() {
        // REPLACE semantics: the payload is the whole live set.
        Some("background_tasks_changed") => Change::Tasks(
            message["tasks"]
                .as_array()
                .map(|tasks| tasks.iter().map(describe_task).collect())
                .unwrap_or_default(),
        ),
        Some("session_state_changed") => {
            Change::Running(message["state"].as_str() == Some("running"))
        }
        _ => Change::None,
    };
    Some(SdkUpdate { session_id, change })
}

/// Most sessions [`BackgroundWork`] tracks per agent process. One process
/// serves a handful of sessions; ids beyond this are ignored.
const MAX_TRACKED_SESSIONS: usize = 64;
/// Most background tasks kept per session.
const MAX_TASKS_PER_SESSION: usize = 32;

impl BackgroundWork {
    /// Apply one parsed `_claude/sdkMessage` notification.
    pub(crate) fn apply(&mut self, update: SdkUpdate) {
        if update.session_id.is_empty() {
            return;
        }
        if !self.sessions.contains_key(&update.session_id)
            && self.sessions.len() >= MAX_TRACKED_SESSIONS
        {
            return;
        }
        match update.change {
            Change::Tasks(mut tasks) => {
                tasks.truncate(MAX_TASKS_PER_SESSION);
                self.sessions.entry(update.session_id).or_default().tasks = tasks;
            }
            Change::Running(running) => {
                self.sessions.entry(update.session_id).or_default().running = running;
            }
            Change::None => {}
        }
        self.sessions
            .retain(|_, activity| activity.running || !activity.tasks.is_empty());
    }

    /// Parse and apply `line`; `true` when it was a `_claude/sdkMessage`.
    #[cfg(test)]
    fn observe_line(&mut self, line: &str) -> bool {
        match parse_sdk_message(line) {
            Some(update) => {
                self.apply(update);
                true
            }
            None => false,
        }
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
    /// nothing is pending. Items name kinds only, never task descriptions,
    /// so they are safe to log.
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

/// A task's kind, such as `local_bash`. The description is model-written
/// text that can quote anything, so it is not kept.
fn describe_task(task: &Value) -> String {
    task["task_type"]
        .as_str()
        .filter(|kind| {
            !kind.is_empty()
                && kind.len() <= 32
                && kind
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        })
        .unwrap_or("task")
        .to_owned()
}

/// Per-process directory the Stop hook writes session crons into. Removed on
/// drop.
#[derive(Debug)]
pub(crate) struct StopHookDir {
    path: PathBuf,
}

impl StopHookDir {
    /// Create a fresh directory under the system temp dir, readable only by
    /// this user: the cron prompts the hook writes are the agent's own words.
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
    pub(crate) fn path(&self) -> &Path {
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
        let hook = serde_json::json!({ "hooks": [{ "type": "command", "command": command }] });
        let mut settings = match model_config.map(serde_json::from_str::<Value>) {
            Some(Ok(Value::Object(config))) => config,
            Some(_) => {
                tracing::warn!("CLAUDE_MODEL_CONFIG is not a JSON object — not merged");
                serde_json::Map::new()
            }
            None => serde_json::Map::new(),
        };
        let hooks = settings
            .entry("hooks")
            .or_insert_with(|| Value::Object(serde_json::Map::new()));
        if !hooks.is_object() {
            tracing::warn!("CLAUDE_MODEL_CONFIG has a non-object `hooks` — replaced");
            *hooks = Value::Object(serde_json::Map::new());
        }
        if let Some(hooks) = hooks.as_object_mut() {
            match hooks.get_mut("Stop").and_then(Value::as_array_mut) {
                Some(stop) => stop.push(hook),
                None => {
                    hooks.insert("Stop".to_owned(), Value::Array(vec![hook]));
                }
            }
        }
        Value::Object(settings)
    }

    /// The scheduled wakeups of every session, one line per cron. A session
    /// whose last Stop hook listed no crons contributes nothing.
    ///
    /// The agent can write to this directory, so only regular files up to
    /// [`MAX_RECORD_BYTES`] count, and at most [`MAX_RECORDS`] of them.
    pub(crate) fn crons(&self) -> Vec<String> {
        let Ok(entries) = std::fs::read_dir(&self.path) else {
            return Vec::new();
        };
        let mut crons = Vec::new();
        for entry in entries.flatten().take(MAX_RECORDS) {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                remove_if_abandoned(&path);
                continue;
            }
            let Some(text) = read_record(&path) else {
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

/// Most entries of the Stop hook directory [`StopHookDir::crons`] looks at.
const MAX_RECORDS: usize = 256;
/// Largest record [`StopHookDir::crons`] reads.
const MAX_RECORD_BYTES: u64 = 64 * 1024;

/// The text of `path` if it is a regular file (not a link, FIFO or device) of
/// at most [`MAX_RECORD_BYTES`].
fn read_record(path: &Path) -> Option<String> {
    let meta = std::fs::symlink_metadata(path).ok()?;
    if !meta.file_type().is_file() || meta.len() > MAX_RECORD_BYTES {
        return None;
    }
    let mut text = String::new();
    std::fs::File::open(path)
        .ok()?
        .take(MAX_RECORD_BYTES)
        .read_to_string(&mut text)
        .ok()?;
    Some(text)
}

/// Remove `path` if it is a `.json.tmp` the hook stopped writing a minute
/// ago or more: a hook that died between write and rename.
fn remove_if_abandoned(path: &Path) {
    let is_tmp = path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.ends_with(".json.tmp"));
    let abandoned = std::fs::symlink_metadata(path)
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(|modified| modified.elapsed().ok())
        .is_some_and(|age| age >= std::time::Duration::from_secs(60));
    if is_tmp && abandoned {
        let _ = std::fs::remove_file(path);
    }
}

impl Drop for StopHookDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// A cron's kind and schedule. The prompt is the agent's own words, so it is
/// not kept.
fn describe_cron(cron: &Value) -> String {
    let schedule = cron["schedule"]
        .as_str()
        .filter(|schedule| {
            !schedule.is_empty()
                && schedule.len() <= 64
                && schedule
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || " */,-?".contains(c))
        })
        .unwrap_or("?");
    if cron["recurring"].as_bool() == Some(true) {
        format!("recurring wakeup `{schedule}`")
    } else {
        format!("wakeup `{schedule}`")
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
    let Some(input) = read_stdin_bounded() else {
        return 0;
    };
    if let Err(error) = record_stop_hook_input(Path::new(dir), &input) {
        // stdout and stderr must stay silent; leave a trace next to the
        // records, up to a size.
        use std::io::Write;
        let log = Path::new(dir).join("stop-hook-errors.log");
        let small = std::fs::metadata(&log).map_or(true, |meta| meta.len() < MAX_RECORD_BYTES);
        if small {
            if let Ok(mut log) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&log)
            {
                let _ = writeln!(log, "{error}");
            }
        }
    }
    0
}

/// Largest Stop hook input read. A Stop input carries the last assistant
/// message, so it can be large; anything past this is not a usable record.
const MAX_STOP_HOOK_INPUT_BYTES: u64 = 1024 * 1024;
/// How long the hook waits for its input before it gives up.
const STOP_HOOK_INPUT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// Stdin up to [`MAX_STOP_HOOK_INPUT_BYTES`], or `None` when it is larger,
/// unreadable, or does not end within [`STOP_HOOK_INPUT_TIMEOUT`].
fn read_stdin_bounded() -> Option<String> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut input = String::new();
        let read = std::io::stdin()
            .take(MAX_STOP_HOOK_INPUT_BYTES + 1)
            .read_to_string(&mut input);
        let _ = tx.send(read.ok().map(|_| input));
    });
    let input = rx.recv_timeout(STOP_HOOK_INPUT_TIMEOUT).ok()??;
    (input.len() as u64 <= MAX_STOP_HOOK_INPUT_BYTES).then_some(input)
}

fn record_stop_hook_input(dir: &Path, input: &str) -> std::io::Result<()> {
    let hook: Value = serde_json::from_str(input).map_err(std::io::Error::other)?;
    let session_id = hook["session_id"]
        .as_str()
        .filter(|id| !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'))
        .ok_or_else(|| std::io::Error::other("stop hook input has no usable session_id"))?;
    // No `session_crons` means the hook could not tell, not that there are
    // none: keep the last record.
    let Some(crons) = hook.get("session_crons").filter(|crons| crons.is_array()) else {
        return Ok(());
    };
    // Keep only what `crons()` reads. The prompt is the agent's own words.
    let crons: Vec<Value> = crons
        .as_array()
        .into_iter()
        .flatten()
        .map(|cron| {
            serde_json::json!({
                "schedule": cron.get("schedule").cloned().unwrap_or(Value::Null),
                "recurring": cron.get("recurring").cloned().unwrap_or(Value::Null),
            })
        })
        .collect();
    let record = serde_json::json!({ "session_crons": crons });
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
        assert_eq!(work.pending(), vec!["local_bash".to_owned()]);

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
        assert_eq!(dir.crons(), vec!["wakeup `43 10 1 11 *`".to_owned()]);

        // The next Stop hook of the same session replaces the record.
        let input = serde_json::json!({ "session_id": "d9367-4cd1", "session_crons": [] });
        record_stop_hook_input(dir.path(), &input.to_string()).unwrap();
        assert!(dir.crons().is_empty());
    }

    #[test]
    fn a_stop_hook_without_crons_keeps_the_last_record() {
        let dir = StopHookDir::create().unwrap();
        let with_cron = serde_json::json!({
            "session_id": "s-1",
            "session_crons": [{ "schedule": "0 9 * * *", "recurring": true, "prompt": "p" }],
        });
        record_stop_hook_input(dir.path(), &with_cron.to_string()).unwrap();
        let unknown = serde_json::json!({ "session_id": "s-1" });
        record_stop_hook_input(dir.path(), &unknown.to_string()).unwrap();
        assert_eq!(dir.crons(), vec!["recurring wakeup `0 9 * * *`".to_owned()]);
    }

    #[test]
    fn cron_and_task_lines_carry_no_model_text() {
        let dir = StopHookDir::create().unwrap();
        let input = serde_json::json!({
            "session_id": "s-1",
            "session_crons": [{ "schedule": "0 9 * * *", "prompt": "token=SECRET" }],
        });
        record_stop_hook_input(dir.path(), &input.to_string()).unwrap();
        assert!(dir.crons().iter().all(|line| !line.contains("SECRET")));

        let mut work = BackgroundWork::default();
        work.observe_line(&sdk_message(
            "s1",
            serde_json::json!({
                "type": "system",
                "subtype": "background_tasks_changed",
                "tasks": [{ "task_type": "local_bash", "description": "curl -H 'Authorization: SECRET'" }],
            }),
        ));
        assert!(work.pending().iter().all(|line| !line.contains("SECRET")));
    }

    #[test]
    fn a_non_object_hooks_in_the_model_config_is_replaced_not_a_panic() {
        let dir = StopHookDir::create().unwrap();
        for config in [
            r#"{"hooks":[]}"#,
            r#"{"hooks":"none"}"#,
            r#"{"hooks":{"Stop":7}}"#,
        ] {
            let settings = dir.settings(Path::new("/bin/buzz-acp"), Some(config));
            assert!(
                settings["hooks"]["Stop"][0]["hooks"][0]["command"].is_string(),
                "{config}: {settings}"
            );
        }
        let config = r#"{"hooks":{"Stop":[{"hooks":[]}]}}"#;
        let settings = dir.settings(Path::new("/bin/buzz-acp"), Some(config));
        assert_eq!(settings["hooks"]["Stop"].as_array().map(Vec::len), Some(2));
    }

    #[test]
    fn the_record_keeps_schedule_and_recurrence_only() {
        let dir = StopHookDir::create().unwrap();
        let input = serde_json::json!({
            "session_id": "s-1",
            "session_crons": [{ "id": "c1", "schedule": "0 9 * * *", "recurring": false, "prompt": "token=SECRET" }],
        });
        record_stop_hook_input(dir.path(), &input.to_string()).unwrap();
        let text = std::fs::read_to_string(dir.path().join("s-1.json")).unwrap();
        assert!(!text.contains("SECRET") && !text.contains("c1"), "{text}");
        assert_eq!(dir.crons(), vec!["wakeup `0 9 * * *`".to_owned()]);
    }

    #[cfg(unix)]
    #[test]
    fn crons_skip_links_and_oversized_records() {
        let dir = StopHookDir::create().unwrap();
        std::os::unix::fs::symlink("/dev/zero", dir.path().join("zero.json")).unwrap();
        let big = format!(
            r#"{{"session_crons":[{{"schedule":"0 9 * * *"}}],"pad":"{}"}}"#,
            "x".repeat(MAX_RECORD_BYTES as usize)
        );
        std::fs::write(dir.path().join("big.json"), big).unwrap();
        assert!(dir.crons().is_empty());
    }

    #[test]
    fn agent_written_kinds_and_schedules_are_validated() {
        let task = serde_json::json!({ "task_type": "local_bash\nforged log line" });
        assert_eq!(describe_task(&task), "task");
        let task = serde_json::json!({ "task_type": "x".repeat(33) });
        assert_eq!(describe_task(&task), "task");
        let cron = serde_json::json!({ "schedule": "0 9 * * *\nforged" });
        assert_eq!(describe_cron(&cron), "wakeup `?`");
    }

    #[test]
    fn tracked_sessions_are_capped() {
        let mut work = BackgroundWork::default();
        for i in 0..MAX_TRACKED_SESSIONS + 10 {
            work.observe_line(&sdk_message(
                &format!("s{i}"),
                serde_json::json!({ "type": "system", "subtype": "session_state_changed", "state": "running" }),
            ));
        }
        assert_eq!(work.pending().len(), MAX_TRACKED_SESSIONS);
    }

    #[test]
    fn an_abandoned_tmp_file_is_removed_and_a_fresh_one_kept() {
        let dir = StopHookDir::create().unwrap();
        let fresh = dir.path().join("a.json.tmp");
        let old = dir.path().join("b.json.tmp");
        std::fs::write(&fresh, "{}").unwrap();
        std::fs::write(&old, "{}").unwrap();
        let hour_ago = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
        std::fs::File::options()
            .write(true)
            .open(&old)
            .unwrap()
            .set_modified(hour_ago)
            .unwrap();
        dir.crons();
        assert!(fresh.exists());
        assert!(!old.exists());
    }

    #[test]
    fn the_raw_message_filter_names_the_two_subtypes() {
        assert_eq!(
            emit_raw_filter(),
            serde_json::json!([
                { "type": "system", "subtype": "background_tasks_changed" },
                { "type": "system", "subtype": "session_state_changed" },
            ])
        );
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
