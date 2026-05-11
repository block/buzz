use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::Command;

const MAX_LINE_BYTES: usize = 1024 * 1024; // 1MB per line — skip files with longer lines (likely binary)
const MAX_OUTPUT_BYTES: usize = 50 * 1024;
const MAX_OUTPUT_LINES: usize = 2000;
const MAX_CONTEXT: usize = 100;
const MAX_WALK_DEPTH: usize = 50;

pub fn run(args: Vec<String>) -> i32 {
    if let Some(code) = try_system_rg(&args) {
        return code;
    }
    fallback(args)
}

fn try_system_rg(args: &[String]) -> Option<i32> {
    let self_exe = std::env::current_exe().ok()?;
    let self_canon = std::fs::canonicalize(&self_exe).ok()?;
    let cleaned_path = clean_path(&self_canon);
    let candidate = which_rg(&cleaned_path)?;

    let status = Command::new(&candidate)
        .args(args)
        .env("PATH", &cleaned_path)
        .status()
        .ok()?;
    Some(status.code().unwrap_or(2))
}

fn clean_path(self_canon: &Path) -> String {
    let original = std::env::var("PATH").unwrap_or_default();
    original
        .split(':')
        .filter(|dir| {
            if dir.is_empty() {
                return false;
            }
            let candidate = Path::new(dir).join("rg");
            match std::fs::canonicalize(&candidate) {
                Ok(c) => c != *self_canon,
                Err(_) => true,
            }
        })
        .collect::<Vec<_>>()
        .join(":")
}

fn which_rg(path: &str) -> Option<PathBuf> {
    for dir in path.split(':') {
        if dir.is_empty() {
            continue;
        }
        let candidate = Path::new(dir).join("rg");
        if candidate.is_file() && is_executable(&candidate) {
            return Some(candidate);
        }
    }
    None
}

