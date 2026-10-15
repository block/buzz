use super::project_git_exec::{
    build_git_auth_config, clean_branch, run_git, validate_workspace_clone_url, GitAuthConfig,
};
use super::project_repo_paths::find_local_repo_dir;
use crate::app_state::AppState;
use serde::Serialize;
use tauri::State;

/// Per-file cap on rendered patch lines. One regenerated lockfile or
/// minified bundle would otherwise produce tens of thousands of DOM nodes
/// in the diff view and freeze the webview.
const MAX_PATCH_LINES: usize = 2_000;

#[derive(Serialize)]
pub struct ProjectRepoDiffFileInfo {
    pub path: String,
    pub additions: usize,
    pub deletions: usize,
    pub patch: String,
    pub truncated: bool,
}

#[derive(Serialize)]
pub struct ProjectRepoDiffInfo {
    pub files: Vec<ProjectRepoDiffFileInfo>,
    pub additions: usize,
    pub deletions: usize,
    pub commit_body: Option<String>,
}

fn clean_target_ref(value: Option<String>) -> Option<String> {
    value.filter(|value| {
        value.starts_with("refs/")
            && !value.contains("..")
            && value
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '_' | '.' | '-'))
    })
}

pub(crate) fn clean_commit(value: Option<String>) -> Option<String> {
    value
        .filter(|value| matches!(value.len(), 40 | 64))
        .filter(|value| value.chars().all(|c| c.is_ascii_hexdigit()))
}

fn fetch_target(
    repo_dir: &std::path::Path,
    auth: &GitAuthConfig,
    branch: Option<&str>,
    target_ref: Option<&str>,
    target_commit: Option<&str>,
) -> Result<(), String> {
    if let Some(target_ref) = target_ref {
        if run_git(
            &["fetch", "--depth=100", "origin", target_ref],
            Some(repo_dir),
            auth,
        )
        .is_ok()
        {
            run_git(
                &["checkout", "--detach", "FETCH_HEAD"],
                Some(repo_dir),
                auth,
            )?;
            return Ok(());
        }
    } else if let Some(target_commit) = target_commit {
        if run_git(
            &["fetch", "--depth=100", "origin", target_commit],
            Some(repo_dir),
            auth,
        )
        .is_ok()
        {
            run_git(
                &["checkout", "--detach", "FETCH_HEAD"],
                Some(repo_dir),
                auth,
            )?;
            return Ok(());
        }
    }

    if let Some(target_commit) = target_commit {
        if run_git(
            &["fetch", "--depth=100", "origin", target_commit],
            Some(repo_dir),
            auth,
        )
        .is_ok()
        {
            run_git(
                &["checkout", "--detach", "FETCH_HEAD"],
                Some(repo_dir),
                auth,
            )?;
            return Ok(());
        }
    }

    if let Some(branch) = branch {
        let refspec = format!("refs/heads/{branch}:refs/remotes/origin/{branch}");
        run_git(
            &["fetch", "--depth=100", "origin", &refspec],
            Some(repo_dir),
            auth,
        )?;
        run_git(
            &["checkout", "--detach", &format!("origin/{branch}")],
            Some(repo_dir),
            auth,
        )?;
        return Ok(());
    }

    run_git(
        &["fetch", "--depth=100", "origin", "HEAD"],
        Some(repo_dir),
        auth,
    )?;
    run_git(
        &["checkout", "--detach", "FETCH_HEAD"],
        Some(repo_dir),
        auth,
    )?;
    Ok(())
}

fn diff_base_ref(
    repo_dir: &std::path::Path,
    auth: &GitAuthConfig,
    base_branch: Option<&str>,
) -> Option<String> {
    let base_branch = base_branch?;
    let refspec = format!("refs/heads/{base_branch}:refs/remotes/origin/{base_branch}");
    run_git(
        &["fetch", "--depth=100", "origin", &refspec],
        Some(repo_dir),
        auth,
    )
    .ok()?;
    Some(format!("origin/{base_branch}"))
}

fn parse_count(value: &str) -> usize {
    value.parse::<usize>().unwrap_or_default()
}

