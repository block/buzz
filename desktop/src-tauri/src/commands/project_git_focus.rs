use super::project_git_exec::{clean_target_commit, clean_target_ref};

pub(crate) struct RepositoryTarget {
    pub(crate) target_ref: Option<String>,
    pub(crate) target_commit: Option<String>,
    pub(crate) target_path: Option<String>,
}

pub(crate) fn clean_repository_target(
    target_ref: Option<String>,
    target_commit: Option<String>,
    target_path: Option<String>,
) -> Result<RepositoryTarget, String> {
    Ok(RepositoryTarget {
        target_ref: target_ref
            .map(|value| {
                clean_target_ref(Some(value))
                    .ok_or_else(|| "Invalid repository target ref.".to_string())
            })
            .transpose()?,
        target_commit: target_commit
            .map(|value| {
                clean_target_commit(Some(value))
                    .ok_or_else(|| "Invalid repository target commit.".to_string())
            })
            .transpose()?,
        target_path: target_path
            .map(|value| {
                clean_repository_focus_path(Some(value))
                    .ok_or_else(|| "Invalid repository target path.".to_string())
            })
            .transpose()?,
    })
}

const MAX_FOCUS_PATH_BYTES: usize = 4096;

pub(crate) fn clean_repository_focus_path(value: Option<String>) -> Option<String> {
    let value = value?;
    if value.is_empty()
        || value.len() > MAX_FOCUS_PATH_BYTES
        || value.starts_with(['/', '\\'])
        || value.ends_with('/')
        || value.contains('\\')
        || value.chars().any(char::is_control)
        || value
            .split('/')
            .any(|component| component.is_empty() || matches!(component, "." | ".."))
    {
        return None;
    }
    Some(value)
}

fn tree_line_path(line: &str) -> Option<&str> {
    line.split_once('\t').map(|(_, path)| path)
}

pub(crate) fn select_repository_tree_lines<'a>(
    output: &'a str,
    focus_path: Option<&str>,
    base_limit: usize,
    focus_limit: usize,
) -> Vec<&'a str> {
    let records: Vec<_> = output
        .split_terminator('\0')
        .filter(|record| !record.is_empty())
        .collect();
    let mut selected: Vec<_> = records.iter().take(base_limit).copied().collect();
    let Some(focus_path) = focus_path else {
        return selected;
    };
    let directory_prefix = format!("{focus_path}/");
    selected.extend(
        records
            .iter()
            .skip(base_limit)
            .filter(|line| {
                tree_line_path(line)
                    .is_some_and(|path| path == focus_path || path.starts_with(&directory_prefix))
            })
            .take(focus_limit)
            .copied(),
    );
    selected
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repository_target_normalizes_all_explicit_coordinates() {
        let target = clean_repository_target(
            Some("refs/heads/feature/repo-links".into()),
            Some("A".repeat(40)),
            Some("GUIDES/setup.md".into()),
        )
        .expect("valid target");
        assert_eq!(
            target.target_ref.as_deref(),
            Some("refs/heads/feature/repo-links")
        );
        assert_eq!(
            target.target_commit.as_deref(),
            Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
        );
        assert_eq!(target.target_path.as_deref(), Some("GUIDES/setup.md"));
    }

    #[test]
    fn repository_target_rejects_any_invalid_explicit_coordinate() {
        assert!(clean_repository_target(Some("refs/heads/main.lock".into()), None, None).is_err());
        assert!(clean_repository_target(None, Some("short".into()), None).is_err());
        assert!(clean_repository_target(None, None, Some("../README.md".into())).is_err());
    }

    #[test]
    fn focus_path_accepts_safe_repository_coordinates() {
        assert_eq!(
            clean_repository_focus_path(Some("GUIDES/setup/install.md".into())).as_deref(),
            Some("GUIDES/setup/install.md")
        );
    }

    #[test]
    fn focus_path_rejects_absolute_traversal_ambiguous_and_control_paths() {
        for value in [
            "",
            "/etc/passwd",
            "../README.md",
            "docs/../README.md",
            "docs//README.md",
            "docs/./README.md",
            "docs\\README.md",
            "docs/README.md/",
            "docs/\u{0}README.md",
        ] {
            assert_eq!(
                clean_repository_focus_path(Some(value.into())),
                None,
                "{value:?}"
            );
        }
    }

    #[test]
    fn focused_file_outside_base_cap_is_appended_once() {
        let output = [
            "100644 blob a 1\tA.txt",
            "100644 blob b 1\tB.txt",
            "100644 blob c 1\tGUIDES/RUNBOOK.md",
        ]
        .join("\0");
        assert_eq!(
            select_repository_tree_lines(&output, Some("GUIDES/RUNBOOK.md"), 2, 20),
            vec![
                "100644 blob a 1\tA.txt",
                "100644 blob b 1\tB.txt",
                "100644 blob c 1\tGUIDES/RUNBOOK.md",
            ]
        );
    }

    #[test]
    fn focused_directory_adds_descendants_without_partial_prefix_matches() {
        let output = [
            "100644 blob a 1\tA.txt",
            "100644 blob b 1\tGUIDES.md",
            "100644 blob c 1\tGUIDES/setup/install.md",
            "100644 blob d 1\tGUIDES/RUNBOOK.md",
        ]
        .join("\0");
        assert_eq!(
            select_repository_tree_lines(&output, Some("GUIDES"), 1, 20),
            vec![
                "100644 blob a 1\tA.txt",
                "100644 blob c 1\tGUIDES/setup/install.md",
                "100644 blob d 1\tGUIDES/RUNBOOK.md",
            ]
        );
    }

    #[test]
    fn nul_delimited_non_ascii_path_matches_without_git_c_quoting() {
        let output = ["100644 blob a 1\tA.txt", "100644 blob b 1\tcafé.txt"].join("\0");
        assert_eq!(
            select_repository_tree_lines(&output, Some("café.txt"), 1, 20),
            vec!["100644 blob a 1\tA.txt", "100644 blob b 1\tcafé.txt",]
        );
    }

    #[test]
    fn focused_entries_respect_their_own_cap() {
        let output = [
            "100644 blob a 1\tA.txt",
            "100644 blob b 1\tdocs/1.md",
            "100644 blob c 1\tdocs/2.md",
            "100644 blob d 1\tdocs/3.md",
        ]
        .join("\0");
        assert_eq!(
            select_repository_tree_lines(&output, Some("docs"), 1, 2),
            vec![
                "100644 blob a 1\tA.txt",
                "100644 blob b 1\tdocs/1.md",
                "100644 blob c 1\tdocs/2.md",
            ]
        );
    }
}
