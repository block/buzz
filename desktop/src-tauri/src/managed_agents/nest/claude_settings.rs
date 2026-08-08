//! Seed the nest's local Claude Code settings with an allowlist
//! for the bundled `buzz` CLI.
//!
//! Since #4609 the ACP harness answers every `session/request_permission`
//! with a rejection (fail closed), and desktop managed sessions run in
//! `dontAsk` mode with no in-app approval prompt. Claude Code evaluates
//! `permissions.allow` rules *before* the permission mode, so a local
//! allow rule for the bundled CLI keeps agents able to reach the relay
//! without reopening the blanket auto-approval that #4609 removed.
//!
//! The rules land in `<nest>/.claude/settings.local.json`. Unlike project-level
//! `settings.json`, Claude Code honors local allow rules before workspace trust
//! has been accepted — essential for fresh headless nests where no trust dialog
//! can be presented. The user's own `~/.claude` directory is never touched.
//!
//! This is temporary Buzz-owned compatibility behavior. Reconcile or remove
//! the workspace grant when Desktop permission approval UI ships in #5106.
//!
//! Known limitation: allow rules match single commands only. `buzz feed get`
//! is authorized; `buzz feed get | head` or `buzz ...; echo $?` still raises
//! a permission request and is rejected under `dontAsk`.

use std::fs;
use std::path::Path;

/// Permission rules granted to the bundled CLI. Exact-match rule covers a
/// bare `buzz` invocation; the `:*` prefix rule covers `buzz <args...>`.
const ALLOW_RULES: &[&str] = &["Bash(buzz)", "Bash(buzz:*)"];