/// Parses `git diff --numstat -z` output.
///
/// NUL-separated records sidestep git's path quoting entirely — not just the
/// octal escaping of non-ASCII bytes that `core.quotepath=false` disables, but
/// also the quoting git still applies to paths containing `"`, a backslash or
/// a newline. The path handed back is therefore the literal one on disk and is
/// safe to reuse as a pathspec.
///
/// Record shapes:
/// - normal: `additions \t deletions \t path NUL`
/// - rename or copy: `additions \t deletions \t NUL old NUL new NUL` — the
///   empty third field signals that two more NUL-terminated fields follow.
///   The new path is reported, because that is what the patch and the UI
///   refer to.
///
/// Binary files carry `-` counts; [`parse_count`] maps those to 0 and the
/// record is kept, so a binary change still shows up as a file.
fn parse_numstat(output: &str) -> Vec<(String, usize, usize)> {
    let mut records = output.split('\0');
    let mut files = Vec::new();
    while let Some(record) = records.next() {
        if record.is_empty() {
            continue;
        }
        let mut fields = record.splitn(3, '\t');
        let (Some(additions), Some(deletions), Some(path)) =
            (fields.next(), fields.next(), fields.next())
        else {
            continue;
        };
        let path = if path.is_empty() {
            // Rename or copy: skip the old path, take the new one.
            records.next();
            match records.next() {
                Some(new_path) if !new_path.is_empty() => new_path,
                _ => continue,
            }
        } else {
            path
        };
        files.push((
            path.to_string(),
            parse_count(additions),
            parse_count(deletions),
        ));
        if files.len() == 250 {
            break;
        }
    }
    files
}

fn empty_tree_ref(repo_dir: &std::path::Path, auth: &GitAuthConfig) -> Result<String, String> {
    run_git(
        &["hash-object", "-t", "tree", "/dev/null"],
        Some(repo_dir),
        auth,
    )
    .map(|output| output.trim().to_string())
}

fn diff_range(
    repo_dir: &std::path::Path,
    auth: &GitAuthConfig,
    base_ref: Option<String>,
) -> String {
    if let Some(base_ref) = base_ref {
        return if run_git(&["merge-base", &base_ref, "HEAD"], Some(repo_dir), auth).is_ok() {
            format!("{base_ref}...HEAD")
        } else {
            format!("{base_ref}..HEAD")
        };
    }

    empty_tree_ref(repo_dir, auth)
        .map(|empty_tree| format!("{empty_tree}..HEAD"))
        .unwrap_or_else(|_| "HEAD^..HEAD".to_string())
}

/// Range for a single commit against its parent, used by the commit detail
/// view. Root commits fall back to the empty tree so the whole initial tree
/// renders as additions. Errors when the commit is not reachable in the
/// available history — diffing an unrelated ref instead would be misleading.
fn commit_parent_range(
    repo_dir: &std::path::Path,
    auth: &GitAuthConfig,
    commit: &str,
) -> Result<String, String> {
    run_git(
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("{commit}^{{commit}}"),
        ],
        Some(repo_dir),
        auth,
    )
    .map_err(|_| format!("commit {commit} was not found in the repository history"))?;
    let parent = format!("{commit}^");
    if run_git(
        &["rev-parse", "--verify", "--quiet", &parent],
        Some(repo_dir),
        auth,
    )
    .is_ok()
    {
        return Ok(format!("{parent}..{commit}"));
    }
    let empty_tree = empty_tree_ref(repo_dir, auth)?;
    Ok(format!("{empty_tree}..{commit}"))
}

fn local_ref_exists(repo_dir: &std::path::Path, auth: &GitAuthConfig, ref_name: &str) -> bool {
    run_git(
        &["rev-parse", "--verify", "--quiet", ref_name],
        Some(repo_dir),
        auth,
    )
    .is_ok()
}

fn local_target_ref(
    repo_dir: &std::path::Path,
    auth: &GitAuthConfig,
    branch: Option<&str>,
    target_commit: Option<&str>,
) -> String {
    if let Some(target_commit) = target_commit {
        if local_ref_exists(repo_dir, auth, target_commit) {
            return target_commit.to_string();
        }
    }
    if let Some(branch) = branch {
        if local_ref_exists(repo_dir, auth, branch) {
            return branch.to_string();
        }
        let origin_branch = format!("origin/{branch}");
        if local_ref_exists(repo_dir, auth, &origin_branch) {
            return origin_branch;
        }
    }
    "HEAD".to_string()
}

fn local_base_ref(
    repo_dir: &std::path::Path,
    auth: &GitAuthConfig,
    branch: Option<&str>,
    target_branch: Option<&str>,
) -> Option<String> {
    let branch = branch?;
    let origin_branch = format!("origin/{branch}");
    if local_ref_exists(repo_dir, auth, &origin_branch) {
        return Some(origin_branch);
    }
    if target_branch == Some(branch) {
        return None;
    }
    local_ref_exists(repo_dir, auth, branch).then_some(branch.to_string())
}

