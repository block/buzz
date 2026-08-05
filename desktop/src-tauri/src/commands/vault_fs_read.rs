//! Read-only vault filesystem commands.

use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;
use tauri::State;

use crate::commands::vault_path::VaultState;

/// How deep the tree will descend. Linked folders can point anywhere, so this is
/// a backstop against a pathologically deep target as well as against cycles.
const MAX_TREE_DEPTH: usize = 32;

#[derive(Debug, Serialize)]
pub struct VaultEntry {
    name: String,
    path: String,
    is_directory: bool,
    /// `None` for files. Kept minimal on purpose — no stats, no mtime — because
    /// the whole tree crosses IPC in one payload.
    children: Option<Vec<VaultEntry>>,
}

#[derive(Debug, Serialize)]
pub struct VaultFileContent {
    path: String,
    /// `None` when the file could not be read; the batch skips rather than fails
    /// so one unreadable note cannot break indexing for the whole vault.
    content: Option<String>,
}

fn build_vault_tree(path: &Path) -> Vec<VaultEntry> {
    let mut visited: Vec<PathBuf> = Vec::new();
    build_vault_tree_inner(path, &mut visited, 0)
}

/// `visited` holds the canonical form of every directory on the current branch.
/// Because directory links are followed, `Vault/loop -> Vault` would otherwise
/// recurse until the stack ran out; re-entering a directory we are already
/// inside is the definition of a cycle, so we stop there.
fn build_vault_tree_inner(
    path: &Path,
    visited: &mut Vec<PathBuf>,
    depth: usize,
) -> Vec<VaultEntry> {
    let mut entries: Vec<VaultEntry> = Vec::new();

    if depth >= MAX_TREE_DEPTH {
        return entries;
    }

    let canonical = path.canonicalize().ok();
    if let Some(canonical) = &canonical {
        if visited.contains(canonical) {
            return entries;
        }
        visited.push(canonical.clone());
    }

    if let Ok(read_dir) = fs::read_dir(path) {
        let mut items: Vec<_> = read_dir.filter_map(|e| e.ok()).collect();
        items.sort_by(|a, b| {
            let a_is_dir = a.path().is_dir();
            let b_is_dir = b.path().is_dir();
            match (a_is_dir, b_is_dir) {
                (true, false) => std::cmp::Ordering::Less,
                (false, true) => std::cmp::Ordering::Greater,
                _ => a.file_name().cmp(&b.file_name()),
            }
        });

        for item in items {
            let item_path = item.path();
            let name = item.file_name().to_string_lossy().to_string();

            // Skip hidden files and folders (.git, .obsidian, .trash, ...).
            if name.starts_with('.') {
                continue;
            }

            let is_dir = item_path.is_dir();

            // Markdown and directories only. Onyx also admitted images/PDFs for
            // its embed viewers; v1 has no embeds, so a narrower tree is both
            // faster and less surface.
            if !is_dir && !is_markdown(&name) {
                continue;
            }

            let children = if is_dir {
                Some(build_vault_tree_inner(&item_path, visited, depth + 1))
            } else {
                None
            };

            entries.push(VaultEntry {
                name,
                path: item_path.to_string_lossy().to_string(),
                is_directory: is_dir,
                children,
            });
        }
    }

    if canonical.is_some() {
        visited.pop();
    }

    entries
}

fn is_markdown(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.ends_with(".md") || lower.ends_with(".markdown")
}

/// The whole vault tree. Takes no path — the root comes from [`VaultState`].
#[tauri::command]
pub async fn list_vault_files(state: State<'_, VaultState>) -> Result<Vec<VaultEntry>, String> {
    let root = state.require_root()?;
    tokio::task::spawn_blocking(move || {
        if !root.exists() {
            return Err("The vault folder no longer exists.".to_string());
        }
        Ok(build_vault_tree(&root))
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))?
}

#[tauri::command]
pub async fn read_vault_file(state: State<'_, VaultState>, path: String) -> Result<String, String> {
    let validated = state.validate(&path)?;
    tokio::task::spawn_blocking(move || {
        fs::read_to_string(validated.as_path()).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))?
}

