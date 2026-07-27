use std::io::Write as _;

use tempfile::NamedTempFile;

fn write_log(content: &str) -> NamedTempFile {
    let mut file = NamedTempFile::new().expect("temp log");
    file.write_all(content.as_bytes()).expect("write log");
    file
}

#[test]
fn meaningful_agent_error_from_log_promotes_wrapped_llm_auth() {
    let file =
        write_log("noise\nAgent reported error (code -32001): llm auth: 401 unauthorized: ...\n");
    let result = super::super::meaningful_agent_error_from_log(file.path()).unwrap();
    assert!(result.message.contains("llm auth"));
    assert_eq!(result.code, Some(-32001));
}

#[test]
fn meaningful_agent_error_from_log_promotes_unwrapped_llm_auth() {
    let file = write_log("noise\nllm auth: denied\n");
    let result = super::super::meaningful_agent_error_from_log(file.path()).unwrap();
    assert_eq!(result.message, "Agent reported error: llm auth: denied");
    assert_eq!(result.code, Some(-32001));
}

#[test]
fn meaningful_agent_error_from_log_promotes_bare_model_not_found() {
    let file = write_log("noise\nllm model not found: (some-model) 404\n");
    let result = super::super::meaningful_agent_error_from_log(file.path()).unwrap();
    assert_eq!(
        result.message,
        "Agent reported error: llm model not found: (some-model) 404"
    );
    assert_eq!(result.code, Some(-32002));
}

#[test]
fn meaningful_agent_error_from_log_promotes_legacy_format() {
    let file = write_log("noise\nAgent reported error: llm: 500 internal\n");
    let result = super::super::meaningful_agent_error_from_log(file.path()).unwrap();
    assert_eq!(result.message, "Agent reported error: llm: 500 internal");
    assert_eq!(result.code, None);
}

#[test]
fn meaningful_agent_error_from_log_does_not_promote_midline_auth_text() {
    let file = write_log("noise before llm auth: denied\n");
    assert!(super::super::meaningful_agent_error_from_log(file.path()).is_none());
}

#[test]
fn strips_ansi_from_typical_tracing_line() {
    let input = "\x1b[2m2026-05-27T15:16:32\x1b[0m \x1b[32m INFO\x1b[0m \x1b[2mbuzz_acp\x1b[0m\x1b[2m:\x1b[0m starting";
    assert_eq!(
        strip_ansi_escapes::strip_str(input),
        "2026-05-27T15:16:32  INFO buzz_acp: starting"
    );
}