fn local_diff_range(
    repo_dir: &std::path::Path,
    auth: &GitAuthConfig,
    base_branch: Option<&str>,
    target_branch: Option<&str>,
    base_commit: Option<&str>,
    target_commit: Option<&str>,
) -> String {
    let target_ref = local_target_ref(repo_dir, auth, target_branch, target_commit);
    if let Some(base_commit) = base_commit {
        if base_commit != target_ref && local_ref_exists(repo_dir, auth, base_commit) {
            return if run_git(
                &["merge-base", base_commit, &target_ref],
                Some(repo_dir),
                auth,
            )
            .is_ok()
            {
                format!("{base_commit}...{target_ref}")
            } else {
                format!("{base_commit}..{target_ref}")
            };
        }
    }
    if let Some(base_ref) = local_base_ref(repo_dir, auth, base_branch, target_branch) {
        return if run_git(
            &["merge-base", &base_ref, &target_ref],
            Some(repo_dir),
            auth,
        )
        .is_ok()
        {
            format!("{base_ref}...{target_ref}")
        } else {
            format!("{base_ref}..{target_ref}")
        };
    }
    // With no base at all, a bare commit means "diff against its parent"
    // (commit detail view) rather than against the whole tree.
    if base_commit.is_none() && base_branch.is_none() {
        if let Some(target_commit) = target_commit {
            if local_ref_exists(repo_dir, auth, target_commit) {
                if let Ok(range) = commit_parent_range(repo_dir, auth, target_commit) {
                    return range;
                }
            }
        }
    }
    empty_tree_ref(repo_dir, auth)
        .map(|empty_tree| format!("{empty_tree}..{target_ref}"))
        .unwrap_or_else(|_| format!("{target_ref}^..{target_ref}"))
}

/// Caps a patch at [`MAX_PATCH_LINES`], reporting whether it was cut.
fn truncate_patch(patch: String) -> (String, bool) {
    let mut line_starts = patch
        .char_indices()
        .filter(|(_, c)| *c == '\n')
        .map(|(index, _)| index);
    match line_starts.nth(MAX_PATCH_LINES - 1) {
        Some(cut_at) => (patch[..cut_at].to_string(), true),
        None => (patch, false),
    }
}

fn diff_from_repo(
    repo_dir: &std::path::Path,
    auth: &GitAuthConfig,
    range: &str,
    target_commit: Option<&str>,
) -> Result<ProjectRepoDiffInfo, String> {
    let commit_body = target_commit
        .map(|commit| {
            run_git(
                &[
                    "show",
                    "--no-patch",
                    "--format=%b",
                    "--end-of-options",
                    commit,
                ],
                Some(repo_dir),
                auth,
            )
            .map(|body| body.trim_end().to_string())
        })
        .transpose()?
        .filter(|body| !body.is_empty());
    let numstat = run_git(&["diff", "--numstat", "-z", range], Some(repo_dir), auth)?;
    let files = parse_numstat(&numstat)
        .into_iter()
        .map(|(path, additions, deletions)| {
            // `:(literal)` keeps glob metacharacters (`*`, `?`, `[`) in a file
            // name from being read as a pattern.
            let pathspec = format!(":(literal){path}");
            let patch = run_git(
                &[
                    "diff",
                    "--no-ext-diff",
                    "--find-renames",
                    "--find-copies",
                    "--unified=80",
                    "--src-prefix=a/",
                    "--dst-prefix=b/",
                    range,
                    "--",
                    &pathspec,
                ],
                Some(repo_dir),
                auth,
            )
            .unwrap_or_else(|error| {
                tracing::warn!("project diff: git diff failed for {path}: {error}");
                String::new()
            });
            if patch.is_empty() && (additions > 0 || deletions > 0) {
                // git exits 0 with empty stdout when a pathspec matches
                // nothing, so a mismatch between the numstat path and the
                // pathspec would otherwise be silent.
                tracing::warn!(
                    "project diff: empty patch for {path} despite +{additions} -{deletions}"
                );
            }
            let (patch, truncated) = truncate_patch(patch);
            ProjectRepoDiffFileInfo {
                path,
                additions,
                deletions,
                patch,
                truncated,
            }
        })
        .collect::<Vec<_>>();
    Ok(ProjectRepoDiffInfo {
        additions: files.iter().map(|file| file.additions).sum(),
        deletions: files.iter().map(|file| file.deletions).sum(),
        commit_body,
        files,
    })
}

