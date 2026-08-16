use std::path::PathBuf;

pub(super) fn ordered_command_search_dirs(
    workspace_dirs: Vec<PathBuf>,
    current_dirs: Vec<PathBuf>,
    exe_parent: Option<PathBuf>,
    prefer_exe_parent: bool,
) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if prefer_exe_parent {
        dirs.extend(exe_parent.clone());
    }
    dirs.extend(workspace_dirs);
    dirs.extend(current_dirs);
    if !prefer_exe_parent {
        dirs.extend(exe_parent);
    }

    dirs.into_iter().fold(Vec::new(), |mut unique, dir| {
        if !unique.contains(&dir) {
            unique.push(dir);
        }
        unique
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_search_prefers_bundled_executable() {
        let workspace = vec![PathBuf::from("/source/target/release")];
        let current = vec![PathBuf::from("/cwd/target/release")];
        let bundled = PathBuf::from("/Applications/Buzz.app/Contents/MacOS");

        let dirs = ordered_command_search_dirs(workspace, current, Some(bundled.clone()), true);

        assert_eq!(dirs.first(), Some(&bundled));
    }

    #[test]
    fn debug_search_keeps_bundled_executable_last_and_deduplicates() {
        let workspace = vec![PathBuf::from("/source/target/debug")];
        let current = workspace.clone();
        let bundled = PathBuf::from("/Applications/Buzz.app/Contents/MacOS");

        let dirs =
            ordered_command_search_dirs(workspace.clone(), current, Some(bundled.clone()), false);

        assert_eq!(dirs, vec![workspace[0].clone(), bundled]);
    }
}
