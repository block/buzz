use ignore::WalkBuilder;
use rmcp::ErrorData;
use schemars::JsonSchema;
use serde::Deserialize;
use std::path::{Path, PathBuf};

const MAX_PATH_BYTES: usize = 4 * 1024;
const MAX_FILE_BYTES: u64 = 1024 * 1024;
const MAX_READ_LINES: usize = 2_000;
const MAX_LIST_ENTRIES: usize = 1_000;
const MAX_SEARCH_MATCHES: usize = 1_000;
const MAX_SEARCH_QUERY_BYTES: usize = 4 * 1024;
const MAX_SEARCH_BYTES: u64 = 16 * 1024 * 1024;
const MAX_SEARCH_ENTRIES: usize = 50_000;
const MAX_TOOL_OUTPUT_BYTES: usize = 512 * 1024;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct FilesReadParams {
    /// File path, absolute or relative to the managed workspace root.
    pub path: String,
    /// Zero-based line offset. Defaults to zero.
    #[serde(default)]
    pub offset: Option<usize>,
    /// Maximum lines to return. Defaults to 200 and is capped at 2,000.
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct FilesListParams {
    /// Directory path, absolute or relative to the managed workspace root. Defaults to root.
    #[serde(default)]
    pub path: Option<String>,
    /// Maximum entries to return. Defaults to 200 and is capped at 1,000.
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct SearchTextParams {
    /// Literal text to find.
    pub query: String,
    /// File or directory path, absolute or relative to the managed workspace root. Defaults to root.
    #[serde(default)]
    pub path: Option<String>,
    /// Whether matching is case-sensitive. Defaults to true.
    #[serde(default = "default_true")]
    pub case_sensitive: bool,
    /// Maximum matches to return. Defaults to 200 and is capped at 1,000.
    #[serde(default)]
    pub limit: Option<usize>,
}

fn default_true() -> bool {
    true
}

pub(crate) fn canonical_root(root: PathBuf) -> Result<PathBuf, ErrorData> {
    let root = std::fs::canonicalize(&root).map_err(|error| {
        ErrorData::internal_error(format!("managed workspace is unavailable: {error}"), None)
    })?;
    if !root.is_dir() {
        return Err(ErrorData::internal_error(
            "managed workspace is not a directory",
            None,
        ));
    }
    Ok(root)
}

fn resolve_contained(root: &Path, supplied: &str) -> Result<PathBuf, ErrorData> {
    if supplied.len() > MAX_PATH_BYTES {
        return Err(ErrorData::invalid_params("path exceeds 4 KiB", None));
    }
    let path = Path::new(supplied);
    let candidate = if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    };
    let resolved = std::fs::canonicalize(&candidate).map_err(|error| {
        ErrorData::invalid_params(
            format!("path is not accessible: {} ({error})", candidate.display()),
            None,
        )
    })?;
    if resolved != root && !resolved.starts_with(root) {
        return Err(ErrorData::invalid_params(
            "path escapes the managed workspace",
            None,
        ));
    }
    Ok(resolved)
}

fn relative_display<'a>(root: &'a Path, path: &'a Path) -> &'a Path {
    path.strip_prefix(root).unwrap_or(path)
}

fn append_bounded_marker(output: &mut String, marker: &str) {
    let mut keep = MAX_TOOL_OUTPUT_BYTES.saturating_sub(marker.len());
    keep = keep.min(output.len());
    while !output.is_char_boundary(keep) {
        keep -= 1;
    }
    output.truncate(keep);
    output.push_str(marker);
}

pub(crate) fn files_read(root: &Path, params: FilesReadParams) -> Result<String, ErrorData> {
    let target = resolve_contained(root, &params.path)?;
    let metadata = std::fs::metadata(&target).map_err(|error| {
        ErrorData::internal_error(format!("cannot stat {}: {error}", target.display()), None)
    })?;
    if !metadata.is_file() {
        return Err(ErrorData::invalid_params(
            "path is not a regular file",
            None,
        ));
    }
    if metadata.len() > MAX_FILE_BYTES {
        return Err(ErrorData::invalid_params("file exceeds 1 MiB", None));
    }
    let content = std::fs::read_to_string(&target).map_err(|error| {
        ErrorData::invalid_params(
            format!("file is not readable UTF-8: {} ({error})", target.display()),
            None,
        )
    })?;
    let offset = params.offset.unwrap_or(0);
    let limit = params.limit.unwrap_or(200).min(MAX_READ_LINES);
    let mut output = String::new();
    let mut returned = 0usize;
    let mut truncated = false;
    for (index, line) in content.lines().enumerate().skip(offset).take(limit) {
        let rendered = format!("{}:{}\n", index + 1, line);
        if output.len().saturating_add(rendered.len()) > MAX_TOOL_OUTPUT_BYTES {
            truncated = true;
            break;
        }
        output.push_str(&rendered);
        returned += 1;
    }
    if truncated {
        append_bounded_marker(&mut output, "[output truncated at 512 KiB]\n");
    } else if returned == limit && content.lines().count() > offset.saturating_add(returned) {
        append_bounded_marker(
            &mut output,
            "[more lines available; increase offset to continue]\n",
        );
    }
    Ok(output)
}

pub(crate) fn files_list(root: &Path, params: FilesListParams) -> Result<String, ErrorData> {
    let target = resolve_contained(root, params.path.as_deref().unwrap_or("."))?;
    if !target.is_dir() {
        return Err(ErrorData::invalid_params("path is not a directory", None));
    }
    let limit = params.limit.unwrap_or(200).min(MAX_LIST_ENTRIES);
    let mut entries = std::fs::read_dir(&target)
        .map_err(|error| {
            ErrorData::internal_error(format!("cannot list {}: {error}", target.display()), None)
        })?
        .take(limit.saturating_add(1))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| {
            ErrorData::internal_error(format!("cannot read directory: {error}"), None)
        })?;
    let has_more = entries.len() > limit;
    entries.truncate(limit);
    entries.sort_by_key(|entry| entry.file_name());

    let mut output = String::new();
    let mut output_truncated = false;
    for entry in entries {
        let file_type = entry.file_type().map_err(|error| {
            ErrorData::internal_error(format!("cannot inspect directory entry: {error}"), None)
        })?;
        let kind = if file_type.is_dir() {
            "dir"
        } else if file_type.is_file() {
            "file"
        } else if file_type.is_symlink() {
            "symlink"
        } else {
            "other"
        };
        let rendered = format!(
            "{}\t{}\n",
            kind,
            relative_display(root, &entry.path()).display()
        );
        if output.len().saturating_add(rendered.len()) > MAX_TOOL_OUTPUT_BYTES {
            append_bounded_marker(&mut output, "[output truncated at 512 KiB]\n");
            output_truncated = true;
            break;
        }
        output.push_str(&rendered);
    }
    if has_more && !output_truncated {
        append_bounded_marker(
            &mut output,
            "[more entries available; lower the path scope]\n",
        );
    }
    Ok(output)
}