/// Batch read, for building the note index and backlink corpus.
///
/// Onyx fanned this out as one IPC call per note (16 at a time); on a 2000-note
/// vault that is 2000 round trips. One call returns the lot.
#[tauri::command]
pub async fn read_vault_files(
    state: State<'_, VaultState>,
    paths: Vec<String>,
) -> Result<Vec<VaultFileContent>, String> {
    let validated: Vec<(String, PathBuf)> = paths
        .into_iter()
        .filter_map(|path| {
            state
                .validate(&path)
                .ok()
                .map(|v| (path, v.into_path_buf()))
        })
        .collect();

    tokio::task::spawn_blocking(move || {
        validated
            .into_iter()
            .map(|(path, resolved)| VaultFileContent {
                path,
                content: fs::read_to_string(&resolved).ok(),
            })
            .collect()
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))
}

#[tauri::command]
pub async fn vault_entry_exists(
    state: State<'_, VaultState>,
    path: String,
) -> Result<bool, String> {
    let validated = state.validate(&path)?;
    tokio::task::spawn_blocking(move || validated.as_path().exists())
        .await
        .map_err(|e| format!("spawn_blocking failed: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(tag: &str) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("buzz-vault-tree-{}-{}", std::process::id(), tag));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("Notes")).unwrap();
        fs::write(root.join("Notes/plain.md"), "# plain").unwrap();
        fs::write(root.join("top.md"), "# top").unwrap();
        root
    }

    fn names(entries: &[VaultEntry]) -> Vec<String> {
        entries.iter().map(|e| e.name.clone()).collect()
    }

    #[test]
    fn lists_markdown_and_directories_with_directories_first() {
        let root = fixture("basic");
        let tree = build_vault_tree(&root);
        assert_eq!(
            names(&tree),
            vec!["Notes".to_string(), "top.md".to_string()]
        );
        let notes = tree
            .iter()
            .find(|e| e.name == "Notes")
            .and_then(|e| e.children.as_ref())
            .expect("Notes must carry children");
        assert_eq!(names(notes), vec!["plain.md".to_string()]);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn skips_hidden_entries_and_non_markdown_files() {
        let root = fixture("filtered");
        fs::create_dir_all(root.join(".obsidian")).unwrap();
        fs::write(root.join(".obsidian/config.json"), "{}").unwrap();
        fs::write(root.join(".hidden.md"), "x").unwrap();
        fs::write(root.join("image.png"), "x").unwrap();

        let tree = build_vault_tree(&root);
        let listed = names(&tree);
        assert!(!listed.iter().any(|n| n.starts_with('.')));
        assert!(!listed.contains(&"image.png".to_string()));
        assert!(listed.contains(&"top.md".to_string()));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn accepts_the_markdown_long_extension() {
        let root = fixture("longext");
        fs::write(root.join("legacy.markdown"), "# legacy").unwrap();
        let tree = build_vault_tree(&root);
        assert!(names(&tree).contains(&"legacy.markdown".to_string()));
        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[test]
    fn terminates_on_a_link_cycle() {
        let root = fixture("cycle");
        std::os::unix::fs::symlink(&root, root.join("Notes/loop")).unwrap();
        // Would recurse forever without the visited set.
        let tree = build_vault_tree(&root);
        assert!(!tree.is_empty());
        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[test]
    fn terminates_on_an_ancestor_link() {
        // `parent-loop -> ..` points at the vault itself; `is_dir()` follows it.
        let root = fixture("ancestor");
        std::os::unix::fs::symlink("..", root.join("Notes/parent-loop")).unwrap();
        std::os::unix::fs::symlink("loop-b", root.join("Notes/loop-a")).unwrap();
        std::os::unix::fs::symlink("loop-a", root.join("Notes/loop-b")).unwrap();

        let tree = build_vault_tree(&root);
        assert!(!tree.is_empty(), "the walk must still return real entries");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn stops_at_max_depth() {
        let root = fixture("deep");
        let mut deep = root.clone();
        for i in 0..(MAX_TREE_DEPTH + 5) {
            deep = deep.join(format!("d{i}"));
        }
        fs::create_dir_all(&deep).unwrap();
        fs::write(deep.join("buried.md"), "# buried").unwrap();

        // The walk must terminate and stay bounded rather than recursing all the
        // way down; depth is counted from the vault root.
        let mut node = build_vault_tree(&root);
        let mut levels = 0;
        while let Some(child) = node
            .into_iter()
            .find(|e| e.is_directory)
            .and_then(|e| e.children)
        {
            levels += 1;
            node = child;
            if levels > MAX_TREE_DEPTH + 2 {
                break;
            }
        }
        assert!(
            levels <= MAX_TREE_DEPTH,
            "descended {levels} levels, past the {MAX_TREE_DEPTH} cap"
        );
        let _ = fs::remove_dir_all(&root);
    }
}
