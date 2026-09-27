use super::{command_search_dirs, common_binary_paths, find_nvm_default_bin, login_shell_path};
use std::path::PathBuf;

pub(crate) fn merge_command_discovery_dirs(
    sources: impl IntoIterator<Item = Vec<PathBuf>>,
) -> Vec<PathBuf> {
    sources
        .into_iter()
        .flatten()
        .fold(Vec::new(), |mut unique, dir| {
            if !unique.contains(&dir) {
                unique.push(dir);
            }
            unique
        })
}

pub(crate) fn command_discovery_dirs() -> Vec<PathBuf> {
    let path_dirs = std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).collect())
        .unwrap_or_default();
    let login_shell_dirs = login_shell_path()
        .map(|path| std::env::split_paths(&path).collect())
        .unwrap_or_default();
    let nvm_dirs = dirs::home_dir()
        .and_then(|home| find_nvm_default_bin(&home))
        .into_iter()
        .collect();

    merge_command_discovery_dirs([
        command_search_dirs(),
        path_dirs,
        common_binary_paths().to_vec(),
        login_shell_dirs,
        nvm_dirs,
    ])
}
