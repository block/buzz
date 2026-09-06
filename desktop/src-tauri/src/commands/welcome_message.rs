use std::io::Write;

use serde::{Deserialize, Serialize};
use tauri::AppHandle;

use crate::managed_agents::{
    build_buzz_agent_provider_defaults, default_agent_workdir, known_acp_runtime,
    load_global_agent_config, redact_env_values_in, resolve_command,
    resolve_effective_harness_descriptor, ManagedAgentRecord, DEFAULT_ACP_COMMAND,
};

const WELCOME_WRITER_PROMPT: &str = r#"You write concise, friendly welcome messages for a community chat app.

Return only valid JSON with this exact shape:
{"text":"message text","inserts":[{"id":"short-kebab-id","type":"link|image|channel","title":"Visible label","url":"destination"}]}

Rules:
- Use {{member}} wherever the new member's name belongs.
- Reference an insert in text as {{insert:its-id}}.
- Every insert reference must have exactly one matching inserts entry.
- Add only elements that help fulfill the request.
- Use two short paragraphs at most.
- Do not use Markdown, code fences, or any keys outside the schema.

The admin described the welcome message they want below:
<request>
{request}
</request>"#;

const WELCOME_WRITER_SYSTEM_PROMPT: &str =
    "You are a focused writing assistant. Follow the user's output-format instructions exactly. \
     Respond with plain text only and do not call tools.";

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WelcomeMessageDraft {
    text: String,
    inserts: Vec<WelcomeMessageInsert>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct WelcomeMessageInsert {
    id: String,
    #[serde(rename = "type")]
    insert_type: String,
    title: String,
    url: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PromptOutput {
    message: String,
}

/// Generate a welcome message through the globally configured ACP runtime.
/// This is intentionally provider-agnostic and uses the same harness, model,
/// provider, environment, and credential resolution as managed agents.
#[tauri::command]
pub async fn generate_welcome_message(
    request: String,
    app: AppHandle,
) -> Result<WelcomeMessageDraft, String> {
    let request = request.trim();
    if request.is_empty() {
        return Err("Describe the welcome message you want.".to_string());
    }

    let global = load_global_agent_config(&app).unwrap_or_default();
    let record = global_writer_record(&global);

    let descriptor = resolve_effective_harness_descriptor(&record, &[], &global)
        .map_err(|error| crate::managed_agents::user_facing_harness_error(&error))?;
    let resolved_acp = resolve_command(DEFAULT_ACP_COMMAND).ok_or_else(|| {
        crate::managed_agents::missing_command_message(DEFAULT_ACP_COMMAND, "ACP harness command")
    })?;
    let resolved_agent = resolve_command(&descriptor.command)
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| descriptor.command.clone());
    let prompt = WELCOME_WRITER_PROMPT.replace("{request}", request);
    let env_for_redaction = descriptor.env.clone();

    let raw = tokio::task::spawn_blocking(move || {
        let mut command = std::process::Command::new(resolved_acp);
        if let Some(home) = default_agent_workdir() {
            command.current_dir(home);
        }
        if let Some(path) = crate::managed_agents::readiness::cli_probe::augmented_path() {
            command.env("PATH", path);
        }
        command
            .arg("prompt")
            .arg("--json")
            .env("BUZZ_ACP_AGENT_COMMAND", &resolved_agent)
            .env("BUZZ_ACP_AGENT_ARGS", descriptor.args.join(","))
            .env("BUZZ_ACP_SYSTEM_PROMPT", WELCOME_WRITER_SYSTEM_PROMPT);
        if let Some(meta) = known_acp_runtime(&descriptor.command) {
            for (key, value) in meta.default_env {
                if std::env::var(key).is_err() {
                    command.env(key, value);
                }
            }
        }
        build_buzz_agent_provider_defaults(&mut command);
        for (key, value) in &descriptor.env {
            command.env(key, value);
        }
        crate::managed_agents::configure_runtime_cli(
            &mut command,
            known_acp_runtime(&descriptor.command),
        );
        crate::util::configure_no_window(&mut command);
        let mut child = command
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|error| format!("could not start writing help: {error}"))?;
        child
            .stdin
            .take()
            .ok_or_else(|| "could not open writing help input".to_string())?
            .write_all(prompt.as_bytes())
            .map_err(|error| format!("could not send the writing request: {error}"))?;
        child
            .wait_with_output()
            .map_err(|error| format!("writing help did not finish: {error}"))
    })
    .await
    .map_err(|error| format!("writing help did not finish: {error}"))??;

    if !raw.status.success() {
        let stderr_raw = String::from_utf8_lossy(&raw.stderr);
        let user_message = welcome_provider_error(&stderr_raw);
        let stderr = redact_env_values_in(&stderr_raw, &env_for_redaction);
        tracing::warn!("welcome writing help error: {stderr}");
        return Err(user_message.to_string());
    }

    let output: PromptOutput = serde_json::from_slice(&raw.stdout)
        .map_err(|_| "Writing help returned an unreadable message. Try again.".to_string())?;
    parse_welcome_draft(&output.message)
}

fn welcome_provider_error(stderr: &str) -> &'static str {
    let normalized = stderr.to_ascii_lowercase();
    if normalized.contains("429 too many requests")
        || normalized.contains("rate_limit_error")
        || normalized.contains("rate limit")
    {
        return "Writing help has reached your provider’s usage limit. Try again shortly.";
    }
    if normalized.contains("401 unauthorized")
        || normalized.contains("authentication_error")
        || normalized.contains("invalid api key")
    {
        return "Writing help couldn’t connect with your provider credentials. Check your API key and try again.";
    }
    "Writing help couldn’t create a message. Try again."
}

