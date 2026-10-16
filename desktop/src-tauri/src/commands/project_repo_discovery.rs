//! Bounded discovery of local Git working trees.

use std::collections::HashSet;

/// Maximum number of directory levels below a configured repositories root
/// that local project discovery will inspect.
const MAX_LOCAL_REPO_DISCOVERY_DEPTH: usize = 4;

/// Discover Git working trees below `repos_root` without following symlinks
/// outside the configured root or recursing indefinitely through cycles.
///
/// Repository roots are terminal: once a `.git` entry is found, discovery does
/// not descend into that checkout and accidentally surface its submodules.
pub(crate) fn discover_local_repo_dirs(
    repos_root: &std::path::Path,
) -> Result<Vec<std::path::PathBuf>, String> {
    let repos_root = repos_root
        .canonicalize()
        .map_err(|error| format!("reposDir is not accessible: {error}"))?;
    if !repos_root.is_dir() {
        return Err("reposDir is not a directory".to_string());
    }

    let mut repos = Vec::new();
    let mut visited = HashSet::from([repos_root.clone()]);
    let mut pending = vec![(repos_root.clone(), 0usize)];

    while let Some((directory, depth)) = pending.pop() {
        let entries = match std::fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(error) if depth == 0 => return Err(format!("read reposDir: {error}")),
            Err(_) => continue,
        };

        for entry in entries.filter_map(Result::ok) {
            let Some(file_type) = entry.file_type().ok() else {
                continue;
            };
            if !file_type.is_dir() && !file_type.is_symlink() {
                continue;
            }

            let Ok(path) = entry.path().canonicalize() else {
                continue;
            };
            if !path.starts_with(&repos_root) || !path.is_dir() || !visited.insert(path.clone()) {
                continue;
            }

            if path.join(".git").exists() {
                repos.push(path);
                continue;
            }

            let child_depth = depth + 1;
            if child_depth < MAX_LOCAL_REPO_DISCOVERY_DEPTH {
                pending.push((path, child_depth));
            }
        }
    }

    repos.sort();
    Ok(repos)
}

#[cfg(test)]
mod tests {
    use super::{discover_local_repo_dirs, MAX_LOCAL_REPO_DISCOVERY_DEPTH};
    use std::path::Path;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_TEST_DIR: AtomicU64 = AtomicU64::new(0);

    struct TestDir(std::path::PathBuf);

    impl TestDir {
        fn new() -> Self {
            let sequence = NEXT_TEST_DIR.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "buzz-project-repo-discovery-{}-{sequence}",
                std::process::id()
            ));
            std::fs::create_dir_all(&path).expect("create temp directory");
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).expect("remove temp directory");
        }
    }

    fn create_repo(path: &Path) {
        std::fs::create_dir_all(path.join(".git")).expect("create test repository");
    }

    #[test]
    fn bounds_nested_repository_discovery_depth() {
        let temp = TestDir::new();
        let repos_root = temp.path().join("code");
        let within_bound = repos_root
            .join("one")
            .join("two")
            .join("three")
            .join("repo-within-bound");
        let beyond_bound = repos_root
            .join("one")
            .join("two")
            .join("three")
            .join("four")
            .join("repo-beyond-bound");
        create_repo(&within_bound);
        create_repo(&beyond_bound);

        let discovered = discover_local_repo_dirs(&repos_root).expect("discover repositories");

        assert!(discovered.contains(&within_bound.canonicalize().unwrap()));
        assert!(!discovered.contains(&beyond_bound.canonicalize().unwrap()));
        assert_eq!(MAX_LOCAL_REPO_DISCOVERY_DEPTH, 4);
    }

    #[cfg(unix)]
    #[test]
    fn ignores_symlinks_that_escape_the_configured_root() {
        use std::os::unix::fs::symlink;

        let temp = TestDir::new();
        let repos_root = temp.path().join("code");
        std::fs::create_dir_all(&repos_root).unwrap();
        let external_repo = temp.path().join("external-repo");
        create_repo(&external_repo);
        symlink(&external_repo, repos_root.join("escaped")).expect("create test symlink");

        let discovered = discover_local_repo_dirs(&repos_root).expect("discover repositories");

        assert!(discovered.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn visits_in_root_symlink_targets_only_once() {
        use std::os::unix::fs::symlink;

        let temp = TestDir::new();
        let repos_root = temp.path().join("code");
        let nested_group = repos_root.join("client");
        let nested_repo = nested_group.join("web-app");
        create_repo(&nested_repo);
        symlink(&nested_group, repos_root.join("client-alias")).expect("create test symlink");

        let discovered = discover_local_repo_dirs(&repos_root).expect("discover repositories");

        assert_eq!(discovered, vec![nested_repo.canonicalize().unwrap()]);
    }
}