/// Merge the `buzz` CLI allow rules into `<root>/.claude/settings.local.json`,
/// creating the file if absent.
///
/// Conservative by design:
/// - Existing settings are preserved; only missing rules are appended.
/// - A file that is not valid JSON, or whose `permissions` / `allow` nodes
///   have unexpected shapes, is left untouched (never clobber user edits).
/// - Idempotent: a second call with the rules present writes nothing.
pub(super) fn ensure_claude_buzz_allowlist(root: &Path) -> Result<(), String> {
    let claude_dir = root.join(".claude");
    refuse_symlink(&claude_dir)?;
    fs::create_dir_all(&claude_dir).map_err(|e| format!("create {}: {e}", claude_dir.display()))?;
    // Recheck after creation so a concurrent replacement cannot redirect later I/O.
    refuse_symlink(&claude_dir)?;
    let settings_path = claude_dir.join("settings.local.json");
    refuse_symlink(&settings_path)?;

    let mut settings: serde_json::Value = match fs::read_to_string(&settings_path) {
        Ok(text) => match serde_json::from_str(&text) {
            Ok(value) => value,
            Err(e) => {
                // Unparseable user file — leave it alone rather than clobber.
                eprintln!(
                    "buzz-desktop: {} is not valid JSON ({e}); skipping buzz CLI allowlist seed",
                    settings_path.display()
                );
                return Ok(());
            }
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => serde_json::json!({}),
        Err(e) => return Err(format!("read {}: {e}", settings_path.display())),
    };

    let Some(obj) = settings.as_object_mut() else {
        // Top level isn't an object — unexpected shape, don't touch it.
        return Ok(());
    };
    let permissions = obj
        .entry("permissions")
        .or_insert_with(|| serde_json::json!({}));
    let Some(permissions) = permissions.as_object_mut() else {
        return Ok(());
    };
    let allow = permissions
        .entry("allow")
        .or_insert_with(|| serde_json::json!([]));
    let Some(allow) = allow.as_array_mut() else {
        return Ok(());
    };

    let mut changed = false;
    for rule in ALLOW_RULES {
        if !allow.iter().any(|v| v.as_str() == Some(*rule)) {
            allow.push(serde_json::Value::String((*rule).to_string()));
            changed = true;
        }
    }
    if !changed {
        return Ok(());
    }

    let rendered = serde_json::to_string_pretty(&settings)
        .map_err(|e| format!("serialize {}: {e}", settings_path.display()))?;

    // Atomic write via temp file, matching refresh_skill_md_if_stale.
    let mut tmp = tempfile::NamedTempFile::new_in(&claude_dir)
        .map_err(|e| format!("tempfile in {}: {e}", claude_dir.display()))?;
    {
        use std::io::Write;
        tmp.write_all(rendered.as_bytes())
            .map_err(|e| format!("write tempfile: {e}"))?;
        tmp.write_all(b"\n")
            .map_err(|e| format!("write tempfile: {e}"))?;
    }
    // Recheck immediately before replacement in case the path changed while
    // the existing settings were being parsed.
    refuse_symlink(&claude_dir)?;
    refuse_symlink(&settings_path)?;
    tmp.persist(&settings_path)
        .map_err(|e| format!("persist {}: {e}", settings_path.display()))?;

    Ok(())
}

/// Reject an existing symlink without following it. Missing paths are safe: the
/// caller either just created the directory or will atomically create the file.
fn refuse_symlink(path: &Path) -> Result<(), String> {
    match path.symlink_metadata() {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(format!(
            "{} is a symlink; refusing to seed Claude settings",
            path.display()
        )),
        Ok(_) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("inspect {}: {e}", path.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read_settings(root: &Path) -> serde_json::Value {
        let text = fs::read_to_string(root.join(".claude/settings.local.json")).unwrap();
        serde_json::from_str(&text).unwrap()
    }

    fn allow_rules(value: &serde_json::Value) -> Vec<String> {
        value["permissions"]["allow"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect()
    }

    #[test]
    fn creates_settings_with_allow_rules_when_absent() {
        let tmp = tempfile::tempdir().unwrap();
        ensure_claude_buzz_allowlist(tmp.path()).unwrap();

        let settings = read_settings(tmp.path());
        assert_eq!(allow_rules(&settings), vec!["Bash(buzz)", "Bash(buzz:*)"]);
    }

    #[test]
    fn merges_into_existing_settings_preserving_other_keys() {
        let tmp = tempfile::tempdir().unwrap();
        let claude_dir = tmp.path().join(".claude");
        fs::create_dir_all(&claude_dir).unwrap();
        fs::write(
            claude_dir.join("settings.local.json"),
            r#"{"model": "opus", "permissions": {"allow": ["Bash(git status)"], "deny": ["WebFetch"]}}"#,
        )
        .unwrap();

        ensure_claude_buzz_allowlist(tmp.path()).unwrap();

        let settings = read_settings(tmp.path());
        assert_eq!(settings["model"], "opus");
        assert_eq!(settings["permissions"]["deny"][0], "WebFetch");
        assert_eq!(
            allow_rules(&settings),
            vec!["Bash(git status)", "Bash(buzz)", "Bash(buzz:*)"]
        );
    }

    #[test]
    fn is_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        ensure_claude_buzz_allowlist(tmp.path()).unwrap();
        let first = fs::read_to_string(tmp.path().join(".claude/settings.local.json")).unwrap();

        ensure_claude_buzz_allowlist(tmp.path()).unwrap();
        let second = fs::read_to_string(tmp.path().join(".claude/settings.local.json")).unwrap();

        assert_eq!(first, second);
        let settings = read_settings(tmp.path());
        assert_eq!(allow_rules(&settings).len(), 2, "rules must not duplicate");
    }

    #[test]
    fn leaves_invalid_json_untouched() {
        let tmp = tempfile::tempdir().unwrap();
        let claude_dir = tmp.path().join(".claude");
        fs::create_dir_all(&claude_dir).unwrap();
        fs::write(claude_dir.join("settings.local.json"), "{ not json").unwrap();

        ensure_claude_buzz_allowlist(tmp.path()).unwrap();

        let content = fs::read_to_string(claude_dir.join("settings.local.json")).unwrap();
        assert_eq!(content, "{ not json");
    }

    #[test]
    fn leaves_unexpected_shapes_untouched() {
        let tmp = tempfile::tempdir().unwrap();
        let claude_dir = tmp.path().join(".claude");
        fs::create_dir_all(&claude_dir).unwrap();
        // permissions.allow is an object, not an array.
        fs::write(
            claude_dir.join("settings.local.json"),
            r#"{"permissions": {"allow": {"weird": true}}}"#,
        )
        .unwrap();

        ensure_claude_buzz_allowlist(tmp.path()).unwrap();

        let settings = read_settings(tmp.path());
        assert_eq!(settings["permissions"]["allow"]["weird"], true);
    }

    #[test]
    fn leaves_project_settings_untouched() {
        let tmp = tempfile::tempdir().unwrap();
        let claude_dir = tmp.path().join(".claude");
        fs::create_dir_all(&claude_dir).unwrap();
        let project_settings = claude_dir.join("settings.json");
        fs::write(&project_settings, r#"{"model":"opus"}"#).unwrap();

        ensure_claude_buzz_allowlist(tmp.path()).unwrap();

        assert_eq!(
            fs::read_to_string(project_settings).unwrap(),
            r#"{"model":"opus"}"#
        );
        assert_eq!(allow_rules(&read_settings(tmp.path())).len(), 2);
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlinked_claude_directory_without_writing_target() {
        let tmp = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), tmp.path().join(".claude")).unwrap();

        let error = ensure_claude_buzz_allowlist(tmp.path()).unwrap_err();

        assert!(error.contains("is a symlink"));
        assert!(!outside.path().join("settings.local.json").exists());
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlinked_settings_file_without_modifying_target() {
        let tmp = tempfile::tempdir().unwrap();
        let claude_dir = tmp.path().join(".claude");
        fs::create_dir_all(&claude_dir).unwrap();
        let outside = tmp.path().join("outside.json");
        fs::write(&outside, r#"{"model":"opus"}"#).unwrap();
        std::os::unix::fs::symlink(&outside, claude_dir.join("settings.local.json")).unwrap();

        let error = ensure_claude_buzz_allowlist(tmp.path()).unwrap_err();

        assert!(error.contains("is a symlink"));
        assert_eq!(fs::read_to_string(outside).unwrap(), r#"{"model":"opus"}"#);
    }
}
