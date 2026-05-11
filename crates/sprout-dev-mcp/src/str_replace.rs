use crate::shell::SharedState;
use rmcp::ErrorData;
use schemars::JsonSchema;
use serde::Deserialize;
use similar::{DiffTag, TextDiff};
use std::io::Write;
use std::path::{Path, PathBuf};

const MAX_FILE_BYTES: u64 = 10 * 1024 * 1024;
const MAX_INPUT_BYTES: usize = 1024 * 1024;
const HINT_SCAN_LINE_LIMIT: usize = 200;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct StrReplaceParams {
    pub path: String,
    pub old_str: String,
    pub new_str: String,
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

    let workspace_root = match p.workdir.as_deref() {
        Some(w) => PathBuf::from(w),
        None => state.cwd.clone(),
    };
    let target = match resolve_within(&workspace_root, &p.path) {
        Ok(t) => t,
        Err(e) => return Err(ErrorData::invalid_params(e, None)),
    };

    let meta = match std::fs::metadata(&target) {
        Ok(m) => m,
        Err(e) => {
            return Err(ErrorData::internal_error(
                format!("cannot stat {}: {e}", target.display()),
                None,
            ));
        }
    };
    if !meta.is_file() {
        return Err(ErrorData::invalid_params(
            format!("not a regular file: {}", target.display()),
            None,
        ));
    }
    if meta.len() > MAX_FILE_BYTES {
        return Err(ErrorData::invalid_params(
            format!(
                "file too large: {} is {} bytes (limit {} bytes)",
                target.display(),
                meta.len(),
                MAX_FILE_BYTES
            ),
            None,
        ));
    }

    let file = match std::fs::File::open(&target) {
        Ok(f) => f,
        Err(e) => {
            return Err(ErrorData::internal_error(
                format!("cannot open {}: {e}", target.display()),
                None,
            ));
        }
    };
    let mut buf = Vec::with_capacity(meta.len() as usize);
    use std::io::Read;
    match file.take(MAX_FILE_BYTES + 1).read_to_end(&mut buf) {
        Ok(n) if n as u64 > MAX_FILE_BYTES => {
            return Err(ErrorData::invalid_params(
                format!("file grew past {} bytes during read", MAX_FILE_BYTES),
                None,
            ));
        }
        Ok(_) => {}
        Err(e) => {
            return Err(ErrorData::internal_error(
                format!("cannot read {}: {e}", target.display()),
                None,
            ));
        }
    }
    let content = match String::from_utf8(buf) {
        Ok(s) => s,
        Err(e) => {
            return Err(ErrorData::internal_error(
                format!("not valid UTF-8: {}: {e}", target.display()),
                None,
            ));
        }
    };

    let count = count_occurrences_capped(&content, &p.old_str);
    match count {
        0 => {
            let hint = nearest_line_hint(&content, &p.old_str)
                .map(|h| format!("\n{h}"))
                .unwrap_or_default();
            Err(ErrorData::invalid_params(
                format!(
                    "old_str not found in {}.\nold_str (truncated): {:?}{hint}",
                    target.display(),
                    truncate(&p.old_str, 80)
                ),
                None,
            ))
        }
        1 => {
            let new_content = content.replacen(&p.old_str, &p.new_str, 1);
            if new_content.len() as u64 > MAX_FILE_BYTES {
                return Err(ErrorData::invalid_params(
                    format!(
                        "result would exceed {} byte limit ({} bytes)",
                        MAX_FILE_BYTES,
                        new_content.len()
                    ),
                    None,
                ));
            }
            if let Err(e) = atomic_write(&target, &new_content) {
                return Err(ErrorData::internal_error(
                    format!("failed to write {}: {e}", target.display()),
                    None,
                ));
            }
            let diff = unified_diff(&content, &new_content, &target);
            Ok(format!(
                "Replaced 1 occurrence in {}.\n\n{diff}",
                target.display()
            ))
        }
        _ => Err(ErrorData::invalid_params(
            format!(
                "old_str matched multiple locations in {}; provide more surrounding context to make the match unique.",
                target.display()
            ),
            None,
        )),
    }
}

pub(crate) fn resolve_within(root: &Path, path: &str) -> Result<PathBuf, String> {
    let raw = Path::new(path);
    let candidate: PathBuf = if raw.is_absolute() {
        raw.to_path_buf()
    } else {
        root.join(raw)
    };

    let root_canon = std::fs::canonicalize(root)
        .map_err(|e| format!("workdir not accessible: {} ({e})", root.display()))?;

    let resolved = std::fs::canonicalize(&candidate)
        .map_err(|e| format!("path not accessible: {} ({e})", candidate.display()))?;

    if !resolved.starts_with(&root_canon) {
        return Err(format!(
            "path escapes workspace: {} not within {}",
            resolved.display(),
            root_canon.display()
        ));
    }
    Ok(resolved)
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

    #[test]
    fn resolve_within_rejects_escape() {
        let dir = tempdir().expect("tempdir");
        let inside = dir.path().join("file.txt");
        fs::write(&inside, b"x").expect("write");
        // Symlink targeting outside the dir should be rejected.
        #[cfg(unix)]
        {
            let outside = std::env::temp_dir().join("sprout-mcp-escape-target");
            let _ = fs::remove_file(&outside);
            fs::write(&outside, b"y").expect("write outside");
            let link = dir.path().join("link.txt");
            std::os::unix::fs::symlink(&outside, &link).expect("symlink");
            let err = resolve_within(dir.path(), "link.txt").unwrap_err();
            assert!(err.contains("escapes workspace"), "got: {err}");
            let _ = fs::remove_file(&outside);
        }
        // Resolves a normal path inside.
        let p = resolve_within(dir.path(), "file.txt").expect("resolve");
        assert!(p.ends_with("file.txt"));
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
    fn run_rejects_path_outside_workspace() {
        let dir = tempdir().expect("tempdir");
        let state = make_state(dir.path());
        let p = StrReplaceParams {
            path: "/etc/hosts".into(),
            old_str: "x".into(),
            new_str: "y".into(),
            workdir: Some(dir.path().display().to_string()),
        };
        let err = run(&state, p).unwrap_err();
        let msg = format!("{err:?}");
        assert!(
            msg.contains("escapes workspace") || msg.contains("not accessible"),
            "msg: {msg}"
        );
    }

    #[test]
    fn run_rejects_file_too_large() {
        let dir = tempdir().expect("tempdir");
        let f = dir.path().join("big.bin");
        let big = vec![b'a'; (MAX_FILE_BYTES as usize) + 1024];
        fs::write(&f, &big).expect("write");
        let state = make_state(dir.path());
        let p = StrReplaceParams {
            path: "big.bin".into(),
            old_str: "a".into(),
            new_str: "b".into(),
            workdir: Some(dir.path().display().to_string()),
        };
        let err = run(&state, p).unwrap_err();
        let msg = format!("{err:?}");
        assert!(msg.contains("too large"), "msg: {msg}");
    }
}
