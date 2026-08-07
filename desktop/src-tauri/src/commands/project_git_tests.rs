use super::*;

#[test]
fn parse_ls_tree_keeps_paths_after_eager_preview_limit() {
    let hidden_entries = (0..MAX_EAGER_FILE_PREVIEWS)
        .map(|index| {
            format!(
                "100644 blob {} 1\t.agents/generated-{index:03}.txt",
                "a".repeat(40)
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let output = format!(
        "{hidden_entries}\n100644 blob {} 12\tsrc/application.rs",
        "b".repeat(40)
    );

    let files = parse_ls_tree(
        std::path::Path::new("/path/does/not/exist"),
        &output,
        &std::collections::HashMap::new(),
    );

    assert_eq!(files.len(), MAX_EAGER_FILE_PREVIEWS + 1);
    assert_eq!(
        files.last().map(|file| file.path.as_str()),
        Some("src/application.rs")
    );
}

#[test]
fn parse_ls_tree_counts_only_blobs_toward_eager_preview_limit() {
    let repo_dir = tempfile::tempdir().expect("create temporary repository");
    std::fs::write(repo_dir.path().join("application.rs"), "fn main() {}")
        .expect("write preview file");
    let non_blob_entries = (0..MAX_EAGER_FILE_PREVIEWS)
        .map(|index| {
            format!(
                "160000 commit {} -\tvendor/dependency-{index:03}",
                "a".repeat(40)
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let output = format!(
        "{non_blob_entries}\n100644 blob {} 12\tapplication.rs",
        "b".repeat(40)
    );

    let files = parse_ls_tree(repo_dir.path(), &output, &std::collections::HashMap::new());

    assert_eq!(
        files
            .last()
            .and_then(|file| file.preview_content.as_deref()),
        Some("fn main() {}")
    );
}

#[test]
fn parse_worktree_files_counts_only_files_toward_eager_preview_limit() {
    let repo_dir = tempfile::tempdir().expect("create temporary repository");
    std::fs::create_dir(repo_dir.path().join("directory")).expect("create directory");
    let paths = (0..MAX_EAGER_FILE_PREVIEWS)
        .map(|index| {
            let path = format!("file-{index:03}.txt");
            std::fs::write(repo_dir.path().join(&path), "preview").expect("write preview file");
            path
        })
        .collect::<Vec<_>>();
    let output = std::iter::once("directory")
        .chain(paths.iter().map(String::as_str))
        .collect::<Vec<_>>()
        .join("\0");

    let files = parse_worktree_files(repo_dir.path(), &output, &std::collections::HashMap::new());

    assert_eq!(files.len(), MAX_EAGER_FILE_PREVIEWS);
    assert!(files.iter().all(|file| file.preview_content.is_some()));
}