#[cfg(unix)]
fn is_executable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(p)
        .map(|m| m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(_p: &Path) -> bool {
    true
}

#[cfg_attr(test, derive(Debug))]
struct RgArgs {
    pattern: Option<String>,
    paths: Vec<PathBuf>,
    line_numbers: bool,
    ignore_case: bool,
    files_only: bool,
    list_files_with_matches: bool,
    context: usize,
    glob: Option<String>,
}

fn parse(args: Vec<String>) -> Result<RgArgs, String> {
    let mut out = RgArgs {
        pattern: None,
        paths: Vec::new(),
        line_numbers: false,
        ignore_case: false,
        files_only: false,
        list_files_with_matches: false,
        context: 0,
        glob: None,
    };
    let mut iter = args.into_iter();
    let mut positional: Vec<String> = Vec::new();
    while let Some(a) = iter.next() {
        match a.as_str() {
            "--files" => out.files_only = true,
            "-n" | "--line-number" => out.line_numbers = true,
            "-i" | "--ignore-case" => out.ignore_case = true,
            "-l" | "--files-with-matches" => out.list_files_with_matches = true,
            "-C" | "--context" => {
                let n = iter.next().ok_or("missing value for -C")?;
                let parsed: usize = n.parse().map_err(|_| format!("bad -C value: {n}"))?;
                out.context = parsed.min(MAX_CONTEXT);
            }
            "-g" | "--glob" => {
                out.glob = Some(iter.next().ok_or("missing value for -g")?);
            }
            "--" => positional.extend(iter.by_ref()),
            s if s.starts_with('-') && s.len() > 1 => {
                return Err(format!("unsupported flag (fallback rg): {s}"));
            }
            _ => positional.push(a),
        }
    }
    if out.files_only {
        out.paths = positional.into_iter().map(PathBuf::from).collect();
        if out.paths.is_empty() {
            out.paths.push(PathBuf::from("."));
        }
    } else {
        let mut it = positional.into_iter();
        out.pattern = it.next();
        out.paths = it.map(PathBuf::from).collect();
        if out.paths.is_empty() {
            out.paths.push(PathBuf::from("."));
        }
    }
    Ok(out)
}

struct CappedSink {
    bytes: usize,
    lines: usize,
    capped: bool,
}

impl CappedSink {
    fn new() -> Self {
        Self {
            bytes: 0,
            lines: 0,
            capped: false,
        }
    }

    fn writeln(&mut self, s: &str) {
        if self.capped {
            return;
        }
        let next = self.bytes.saturating_add(s.len()).saturating_add(1);
        if next > MAX_OUTPUT_BYTES || self.lines >= MAX_OUTPUT_LINES {
            self.capped = true;
            tracing::warn!("rg (fallback): output capped at {MAX_OUTPUT_BYTES} bytes / {MAX_OUTPUT_LINES} lines");
            return;
        }
        println!("{s}");
        self.bytes = next;
        self.lines += 1;
    }
}

fn fallback(args: Vec<String>) -> i32 {
    let opts = match parse(args) {
        Ok(o) => o,
        Err(e) => {
            tracing::error!("rg (fallback): {e}");
            return 2;
        }
    };
    let mut sink = CappedSink::new();
    let mut found = false;
    let mut printed: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();

    if opts.files_only {
        for root in &opts.paths {
            walk(root, &opts, &mut |p| {
                sink.writeln(&p.display().to_string());
                found = true;
                !sink.capped
            });
            if sink.capped {
                break;
            }
        }
        return if found { 0 } else { 1 };
    }

    let pattern = match &opts.pattern {
        Some(p) => p.clone(),
        None => {
            tracing::error!("rg (fallback): missing PATTERN");
            return 2;
        }
    };
    let needle = if opts.ignore_case {
        pattern.to_lowercase()
    } else {
        pattern
    };

    for root in &opts.paths {
        walk(root, &opts, &mut |path| {
            if sink.capped {
                return false;
            }
            if scan_file(path, &needle, &opts, &mut sink, &mut printed) {
                found = true;
            }
            !sink.capped
        });
        if sink.capped {
            break;
        }
    }
    if found {
        0
    } else {
        1
    }
}

fn read_bounded_line(reader: &mut impl BufRead, max: usize) -> Option<Result<String, ()>> {
    let mut buf = Vec::new();
    loop {
        let available = match reader.fill_buf() {
            Ok([]) => {
                if buf.is_empty() {
                    return None;
                }
                return match String::from_utf8(buf) {
                    Ok(s) => Some(Ok(s)),
                    Err(_) => Some(Err(())),
                };
            }
            Ok(b) => b,
            Err(_) => return None,
        };
        let take = available
            .iter()
            .position(|b| *b == b'\n')
            .map_or(available.len(), |i| i + 1);
        if buf.len() + take > max {
            return Some(Err(()));
        }
        buf.extend_from_slice(&available[..take]);
        reader.consume(take);
        if buf.ends_with(b"\n") {
            buf.pop();
            return match String::from_utf8(buf) {
                Ok(s) => Some(Ok(s)),
                Err(_) => Some(Err(())),
            };
        }
    }
}

fn scan_file(
    path: &Path,
    needle: &str,
    opts: &RgArgs,
    sink: &mut CappedSink,
    printed: &mut std::collections::HashSet<PathBuf>,
) -> bool {
    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return false,
    };
    let cap = opts.context;
    let mut ring: std::collections::VecDeque<String> =
        std::collections::VecDeque::with_capacity(cap);
    let mut tail = 0usize;
    let mut last: Option<usize> = None;
    let mut found = false;

    let mut reader = BufReader::new(file);
    let mut idx = 0usize;
    loop {
        let line = match read_bounded_line(&mut reader, MAX_LINE_BYTES) {
            None => break,
            Some(Err(())) => return found,
            Some(Ok(l)) => l,
        };
        if sink.capped {
            return found;
        }
        let is_match = if opts.ignore_case {
            line.to_lowercase().contains(needle)
        } else {
            line.contains(needle)
        };

        if is_match {
            found = true;
            if opts.list_files_with_matches {
                if printed.insert(path.to_path_buf()) {
                    sink.writeln(&path.display().to_string());
                }
                return found;
            }
            let ring_start = idx.saturating_sub(ring.len());
            for (offset, prev) in ring.iter().enumerate() {
                let li = ring_start + offset;
                if last.is_none_or(|l| li > l) {
                    emit_line(path, li, prev, opts, sink);
                    last = Some(li);
                }
            }
            if last.is_none_or(|l| idx > l) {
                emit_line(path, idx, &line, opts, sink);
                last = Some(idx);
            }
            tail = cap;
        } else if tail > 0 {
            emit_line(path, idx, &line, opts, sink);
            last = Some(idx);
            tail -= 1;
        }

        if cap > 0 {
            if ring.len() == cap {
                ring.pop_front();
            }
            ring.push_back(line);
        }
        idx += 1;
    }
    found
}

