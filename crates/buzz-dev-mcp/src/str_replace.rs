use crate::shell::SharedState;
use rmcp::ErrorData;
use schemars::JsonSchema;
use serde::Deserialize;
use similar::{DiffTag, TextDiff};
use std::io::Write;
use std::path::Path;

const MAX_INPUT_BYTES: usize = 1024 * 1024;
const HINT_SCAN_LINE_LIMIT: usize = 200;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct StrReplaceParams {
    pub path: String,
    pub old_str: String,
    pub new_str: String,
    /// When true, replace ALL occurrences of old_str instead of requiring
    /// exactly one match.
    #[serde(default)]
    pub replace_all: bool,
    #[serde(default)]
    pub workdir: Option<String>,
}

pub fn run(state: &SharedState, p: StrReplaceParams) -> Result<String, ErrorData> {
    if p.old_str.is_empty() {
        return Err(ErrorData::invalid_params(
            "old_str must not be empty".to_string(),
            None,
        ));
    }
    if p.old_str.len() > MAX_INPUT_BYTES || p.new_str.len() > MAX_INPUT_BYTES {
        return Err(ErrorData::invalid_params(
            format!("old_str/new_str exceeds {} byte limit", MAX_INPUT_BYTES),
            None,
        ));
    }

    let (target, content) = crate::paths::read_text_file(state, &p.path, p.workdir.as_deref())?;

    let count = if p.replace_all {
        content.matches(&p.old_str).count()
    } else {
        count_occurrences_capped(&content, &p.old_str)
    };

    if count == 0 {
        let hint = nearest_line_hint(&content, &p.old_str)
            .map(|h| format!("\n{h}"))
            .unwrap_or_default();
        return Err(ErrorData::invalid_params(
            format!(
                "old_str not found in {}.\nold_str (truncated): {:?}{hint}",
                target.display(),
                truncate(&p.old_str, 80)
            ),
            None,
        ));
    }
    if !p.replace_all && count > 1 {
        return Err(ErrorData::invalid_params(
            format!(
                "old_str matched multiple locations in {}; provide more surrounding context to make the match unique.",
                target.display()
            ),
            None,
        ));
    }

    // Preflight: reject before allocating if the result would exceed the limit.
    let size_delta = (p.new_str.len() as i64) - (p.old_str.len() as i64);
    let projected = (content.len() as i64).saturating_add(size_delta.saturating_mul(count as i64));
    if projected < 0 || projected as u64 > crate::paths::MAX_FILE_BYTES {
        return Err(ErrorData::invalid_params(
            format!(
                "result would exceed {} byte limit ({} bytes projected)",
                crate::paths::MAX_FILE_BYTES,
                projected
            ),
            None,
        ));
    }

    let new_content = if p.replace_all {
        content.replace(&p.old_str, &p.new_str)
    } else {
        content.replacen(&p.old_str, &p.new_str, 1)
    };

    if let Err(e) = atomic_write(&target, &new_content) {
        return Err(ErrorData::internal_error(
            format!("failed to write {}: {e}", target.display()),
            None,
        ));
    }
    let diff = unified_diff(&content, &new_content, &target);
    let label = if count == 1 {
        "1 occurrence".to_string()
    } else {
        format!("{count} occurrence(s)")
    };
    Ok(format!(
        "Replaced {label} in {}.\n\n{diff}",
        target.display()
    ))
}

pub(crate) fn count_occurrences_capped(text: &str, pattern: &str) -> usize {
    if pattern.is_empty() {
        return 0;
    }
    let mut count = 0;
    let mut start = 0;
    while let Some(pos) = text[start..].find(pattern) {
        count += 1;
        if count >= 2 {
            return count;
        }
        start += pos + pattern.len();
    }
    count
}

fn atomic_write(target: &Path, content: &str) -> std::io::Result<()> {
    let parent = target.parent().unwrap_or_else(|| Path::new("."));
    // Preserve original permissions so the atomic rename doesn't drop the file's mode.
    let original_perms = std::fs::metadata(target).ok().map(|m| m.permissions());

    let mut tmp = tempfile::NamedTempFile::new_in(parent)?;
    tmp.write_all(content.as_bytes())?;
    tmp.flush()?;
    tmp.persist(target).map_err(|e| e.error)?;

    if let Some(perms) = original_perms {
        let _ = std::fs::set_permissions(target, perms);
    }
    Ok(())
}

const MAX_DIFF_BYTES: usize = 64 * 1024;

fn unified_diff(old: &str, new: &str, path: &Path) -> String {
    let diff = TextDiff::from_lines(old, new);
    let display = path.display();
    let mut out = format!("--- a/{display}\n+++ b/{display}\n");
    for hunk in diff.unified_diff().context_radius(3).iter_hunks() {
        let h = hunk.to_string();
        if out.len() + h.len() > MAX_DIFF_BYTES {
            out.push_str("\n[diff truncated]\n");
            break;
        }
        out.push_str(&h);
    }
    out
}

fn truncate(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        s.to_string()
    } else {
        let head: String = s.chars().take(max_chars).collect();
        format!("{head}…")
    }
}

fn truncate_str(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut cut = max;
    while cut > 0 && !s.is_char_boundary(cut) {
        cut -= 1;
    }
    &s[..cut]
}

fn similarity(a: &str, b: &str) -> f64 {
    if a == b {
        return 1.0;
    }
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    const MAX: usize = 512;
    let a = truncate_str(a, MAX);
    let b = truncate_str(b, MAX);
    let matched: usize = TextDiff::from_chars(a, b)
        .ops()
        .iter()
        .filter(|op| matches!(op.tag(), DiffTag::Equal))
        .map(|op| op.new_range().len())
        .sum();
    matched as f64 / a.len().max(b.len()) as f64
}