pub(crate) fn search_text(root: &Path, params: SearchTextParams) -> Result<String, ErrorData> {
    if params.query.is_empty() {
        return Err(ErrorData::invalid_params("query must not be empty", None));
    }
    if params.query.len() > MAX_SEARCH_QUERY_BYTES {
        return Err(ErrorData::invalid_params("query exceeds 4 KiB", None));
    }
    let target = resolve_contained(root, params.path.as_deref().unwrap_or("."))?;
    let limit = params.limit.unwrap_or(200).min(MAX_SEARCH_MATCHES);
    let needle = if params.case_sensitive {
        None
    } else {
        Some(params.query.to_lowercase())
    };
    let mut output = String::new();
    let mut matches = 0usize;
    let mut scanned = 0u64;
    let mut visited = 0usize;
    let mut search_limited = false;

    let walker = WalkBuilder::new(&target)
        .follow_links(false)
        .standard_filters(true)
        .build();
    for entry in walker.filter_map(Result::ok) {
        visited += 1;
        if visited > MAX_SEARCH_ENTRIES {
            search_limited = true;
            break;
        }
        if matches >= limit {
            break;
        }
        if scanned >= MAX_SEARCH_BYTES {
            search_limited = true;
            break;
        }
        let Some(file_type) = entry.file_type() else {
            continue;
        };
        if !file_type.is_file() {
            continue;
        }
        let path = entry.path();
        let resolved = match std::fs::canonicalize(path) {
            Ok(path) if path == root || path.starts_with(root) => path,
            _ => continue,
        };
        let metadata = match std::fs::metadata(&resolved) {
            Ok(metadata) if metadata.is_file() && metadata.len() <= MAX_FILE_BYTES => metadata,
            _ => continue,
        };
        if scanned.saturating_add(metadata.len()) > MAX_SEARCH_BYTES {
            search_limited = true;
            break;
        }
        scanned += metadata.len();
        let content = match std::fs::read_to_string(&resolved) {
            Ok(content) => content,
            Err(_) => continue,
        };
        for (line_index, line) in content.lines().enumerate() {
            let found = match &needle {
                Some(needle) => line.to_lowercase().contains(needle),
                None => line.contains(&params.query),
            };
            if !found {
                continue;
            }
            let rendered = format!(
                "{}:{}:{}\n",
                relative_display(root, &resolved).display(),
                line_index + 1,
                line
            );
            if output.len().saturating_add(rendered.len()) > MAX_TOOL_OUTPUT_BYTES {
                append_bounded_marker(&mut output, "[output truncated at 512 KiB]\n");
                return Ok(output);
            }
            output.push_str(&rendered);
            matches += 1;
            if matches >= limit {
                break;
            }
        }
    }
    if search_limited {
        append_bounded_marker(
            &mut output,
            "[search stopped at the 16 MiB or 50,000-entry bound]\n",
        );
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn rejects_absolute_escape() {
        let root = tempfile::tempdir().unwrap();
        let root_path = canonical_root(root.path().to_owned()).unwrap();
        let outside = tempfile::NamedTempFile::new().unwrap();
        let result = files_read(
            &root_path,
            FilesReadParams {
                path: outside.path().display().to_string(),
                offset: None,
                limit: None,
            },
        );
        assert!(result.is_err());
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlink_escape() {
        use std::os::unix::fs::symlink;
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::NamedTempFile::new().unwrap();
        symlink(outside.path(), root.path().join("escape")).unwrap();
        let root_path = canonical_root(root.path().to_owned()).unwrap();
        let result = files_read(
            &root_path,
            FilesReadParams {
                path: "escape".into(),
                offset: None,
                limit: None,
            },
        );
        assert!(result.is_err());
    }

    #[test]
    fn reads_lists_and_searches_inside_root() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("safe.txt"), "alpha\nneedle\nomega\n").unwrap();
        let root_path = canonical_root(root.path().to_owned()).unwrap();
        let read = files_read(
            &root_path,
            FilesReadParams {
                path: "safe.txt".into(),
                offset: Some(1),
                limit: Some(1),
            },
        )
        .unwrap();
        assert_eq!(
            read,
            "2:needle\n[more lines available; increase offset to continue]\n"
        );
        let listed = files_list(
            &root_path,
            FilesListParams {
                path: None,
                limit: None,
            },
        )
        .unwrap();
        assert!(listed.contains("file\tsafe.txt"));
        let found = search_text(
            &root_path,
            SearchTextParams {
                query: "needle".into(),
                path: None,
                case_sensitive: true,
                limit: None,
            },
        )
        .unwrap();
        assert_eq!(found, "safe.txt:2:needle\n");
    }
}
