pub(super) fn child_rust_log_filter() -> String {
    match std::env::var("RUST_LOG") {
        Ok(existing) if existing.contains("buzz_acp") => existing,
        Ok(existing) if !existing.trim().is_empty() => format!("{existing},buzz_acp=info"),
        _ => "buzz_acp=info".to_string(),
    }
}

/// Returns catalog-owned model/provider environment values for an agent process.
pub(crate) fn runtime_metadata_env_vars<'a>(
    model_env_var: Option<&'a str>,
    provider_env_var: Option<&'a str>,
    provider_locked: bool,
    locked_provider_id: Option<&'a str>,
    effective_model: Option<&'a str>,
    effective_provider: Option<&'a str>,
) -> Vec<(&'a str, &'a str)> {
    let mut vars = Vec::new();
    if let (Some(env_key), Some(model)) = (model_env_var, effective_model) {
        vars.push((env_key, model));
    }
    if provider_locked {
        if let (Some(env_key), Some(provider)) = (provider_env_var, locked_provider_id) {
            vars.push((env_key, provider));
        }
    } else if let (Some(env_key), Some(provider)) = (provider_env_var, effective_provider) {
        vars.push((env_key, provider));
    }
    vars
}
