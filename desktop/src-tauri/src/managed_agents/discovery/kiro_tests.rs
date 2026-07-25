use super::normalize_agent_args;

#[test]
fn normalizes_kiro_cli_args_to_acp() {
    assert_eq!(normalize_agent_args("kiro-cli", Vec::new()), vec!["acp"]);
    assert_eq!(
        normalize_agent_args("/usr/local/bin/kiro-cli", vec!["".into()]),
        vec!["acp"]
    );
    assert_eq!(
        normalize_agent_args("Kiro_CLI.EXE", Vec::new()),
        vec!["acp"]
    );
}
