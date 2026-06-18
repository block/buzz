use crate::shell::SharedState;
use rmcp::ErrorData;
use schemars::JsonSchema;
use serde::Deserialize;

const DEFAULT_LIMIT: usize = 2000;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ReadFileParams {
    /// File path (absolute or relative to workdir).
    pub path: String,
    /// 0-based line offset to start reading from. Defaults to 0.
    #[serde(default)]
    pub offset: Option<usize>,
    /// Maximum number of lines to return. Defaults to 2000.
    #[serde(default)]
    pub limit: Option<usize>,
    /// Workspace root for relative path resolution. Defaults to server cwd.
    #[serde(default)]
    pub workdir: Option<String>,
}

pub fn run(state: &SharedState, p: ReadFileParams) -> Result<String, ErrorData> {
    let (_target, content) = crate::paths::read_text_file(state, &p.path, p.workdir.as_deref())?;

    let all_lines: Vec<&str> = content.lines().collect();
    let total = all_lines.len();

    if total == 0 {
        return Ok(format!("{} is empty (0 lines)", p.path));
    }

    let offset = p.offset.unwrap_or(0);
    let limit = p.limit.unwrap_or(DEFAULT_LIMIT);

    let slice = &all_lines[offset.min(total)..];
    let slice = &slice[..slice.len().min(limit)];

    if slice.is_empty() {
        return Ok(format!(
            "{} (no lines in range, file has {} lines)",
            p.path, total
        ));
    }

    // 1-based line numbers in the output.
    let start_line = offset + 1;
    let end_line = offset + slice.len();

    let mut out = format!(
        "{} (lines {}-{} of {})\n",
        p.path, start_line, end_line, total
    );
    for (i, line) in slice.iter().enumerate() {
        let line_number = offset + i + 1;
        out.push_str(&format!("{line_number}:{line}\n"));
    }

    if end_line < total {
        out.push_str(&format!(
            "[showing lines {start_line}-{end_line} of {total}; use offset={end_line} to continue]\n"
        ));
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn make_state(cwd: &std::path::Path) -> SharedState {
        let shim = crate::shim::Shim::install().expect("shim install");
        SharedState::new(cwd.to_path_buf(), shim).expect("state new")
    }

    #[test]
    fn read_basic() {
        let dir = tempdir().expect("tempdir");
        let f = dir.path().join("basic.txt");
        fs::write(&f, "line1\nline2\nline3\nline4\nline5\n").expect("write");
        let state = make_state(dir.path());
        let p = ReadFileParams {
            path: "basic.txt".into(),
            offset: None,
            limit: None,
            workdir: Some(dir.path().display().to_string()),
        };
        let out = run(&state, p).expect("ok");
        assert!(out.contains("lines 1-5 of 5"), "out: {out}");
        assert!(out.contains("1:line1"), "out: {out}");
        assert!(out.contains("2:line2"), "out: {out}");
        assert!(out.contains("3:line3"), "out: {out}");
        assert!(out.contains("4:line4"), "out: {out}");
        assert!(out.contains("5:line5"), "out: {out}");
        assert!(
            !out.contains("[showing lines"),
            "full file should have no truncation footer: {out}"
        );
    }

    #[test]
    fn read_offset_limit() {
        let dir = tempdir().expect("tempdir");
        let f = dir.path().join("ten.txt");
        let contents: String = (1..=10).map(|i| format!("line{i}\n")).collect();
        fs::write(&f, &contents).expect("write");
        let state = make_state(dir.path());
        let p = ReadFileParams {
            path: "ten.txt".into(),
            offset: Some(3),
            limit: Some(2),
            workdir: Some(dir.path().display().to_string()),
        };
        let out = run(&state, p).expect("ok");
        assert!(out.contains("lines 4-5 of 10"), "out: {out}");
        assert!(out.contains("4:line4"), "out: {out}");
        assert!(out.contains("5:line5"), "out: {out}");
        assert!(
            out.contains("[showing lines 4-5 of 10; use offset=5 to continue]"),
            "out: {out}"
        );
    }

    #[test]
    fn read_empty_file() {
        let dir = tempdir().expect("tempdir");
        let f = dir.path().join("empty.txt");
        fs::write(&f, b"").expect("write");
        let state = make_state(dir.path());
        let p = ReadFileParams {
            path: "empty.txt".into(),
            offset: None,
            limit: None,
            workdir: Some(dir.path().display().to_string()),
        };
        let out = run(&state, p).expect("ok");
        assert!(out.contains("is empty (0 lines)"), "out: {out}");
    }

    #[test]
    fn read_allows_absolute_path() {
        let dir = tempdir().expect("tempdir");
        let state = make_state(dir.path());
        let p = ReadFileParams {
            path: "/etc/hosts".into(),
            offset: None,
            limit: None,
            workdir: Some(dir.path().display().to_string()),
        };
        let out = run(&state, p).expect("ok");
        assert!(
            out.contains("localhost"),
            "expected /etc/hosts content, got: {out}"
        );
    }

    #[test]
    fn read_rejects_too_large() {
        let dir = tempdir().expect("tempdir");
        let f = dir.path().join("big.bin");
        let big = vec![b'a'; (10 * 1024 * 1024_usize) + 1024];
        fs::write(&f, &big).expect("write");
        let state = make_state(dir.path());
        let p = ReadFileParams {
            path: "big.bin".into(),
            offset: None,
            limit: None,
            workdir: Some(dir.path().display().to_string()),
        };
        let err = run(&state, p).unwrap_err();
        let msg = format!("{err:?}");
        assert!(msg.contains("too large"), "msg: {msg}");
    }

    #[test]
    fn read_offset_past_end() {
        let dir = tempdir().expect("tempdir");
        let f = dir.path().join("short.txt");
        fs::write(&f, "line1\nline2\n").expect("write");
        let state = make_state(dir.path());
        let p = ReadFileParams {
            path: "short.txt".into(),
            offset: Some(100),
            limit: None,
            workdir: Some(dir.path().display().to_string()),
        };
        let out = run(&state, p).expect("ok");
        assert!(out.contains("no lines in range"), "out: {out}");
        assert!(out.contains("file has 2 lines"), "out: {out}");
    }

    #[test]
    fn read_limit_zero() {
        let dir = tempdir().expect("tempdir");
        let f = dir.path().join("some.txt");
        fs::write(&f, "line1\nline2\n").expect("write");
        let state = make_state(dir.path());
        let p = ReadFileParams {
            path: "some.txt".into(),
            offset: None,
            limit: Some(0),
            workdir: Some(dir.path().display().to_string()),
        };
        let out = run(&state, p).expect("ok");
        assert!(out.contains("no lines in range"), "out: {out}");
    }

    #[test]
    fn read_file_without_trailing_newline() {
        let dir = tempdir().expect("tempdir");
        let f = dir.path().join("notrail.txt");
        fs::write(&f, "line1\nline2\nline3").expect("write");
        let state = make_state(dir.path());
        let p = ReadFileParams {
            path: "notrail.txt".into(),
            offset: None,
            limit: None,
            workdir: Some(dir.path().display().to_string()),
        };
        let out = run(&state, p).expect("ok");
        assert!(out.contains("lines 1-3 of 3"), "out: {out}");
        assert!(out.contains("3:line3"), "out: {out}");
    }
}