#[tauri::command]
pub async fn get_project_repo_diff(
    clone_url: String,
    default_branch: Option<String>,
    base_branch: Option<String>,
    target_ref: Option<String>,
    target_commit: Option<String>,
    state: State<'_, AppState>,
) -> Result<ProjectRepoDiffInfo, String> {
    validate_workspace_clone_url(&clone_url, &state)?;
    let auth = build_git_auth_config(&state)?;
    let branch = clean_branch(default_branch);
    let base_branch = clean_branch(base_branch);
    let target_ref = clean_target_ref(target_ref);
    let target_commit = clean_commit(target_commit);

    tauri::async_runtime::spawn_blocking(move || {
        let temp_dir = tempfile::tempdir().map_err(|error| format!("create temp dir: {error}"))?;
        let repo_dir = temp_dir.path().join("repo");
        let repo_path = repo_dir
            .to_str()
            .ok_or_else(|| "temporary repository path is not UTF-8".to_string())?;
        run_git(
            &[
                "clone",
                "--filter=blob:none",
                "--no-checkout",
                &clone_url,
                repo_path,
            ],
            None,
            &auth,
        )?;
        fetch_target(
            &repo_dir,
            &auth,
            branch.as_deref(),
            target_ref.as_deref(),
            target_commit.as_deref(),
        )?;
        // A commit with no base branch or target ref means "diff this commit
        // against its parent" (commit detail view), not "diff HEAD against a
        // base".
        let range = match (&target_ref, &base_branch, &target_commit) {
            (None, None, Some(commit)) => commit_parent_range(&repo_dir, &auth, commit)?,
            _ => diff_range(
                &repo_dir,
                &auth,
                diff_base_ref(&repo_dir, &auth, base_branch.as_deref()),
            ),
        };
        let commit_body_ref = if target_ref.is_none() && base_branch.is_none() {
            target_commit.as_deref()
        } else {
            None
        };
        diff_from_repo(&repo_dir, &auth, &range, commit_body_ref)
    })
    .await
    .map_err(|error| format!("repo diff task failed: {error}"))?
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn get_project_local_repo_diff(
    repos_dir: Option<String>,
    project_dtag: String,
    clone_url: Option<String>,
    default_branch: Option<String>,
    base_branch: Option<String>,
    base_commit: Option<String>,
    target_commit: Option<String>,
    state: State<'_, AppState>,
) -> Result<Option<ProjectRepoDiffInfo>, String> {
    let auth = build_git_auth_config(&state)?;
    let branch = clean_branch(default_branch);
    let base_branch = clean_branch(base_branch);
    let base_commit = clean_commit(base_commit);
    let target_commit = clean_commit(target_commit);

    tauri::async_runtime::spawn_blocking(move || {
        let Some(repo_dir) =
            find_local_repo_dir(repos_dir.as_deref(), &project_dtag, clone_url.as_deref())?
        else {
            return Ok(None);
        };
        let range = local_diff_range(
            &repo_dir,
            &auth,
            base_branch.as_deref(),
            branch.as_deref(),
            base_commit.as_deref(),
            target_commit.as_deref(),
        );
        let commit_body_ref = if base_commit.is_none() && base_branch.is_none() {
            target_commit.as_deref()
        } else {
            None
        };
        diff_from_repo(&repo_dir, &auth, &range, commit_body_ref).map(Some)
    })
    .await
    .map_err(|error| format!("local repo diff task failed: {error}"))?
}

#[cfg(test)]
mod tests {
    use super::{diff_from_repo, parse_numstat};
    use crate::commands::project_git_exec::{build_test_git_auth_config, run_git, GitAuthConfig};

    /// A path that exercises both octal escaping (`ä` → `\303\244`) and a
    /// multi-byte punctuation character (`—` → `\342\200\224`), plus a space.
    const NON_ASCII_PATH: &str = "Beppo-Aufträge/B1 — Ergebnis.md";

    #[test]
    fn parse_numstat_reads_nul_terminated_records() {
        assert_eq!(
            parse_numstat("1\t0\tsrc/main.rs\0"),
            vec![("src/main.rs".to_string(), 1, 0)]
        );
    }

    #[test]
    fn parse_numstat_keeps_non_ascii_paths_verbatim() {
        let output = format!("158\t0\t{NON_ASCII_PATH}\0");
        assert_eq!(
            parse_numstat(&output),
            vec![(NON_ASCII_PATH.to_string(), 158, 0)]
        );
    }

    #[test]
    fn parse_numstat_reports_the_new_path_of_a_rename() {
        assert_eq!(
            parse_numstat("0\t0\t\0alt.md\0neu.md\0"),
            vec![("neu.md".to_string(), 0, 0)]
        );
    }

    #[test]
    fn parse_numstat_reads_a_mixed_record_sequence() {
        let output = format!(
            "3\t1\ta.rs\0\
             -\t-\tbin.dat\0\
             7\t2\t\0old.md\0new.md\0\
             9\t0\t{NON_ASCII_PATH}\0"
        );
        assert_eq!(
            parse_numstat(&output),
            vec![
                ("a.rs".to_string(), 3, 1),
                // Binary counts are `-`; the record must survive as a 0/0 file
                // rather than being swallowed.
                ("bin.dat".to_string(), 0, 0),
                ("new.md".to_string(), 7, 2),
                (NON_ASCII_PATH.to_string(), 9, 0),
            ]
        );
    }

    #[test]
    fn parse_numstat_caps_the_file_list() {
        let output = (0..300)
            .map(|index| format!("1\t0\tfile{index}.txt\0"))
            .collect::<String>();
        assert_eq!(parse_numstat(&output).len(), 250);
    }

    fn commit(repo: &std::path::Path, auth: &GitAuthConfig, message: &str) {
        run_git(&["add", "-A"], Some(repo), auth).expect("stage fixture");
        run_git(
            &[
                "-c",
                "user.name=Buzz Test",
                "-c",
                "user.email=test@example.com",
                "commit",
                "-m",
                message,
            ],
            Some(repo),
            auth,
        )
        .expect("commit fixture");
    }

    #[test]
    fn diff_reports_unescaped_non_ascii_paths_with_a_populated_patch() {
        let auth = build_test_git_auth_config().expect("build test git config");
        let root = tempfile::tempdir().expect("create test directory");
        let repo = root.path().join("repo");
        run_git(
            &["init", "--", repo.to_str().expect("repo path")],
            None,
            &auth,
        )
        .expect("init repo");

        let file = repo.join(NON_ASCII_PATH);
        std::fs::create_dir_all(file.parent().expect("parent")).expect("create directory");
        std::fs::write(&file, "hallo\n").expect("write fixture");
        commit(&repo, &auth, "init");
        std::fs::write(&file, "hallo\nzeile2\n").expect("update fixture");
        commit(&repo, &auth, "second");

        let diff = diff_from_repo(&repo, &auth, "HEAD~1..HEAD", None).expect("diff repo");
        let [file] = diff.files.as_slice() else {
            panic!(
                "expected exactly one changed file, got {}",
                diff.files.len()
            );
        };
        assert_eq!(file.path, NON_ASCII_PATH);
        assert_eq!((file.additions, file.deletions), (1, 0));
        assert!(
            file.patch.contains("+zeile2"),
            "patch should carry the changed line, got {:?}",
            file.patch
        );
        // The patch header repeats the path; without `core.quotepath=false`
        // it arrives octal-escaped and the UI renders it that way.
        assert!(
            file.patch.contains(&format!("--- a/{NON_ASCII_PATH}")),
            "patch header should carry the unescaped path, got {:?}",
            file.patch
        );
    }

    #[test]
    fn diff_reports_the_new_path_and_patch_of_a_rename() {
        let auth = build_test_git_auth_config().expect("build test git config");
        let root = tempfile::tempdir().expect("create test directory");
        let repo = root.path().join("repo");
        run_git(
            &["init", "--", repo.to_str().expect("repo path")],
            None,
            &auth,
        )
        .expect("init repo");

        std::fs::write(repo.join("alt.md"), "a\nb\nc\n").expect("write fixture");
        commit(&repo, &auth, "init");
        run_git(&["mv", "alt.md", "neu.md"], Some(&repo), &auth).expect("rename fixture");
        commit(&repo, &auth, "rename");

        let diff = diff_from_repo(&repo, &auth, "HEAD~1..HEAD", None).expect("diff repo");
        let [file] = diff.files.as_slice() else {
            panic!(
                "expected exactly one changed file, got {}",
                diff.files.len()
            );
        };
        assert_eq!(file.path, "neu.md");
        assert!(
            !file.patch.is_empty(),
            "a rename must still render a patch for the new path"
        );
    }
}
