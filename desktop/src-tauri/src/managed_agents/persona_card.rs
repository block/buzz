/// Find `.plugin/plugin.json` in a directory. Returns the parent of `.plugin/`.
/// Checks root and root/* only, matching supported team-pack layout.
pub fn find_plugin_json(root: &std::path::Path) -> Option<std::path::PathBuf> {
    if root.join(".plugin").join("plugin.json").exists() {
        return Some(root.to_path_buf());
    }

    let entries = std::fs::read_dir(root).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() && path.join(".plugin").join("plugin.json").exists() {
            return Some(path);
        }
    }
    None
}
