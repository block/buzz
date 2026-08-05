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
