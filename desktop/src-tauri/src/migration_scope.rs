pub(crate) use backfill::backfill_standalone_agents_in_dir;
pub(crate) use detach::detach_directory_backed_teams_in_dir;
pub(crate) use fold::fold_personas_in_dir;
pub(crate) use materialize::materialize_runtimes_in_file;
pub(crate) use team_suffix::strip_baked_team_instructions_in_dir;

/// Rename `provider` → `runtime` in a scoped `definitions_dir/personas.json`.
///
/// Runs BEFORE `fold_personas_in_dir` so the fold reads the correct `runtime`
/// field. Idempotent: records that already have `runtime` are unchanged.
/// Returns `Ok(())` when there is no `personas.json` to migrate.
/// Returns `Err` when the file exists but the patch fails.
pub(crate) fn migrate_persona_provider_to_runtime_at(
    definitions_dir: &std::path::Path,
) -> Result<(), String> {
    let path = definitions_dir.join("personas.json");
    if path.exists() {
        rename_provider_to_runtime_in_personas(&path)?;
    }
    Ok(())
}

/// Reconcile `mcp_command` values in a scoped `definitions_dir`.
pub(crate) fn reconcile_provider_mcp_commands_at(definitions_dir: &std::path::Path) -> Result<(), String> {
    let path = definitions_dir.join("managed-agents.json");
    if path.exists() {
        reconcile_mcp_commands_in_file(&path)?;
    }
    Ok(())
}

/// Reconcile Databricks V1 → V2 provider entries in a scoped `definitions_dir`.
pub(crate) fn reconcile_databricks_v1_to_v2_at(definitions_dir: &std::path::Path) -> Result<(), String> {
    use crate::managed_agents::baked_build_env;
    let rewrite_v1_provider = baked_build_env()
        .get("BUZZ_AGENT_PROVIDER")
        .map(|v| v == "databricks_v2")
        .unwrap_or(false);
    let path = definitions_dir.join("managed-agents.json");
    if path.exists() {
        reconcile_databricks_v1_to_v2_in_file(&path, rewrite_v1_provider)?;
    }
    Ok(())
}

/// Refresh legacy built-in agent avatars in a scoped `definitions_dir`.
pub(crate) fn refresh_builtin_agent_avatars_at(definitions_dir: &std::path::Path) -> Result<(), String> {
    let path = definitions_dir.join("managed-agents.json");
    if path.exists() {
        refresh_builtin_agent_avatars_in_file(&path, LEGACY_BUILTIN_AVATARS, &crate::util::now_iso())?;
    }
    Ok(())
}

/// Reconcile legacy command names in a scoped `definitions_dir`.
pub(crate) fn reconcile_legacy_command_names_at(definitions_dir: &std::path::Path) -> Result<(), String> {
    let path = definitions_dir.join("managed-agents.json");
    if path.exists() {
        reconcile_legacy_command_names_in_file(&path)?;
    }
    Ok(())
}

/// Materialize per-record runtimes in a scoped `definitions_dir`.
pub(crate) fn materialize_agent_runtimes_at(definitions_dir: &std::path::Path) -> Result<(), String> {
    let path = definitions_dir.join("managed-agents.json");
    if path.exists() {
        materialize_runtimes_in_file(&path)?;
    }
    Ok(())
}