fn global_writer_record(global: &crate::managed_agents::GlobalAgentConfig) -> ManagedAgentRecord {
    ManagedAgentRecord {
        pubkey: String::new(),
        name: "Welcome writer".to_string(),
        persona_id: None,
        private_key_nsec: String::new(),
        auth_tag: None,
        relay_url: String::new(),
        avatar_url: None,
        acp_command: DEFAULT_ACP_COMMAND.to_string(),
        agent_command: String::new(),
        agent_command_override: None,
        agent_args: Vec::new(),
        mcp_command: String::new(),
        turn_timeout_seconds: 0,
        idle_timeout_seconds: None,
        max_turn_duration_seconds: None,
        parallelism: 1,
        system_prompt: None,
        model: global.model.clone(),
        provider: global.provider.clone(),
        persona_source_version: None,
        env_vars: Default::default(),
        start_on_app_launch: false,
        auto_restart_on_config_change: false,
        runtime_pid: None,
        backend: Default::default(),
        backend_agent_id: None,
        provider_policy_pending: false,
        provider_binary_path: None,
        team_id: None,
        team_catalog_source: None,
        persona_team_dir: None,
        persona_name_in_team: None,
        created_at: String::new(),
        updated_at: String::new(),
        last_started_at: None,
        last_stopped_at: None,
        last_exit_code: None,
        last_error: None,
        last_error_code: None,
        respond_to: Default::default(),
        respond_to_allowlist: Vec::new(),
        display_name: None,
        slug: None,
        runtime: global.preferred_runtime.clone(),
        name_pool: Vec::new(),
        is_builtin: false,
        is_active: true,
        shared: false,
        source_team: None,
        source_team_persona_slug: None,
        catalog_source: None,
        definition_respond_to: None,
        definition_respond_to_allowlist: Vec::new(),
        definition_parallelism: None,
        relay_mesh: None,
        effort_level: None,
    }
}

fn parse_welcome_draft(message: &str) -> Result<WelcomeMessageDraft, String> {
    let trimmed = message.trim();
    let json = trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```"))
        .and_then(|value| value.strip_suffix("```"))
        .map(str::trim)
        .unwrap_or(trimmed);
    let draft: WelcomeMessageDraft = serde_json::from_str(json)
        .map_err(|_| "Writing help returned an unreadable message. Try again.".to_string())?;
    validate_welcome_draft(draft)
}

fn validate_welcome_draft(draft: WelcomeMessageDraft) -> Result<WelcomeMessageDraft, String> {
    if draft.text.trim().is_empty()
        || draft.text.len() > 4_000
        || draft.text.contains(['<', '>'])
        || draft.inserts.len() > 12
    {
        return Err("Writing help returned an invalid message. Try again.".to_string());
    }
    let mut ids = std::collections::HashSet::new();
    for insert in &draft.inserts {
        let valid_id = !insert.id.is_empty()
            && insert.id.len() <= 64
            && insert.id.chars().all(|character| {
                character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
            });
        let valid_type = matches!(insert.insert_type.as_str(), "link" | "image" | "channel");
        if !valid_id
            || !valid_type
            || insert.title.trim().is_empty()
            || insert.title.len() > 120
            || insert.url.len() > 2_048
            || !ids.insert(insert.id.as_str())
            || !draft
                .text
                .contains(&format!("{{{{insert:{}}}}}", insert.id))
        {
            return Err("Writing help returned an invalid message. Try again.".to_string());
        }
    }
    let token_pattern = regex::Regex::new(r"\{\{insert:([^}]+)\}\}")
        .map_err(|_| "Writing help returned an invalid message. Try again.".to_string())?;
    if token_pattern
        .captures_iter(&draft.text)
        .filter_map(|capture| capture.get(1))
        .any(|id| !ids.contains(id.as_str()))
    {
        return Err("Writing help returned an invalid message. Try again.".to_string());
    }
    Ok(draft)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_fenced_provider_output() {
        let draft = parse_welcome_draft(
            "```json\n{\"text\":\"Welcome, {{member}}! Visit {{insert:guide}}.\",\"inserts\":[{\"id\":\"guide\",\"type\":\"link\",\"title\":\"Guide\",\"url\":\"https://example.com\"}]}\n```",
        )
        .expect("valid draft");
        assert_eq!(draft.inserts[0].insert_type, "link");
    }

    #[test]
    fn rejects_unreferenced_insert() {
        let error = parse_welcome_draft(
            r#"{"text":"Welcome, {{member}}!","inserts":[{"id":"guide","type":"link","title":"Guide","url":"https://example.com"}]}"#,
        )
        .expect_err("insert must be referenced");
        assert!(error.contains("invalid message"));
    }

    #[test]
    fn rejects_missing_insert_and_html() {
        for message in [
            r#"{"text":"Visit {{insert:missing}}.","inserts":[]}"#,
            r#"{"text":"<img src=x>","inserts":[]}"#,
        ] {
            assert!(parse_welcome_draft(message).is_err());
        }
    }

    #[test]
    fn maps_provider_errors_without_exposing_details() {
        assert!(welcome_provider_error("429 Too Many Requests").contains("usage limit"));
        assert!(welcome_provider_error("authentication_error").contains("API key"));
        assert_eq!(
            welcome_provider_error("unexpected provider detail"),
            "Writing help couldn’t create a message. Try again."
        );
    }
}