fn nearest_line_hint(content: &str, pattern: &str) -> Option<String> {
    let first = pattern.lines().next()?.trim();
    if first.is_empty() {
        return None;
    }
    let best = content
        .lines()
        .take(HINT_SCAN_LINE_LIMIT)
        .enumerate()
        .map(|(i, line)| (i, similarity(line.trim(), first), line))
        .filter(|(_, s, _)| *s > 0.6)
        .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))?;
    Some(format!(
        "Hint: nearest match around line {} (similarity {:.2}):\n  found:    {:?}\n  expected: {:?}",
        best.0 + 1,
        best.1,
        best.2.trim(),
        first
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn count_occurrences_capped_returns_0_1_2() {
        assert_eq!(count_occurrences_capped("hello world", "x"), 0);
        assert_eq!(count_occurrences_capped("hello world", "hello"), 1);
        assert_eq!(count_occurrences_capped("a a a a a", "a"), 2); // capped at 2
        assert_eq!(count_occurrences_capped("abc", ""), 0);
    }

    fn make_state(cwd: &std::path::Path) -> SharedState {
        let shim = crate::shim::Shim::install().expect("shim install");
        SharedState::new(cwd.to_path_buf(), shim).expect("state new")
    }

    #[test]
    fn run_basic_replace_emits_diff() {
        let dir = tempdir().expect("tempdir");
        let f = dir.path().join("a.txt");
        fs::write(&f, "alpha\nbeta\ngamma\n").expect("write");
        let state = make_state(dir.path());
        let p = StrReplaceParams {
            path: "a.txt".into(),
            old_str: "beta".into(),
            new_str: "BETA".into(),
            replace_all: false,
            workdir: Some(dir.path().display().to_string()),
        };
        let out = run(&state, p).expect("ok");
        assert!(out.contains("Replaced 1 occurrence"), "out: {out}");
        assert!(out.contains("-beta"), "out: {out}");
        assert!(out.contains("+BETA"), "out: {out}");
        let contents = fs::read_to_string(&f).expect("read");
        assert_eq!(contents, "alpha\nBETA\ngamma\n");
    }

    #[test]
    fn run_allows_path_outside_workspace() {
        let dir = tempdir().expect("tempdir");
        let state = make_state(dir.path());
        // /etc/hosts is readable but won't contain our old_str — we expect
        // a "not found" error, not a path-escape error.
        let p = StrReplaceParams {
            path: "/etc/hosts".into(),
            old_str: "UNIQUE_STRING_NOT_IN_HOSTS_FILE_abc123".into(),
            new_str: "y".into(),
            replace_all: false,
            workdir: Some(dir.path().display().to_string()),
        };
        let err = run(&state, p).unwrap_err();
        let msg = format!("{err:?}");
        assert!(
            msg.contains("not found"),
            "expected 'not found' error (proving path resolved), got: {msg}"
        );
    }

    #[test]
    fn run_rejects_file_too_large() {
        let dir = tempdir().expect("tempdir");
        let f = dir.path().join("big.bin");
        let big = vec![b'a'; (crate::paths::MAX_FILE_BYTES as usize) + 1024];
        fs::write(&f, &big).expect("write");
        let state = make_state(dir.path());
        let p = StrReplaceParams {
            path: "big.bin".into(),
            old_str: "a".into(),
            new_str: "b".into(),
            replace_all: false,
            workdir: Some(dir.path().display().to_string()),
        };
        let err = run(&state, p).unwrap_err();
        let msg = format!("{err:?}");
        assert!(msg.contains("too large"), "msg: {msg}");
    }

    #[test]
    fn run_replace_all_replaces_all_occurrences() {
        let dir = tempdir().expect("tempdir");
        let f = dir.path().join("multi.txt");
        fs::write(&f, "foo bar foo baz foo\n").expect("write");
        let state = make_state(dir.path());
        let p = StrReplaceParams {
            path: "multi.txt".into(),
            old_str: "foo".into(),
            new_str: "qux".into(),
            replace_all: true,
            workdir: Some(dir.path().display().to_string()),
        };
        let out = run(&state, p).expect("ok");
        assert!(out.contains("Replaced 3 occurrence(s)"), "out: {out}");
        let contents = fs::read_to_string(&f).expect("read");
        assert_eq!(contents, "qux bar qux baz qux\n");
    }

    #[test]
    fn run_replace_all_errors_on_zero_matches() {
        let dir = tempdir().expect("tempdir");
        let f = dir.path().join("nomatch.txt");
        fs::write(&f, "hello world\n").expect("write");
        let state = make_state(dir.path());
        let p = StrReplaceParams {
            path: "nomatch.txt".into(),
            old_str: "xyz".into(),
            new_str: "abc".into(),
            replace_all: true,
            workdir: Some(dir.path().display().to_string()),
        };
        let err = run(&state, p).unwrap_err();
        let msg = format!("{err:?}");
        assert!(msg.contains("not found"), "msg: {msg}");
    }

    #[test]
    fn run_without_replace_all_preserves_single_match_behavior() {
        let dir = tempdir().expect("tempdir");
        let f = dir.path().join("multi2.txt");
        fs::write(&f, "foo bar foo\n").expect("write");
        let state = make_state(dir.path());
        let p = StrReplaceParams {
            path: "multi2.txt".into(),
            old_str: "foo".into(),
            new_str: "qux".into(),
            replace_all: false,
            workdir: Some(dir.path().display().to_string()),
        };
        let err = run(&state, p).unwrap_err();
        let msg = format!("{err:?}");
        assert!(msg.contains("matched multiple locations"), "msg: {msg}");
    }
}