fn emit_line(path: &Path, line_idx: usize, line: &str, opts: &RgArgs, sink: &mut CappedSink) {
    let prefix = if opts.line_numbers {
        format!("{}:{}:", path.display(), line_idx + 1)
    } else {
        format!("{}:", path.display())
    };
    sink.writeln(&format!("{prefix}{line}"));
}

fn walk(root: &Path, opts: &RgArgs, on_file: &mut dyn FnMut(&Path) -> bool) {
    if root.is_file() {
        if accept(root, opts) {
            on_file(root);
        }
        return;
    }
    let mut stack: Vec<(PathBuf, usize)> = vec![(root.to_path_buf(), 0)];
    while let Some((dir, depth)) = stack.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = match path.file_name().and_then(|n| n.to_str()) {
                Some(n) => n,
                None => continue,
            };
            if name.starts_with('.') {
                continue;
            }
            let ft = match entry.file_type() {
                Ok(t) => t,
                Err(_) => continue,
            };
            if ft.is_symlink() {
                continue;
            }
            if ft.is_dir() {
                if matches!(name, "target" | "node_modules" | "dist" | "build") {
                    continue;
                }
                if depth < MAX_WALK_DEPTH {
                    stack.push((path, depth + 1));
                }
            } else if ft.is_file() && accept(&path, opts) && !on_file(&path) {
                return;
            }
        }
    }
}

fn accept(path: &Path, opts: &RgArgs) -> bool {
    match &opts.glob {
        None => true,
        Some(g) => glob_match(g, path),
    }
}

fn glob_match(pattern: &str, path: &Path) -> bool {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    let full = path.to_string_lossy();
    simple_glob(pattern, name) || simple_glob(pattern, &full)
}

fn simple_glob(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();

    let (mut pi, mut ti) = (0usize, 0usize);
    let (mut star_pi, mut star_ti) = (usize::MAX, 0usize);

    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star_pi = pi;
            star_ti = ti;
            pi += 1;
        } else if star_pi != usize::MAX {
            pi = star_pi + 1;
            star_ti += 1;
            ti = star_ti;
        } else {
            return false;
        }
    }

    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }

    pi == p.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_basic_pattern_and_path() {
        let opts = parse(vec!["-n".into(), "needle".into(), "src".into()]).expect("parse");
        assert!(opts.line_numbers);
        assert_eq!(opts.pattern.as_deref(), Some("needle"));
        assert_eq!(opts.paths, vec![PathBuf::from("src")]);
    }

    #[test]
    fn parse_files_only() {
        let opts = parse(vec!["--files".into(), ".".into()]).expect("parse");
        assert!(opts.files_only);
        assert_eq!(opts.paths, vec![PathBuf::from(".")]);
    }

    #[test]
    fn parse_rejects_unknown_flag() {
        let err = parse(vec!["-Z".into(), "x".into()]).unwrap_err();
        assert!(err.contains("unsupported flag"));
    }

    #[test]
    fn capped_sink_stops_at_byte_limit() {
        let mut s = CappedSink::new();
        // Below cap: accepted.
        s.bytes = MAX_OUTPUT_BYTES - 10;
        s.writeln("12345");
        assert!(!s.capped);
        // This pushes us over.
        s.writeln("x".repeat(20).as_str());
        assert!(s.capped);
    }

    #[test]
    fn glob_matches_simple_patterns() {
        assert!(simple_glob("*.rs", "main.rs"));
        assert!(simple_glob("src/*.rs", "src/main.rs"));
        assert!(!simple_glob("*.rs", "main.txt"));
        assert!(simple_glob("a?c", "abc"));
    }

    #[test]
    fn fallback_finds_match_in_file() {
        // End-to-end: create a file, run the fallback parser, scan it.
        // We can't easily capture stdout here, so we verify scan_file
        // returns true (match found).
        let dir = tempfile::tempdir().expect("tempdir");
        let f = dir.path().join("a.txt");
        std::fs::write(&f, "alpha\nNEEDLE\nbeta\n").expect("write");
        let opts = parse(vec!["NEEDLE".into(), f.display().to_string()]).expect("parse");
        let mut sink = CappedSink::new();
        let mut printed: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
        let found = scan_file(&f, "NEEDLE", &opts, &mut sink, &mut printed);
        assert!(found, "expected NEEDLE to be found");
    }
}
