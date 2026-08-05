pub(super) fn credential_env_key(provider: Option<&str>) -> Option<&'static str> {
    match provider {
        Some("anthropic") => Some("ANTHROPIC_API_KEY"),
        Some("openai") => Some("OPENAI_COMPAT_API_KEY"),
        Some("databricks") | Some("databricks_v2") | Some("databricks-v2") => {
            Some("DATABRICKS_HOST")
        }
        Some("openrouter") => Some("OPENROUTER_API_KEY"),
        Some("venice") => Some("VENICE_API_KEY"),
        _ => None,
    }
}
