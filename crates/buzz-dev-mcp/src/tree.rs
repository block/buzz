use std::io::Write;
use std::path::{Path, PathBuf};

use ignore::WalkBuilder;

const MAX_OUTPUT_BYTES: usize = 50 * 1024;
const MAX_OUTPUT_LINES: usize = 2000;
const MAX_WALK_DEPTH: usize = 50;
const MAX_FILE_BYTES: u64 = 10 * 1024 * 1024;

struct Frame {
    depth: usize,
    out_idx: usize,
    total: usize,
    leaf: bool,
}

pub fn run(args: Vec<String>) -> i32 {
    let (root, max_depth) = match parse(args) {
        Ok(v) => v,
        Err(e) => {
            tracing::error!("tree: {e}");
            return 2;
        }
    };
    if !root.is_dir() {
        tracing::error!("tree: not a directory: {}", root.display());
        return 2;
    }

    let mut out: Vec<String> = Vec::new();
    let mut stack: Vec<Frame> = vec![Frame {
        depth: 0,
        out_idx: 0,
        total: 0,
        leaf: false,
    }];
    let line_budget = MAX_OUTPUT_LINES.saturating_sub(1);
    let mut truncated = false;

    let mut builder = WalkBuilder::new(&root);
    builder.git_ignore(true);
    builder.git_exclude(true);
    builder.git_global(true);
    builder.require_git(false);
    builder.ignore(true);
    builder.hidden(true);
    builder.max_depth(Some(max_depth));
    builder.sort_by_file_name(|a, b| a.cmp(b));

    for entry in builder.build().flatten() {
        if entry.depth() == 0 {
            continue;
        }
        let depth = entry.depth();
        let is_dir = entry.file_type().is_some_and(|t| t.is_dir());
        let is_file = entry.file_type().is_some_and(|t| t.is_file());

        while stack.last().is_some_and(|f| f.depth >= depth) {
            let Some(frame) = stack.pop() else { break };
            if let Some(parent) = stack.last_mut() {
                parent.total = parent.total.saturating_add(frame.total);
            }
            if frame.depth > 0 && !frame.leaf {
                let placeholder = &mut out[frame.out_idx];
                *placeholder = format!("{}  [{}]", placeholder, frame.total);
            }
        }

        if out.len() >= line_budget {
            truncated = true;
            break;
        }

        let prefix = "  ".repeat(depth - 1);
        let name = entry.file_name().to_string_lossy();

        if is_dir {
            let idx = out.len();
            out.push(format!("{prefix}{name}/"));
            stack.push(Frame {
                depth,
                out_idx: idx,
                total: 0,
                leaf: depth == max_depth,
            });
        } else if is_file {
            let lc = line_count(entry.path());
            if let Some(parent) = stack.last_mut() {
                parent.total = parent.total.saturating_add(lc);
            }
            out.push(format!("{prefix}{name}  [{lc}]"));
        }
    }

    let mut grand_total = 0usize;
    while let Some(frame) = stack.pop() {
        if let Some(parent) = stack.last_mut() {
            parent.total = parent.total.saturating_add(frame.total);
        }
        if frame.depth == 0 {
            grand_total = frame.total;
        } else if !frame.leaf {
            let placeholder = &mut out[frame.out_idx];
            *placeholder = format!("{}  [{}]", placeholder, frame.total);
        }
    }

    let root_name = root
        .file_name()
        .unwrap_or(root.as_os_str())
        .to_string_lossy();

    let stdout = std::io::stdout();
    let mut w = stdout.lock();
    if writeln!(w, "{root_name}/  [{grand_total}]").is_err() {
        return 0;
    }

    let mut bytes = 0usize;
    for line in &out {
        if bytes + line.len() + 1 > MAX_OUTPUT_BYTES {
            let _ = writeln!(w, "[truncated]");
            return 0;
        }
        if writeln!(w, "{line}").is_err() {
            return 0;
        }
        bytes += line.len() + 1;
    }
    if truncated {
        let _ = writeln!(w, "[truncated]");
    }
    0
}

fn parse(args: Vec<String>) -> Result<(PathBuf, usize), String> {
    let mut depth = MAX_WALK_DEPTH;
    let mut path = PathBuf::from(".");
    let mut path_set = false;
    let mut iter = args.into_iter();
    while let Some(a) = iter.next() {
        match a.as_str() {
            "--" => {
                if let Some(p) = iter.next() {
                    if path_set {
                        return Err("multiple paths not supported".to_string());
                    }
                    path = PathBuf::from(p);
                }
                break;
            }
            "-d" | "--depth" => {
                let n = iter.next().ok_or("missing value for --depth")?;
                depth = n.parse::<usize>().map_err(|_| format!("bad depth: {n}"))?;
            }
            s if s.starts_with('-') => return Err(format!("unknown flag: {s}")),
            _ => {
                if path_set {
                    return Err("multiple paths not supported".to_string());
                }
                path = PathBuf::from(a);
                path_set = true;
            }
        }
    }
    Ok((path, depth.min(MAX_WALK_DEPTH)))
}

fn line_count(path: &Path) -> usize {
    std::fs::metadata(path)
        .ok()
        .filter(|m| m.is_file() && m.len() <= MAX_FILE_BYTES)
        .and_then(|_| std::fs::read(path).ok())
        .map(|b| {
            if b.is_empty() {
                0
            } else {
                b.iter().filter(|&&c| c == b'\n').count()
                    + if b.last() != Some(&b'\n') { 1 } else { 0 }
            }
        })
        .unwrap_or(0)
}
