use super::project_git::{ProjectRepoCommitInfo, ProjectRepoFileInfo};
use std::{collections::HashMap, path::Path, time::UNIX_EPOCH};

const MAX_REPOSITORY_FILE_PAYLOAD: usize = 250;

#[derive(Default)]
pub(super) struct ParsedProjectRepoFiles {
    pub(super) files: Vec<ProjectRepoFileInfo>,
    pub(super) total_file_count: usize,
}

fn read_preview_content(repo_dir: &Path, path: &str, size: Option<u64>) -> Option<String> {
    const MAX_PREVIEW_BYTES: u64 = 64 * 1024;
    if size.is_some_and(|value| value > MAX_PREVIEW_BYTES) {
        return None;
    }

    let normalized = repo_dir.join(path).canonicalize().ok()?;
    let repo_root = repo_dir.canonicalize().ok()?;
    if !normalized.starts_with(repo_root) {
        return None;
    }

    let bytes = std::fs::read(normalized).ok()?;
    if bytes.contains(&0) {
        return None;
    }
    String::from_utf8(bytes).ok()
}

fn path_modified_at(path: &Path) -> Option<i64> {
    let modified = std::fs::metadata(path).ok()?.modified().ok()?;
    modified
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_secs()).ok())
}

pub(super) fn parse_worktree_files(
    repo_dir: &Path,
    output: &str,
    latest_commit_by_path: &HashMap<String, ProjectRepoCommitInfo>,
) -> ParsedProjectRepoFiles {
    let mut parsed = ParsedProjectRepoFiles::default();

    for path in output.split('\0').filter(|path| !path.trim().is_empty()) {
        let full_path = repo_dir.join(path);
        let Ok(metadata) = std::fs::metadata(&full_path) else {
            continue;
        };
        if !metadata.is_file() {
            continue;
        }

        parsed.total_file_count += 1;
        if parsed.files.len() >= MAX_REPOSITORY_FILE_PAYLOAD {
            continue;
        }

        let size = Some(metadata.len());
        let latest_commit = latest_commit_by_path.get(path).cloned();
        parsed.files.push(ProjectRepoFileInfo {
            path: path.to_string(),
            kind: "blob".to_string(),
            size,
            preview_content: read_preview_content(repo_dir, path, size),
            last_changed_at: latest_commit
                .as_ref()
                .map(|commit| commit.timestamp)
                .or_else(|| path_modified_at(&full_path)),
            latest_commit,
        });
    }

    parsed
}

pub(super) fn parse_ls_tree(
    repo_dir: &Path,
    output: &str,
    latest_commit_by_path: &HashMap<String, ProjectRepoCommitInfo>,
) -> ParsedProjectRepoFiles {
    let mut parsed = ParsedProjectRepoFiles::default();

    for line in output.lines() {
        let Some((meta, path)) = line.split_once('\t') else {
            continue;
        };
        let mut parts = meta.split_whitespace();
        let (Some(_mode), Some(kind), Some(_object)) = (parts.next(), parts.next(), parts.next())
        else {
            continue;
        };
        let size = parts.next().and_then(|value| value.parse::<u64>().ok());

        parsed.total_file_count += 1;
        if parsed.files.len() >= MAX_REPOSITORY_FILE_PAYLOAD {
            continue;
        }

        let preview_content = (kind == "blob")
            .then(|| read_preview_content(repo_dir, path, size))
            .flatten();
        parsed.files.push(ProjectRepoFileInfo {
            path: path.to_string(),
            kind: kind.to_string(),
            size,
            preview_content,
            last_changed_at: latest_commit_by_path
                .get(path)
                .map(|commit| commit.timestamp),
            latest_commit: latest_commit_by_path.get(path).cloned(),
        });
    }

    parsed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ls_tree_reports_the_full_count_when_the_payload_is_capped() {
        let output = (0..251)
            .map(|index| format!("100644 blob {} 1\tfile-{index:03}.txt", "0".repeat(40)))
            .collect::<Vec<_>>()
            .join("\n");

        let parsed = parse_ls_tree(Path::new("/missing-repository"), &output, &HashMap::new());

        assert_eq!(parsed.files.len(), 250);
        assert_eq!(parsed.total_file_count, 251);
    }

    #[test]
    fn worktree_reports_the_full_count_when_the_payload_is_capped() {
        let repo = tempfile::tempdir().expect("create temporary repository");
        let mut paths = Vec::new();
        for index in 0..251 {
            let path = format!("file-{index:03}.txt");
            std::fs::write(repo.path().join(&path), "x").expect("write fixture file");
            paths.push(path);
        }
        let output = format!("{}\0", paths.join("\0"));

        let parsed = parse_worktree_files(repo.path(), &output, &HashMap::new());

        assert_eq!(parsed.files.len(), 250);
        assert_eq!(parsed.total_file_count, 251);
    }
}
