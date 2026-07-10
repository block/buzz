//! Pack-backed persona write-back: resolve the source `.persona.md` via the
//! pack manifest and rewrite frontmatter fields. Extracted from the parent
//! module to keep it under the desktop file-size cap. Pure relocation.

use super::*;

/// Find the team whose `team_persona_key` equals `source_team`. This matches
/// the same key that `sync_team_from_dir` uses, covering both modern teams
/// (where `team.id` equals the manifest directory name) and legacy/backfilled
/// teams (where `team.id` is a UUID and the manifest id is `source_dir.file_name()`).
fn find_team_for_persona_source<'a>(
    teams: &'a [TeamRecord],
    source_team: &str,
) -> Option<&'a TeamRecord> {
    teams
        .iter()
        .find(|t| t.id == source_team || team_persona_key(t) == source_team)
}

/// Write updated frontmatter fields back to the source `.persona.md` file for
/// pack-backed personas (`source_team` is set). Non-fatal: any miss (no
/// `source_dir`, missing file, pack load failure, parse or write error) is
/// logged and swallowed so that the in-app edit — already persisted to
/// `personas.json` — always lands.
///
/// Returns `Some(warning)` when the write-back failed (non-fatal), `None` on
/// success. The warning is forwarded to the `update_persona` command result so
/// the frontend can surface a "pack file diverged" indicator instead of
/// silently drifting.
///
/// Only the four fields that the UI can set and that live in frontmatter are
/// rewritten: `display_name`, `runtime`, `avatar`, and `model` (the combined
/// `"provider:model"` string used by the pack format). The markdown body is
/// preserved byte-for-byte because `PersonaRecord.system_prompt` is the
/// _composed_ prompt (body + pack instructions appended by `compose_prompt`)
/// and writing it back to the file would cause the instructions to be
/// double-appended on the next launch sync.
///
/// The source file path is derived from the pack manifest via
/// `buzz_persona_pkg::pack::load_pack` — the same resolution the launch sync
/// uses — rather than reconstructed by convention. This ensures write-back
/// targets the correct file regardless of where the manifest places the
/// `.persona.md` (e.g. `personas/` vs `agents/`, nested paths, or filenames
/// that differ from the persona `name:` field).
///
/// The team is located via `find_team_for_persona_source`, which matches the
/// same key as `sync_team_from_dir` (`team_persona_key`). This handles both
/// modern teams (where `team.id` equals the manifest id) and legacy/backfilled
/// teams (where `team.id` is a UUID and the manifest id lives in `source_dir`).
pub(super) fn write_back_persona_md(app: &AppHandle, persona: &PersonaRecord) -> Option<String> {
    // Only pack-backed personas have a source file to write back to.
    persona.source_team.as_ref()?;

    let result = load_teams(app).and_then(|teams| try_write_back_persona_md(&teams, persona));

    match result {
        Ok(()) => None,
        Err(e) => {
            eprintln!("buzz-desktop: persona-writeback: {e}");
            Some(e)
        }
    }
}

/// Inner logic for write-back, extracted for testability. Takes resolved teams
/// rather than an `AppHandle` so tests can exercise the full path → symlink →
/// pack → rewrite flow without a running Tauri application.
///
/// Returns `Err` if write-back fails for any reason (non-fatal at the call
/// site); `Ok(())` if the persona has no pack source or the file was updated
/// (or was already current).
fn try_write_back_persona_md(teams: &[TeamRecord], persona: &PersonaRecord) -> Result<(), String> {
    let Some(source_team_id) = &persona.source_team else {
        return Ok(()); // non-pack persona — nothing to write back
    };
    let Some(slug) = &persona.source_team_persona_slug else {
        return Err(format!(
            "persona {} has source_team but no slug; cannot write back",
            persona.id
        ));
    };

    let team = find_team_for_persona_source(teams, source_team_id)
        .ok_or_else(|| format!("team {source_team_id} not found"))?;
    let source_dir = team
        .source_dir
        .as_ref()
        .ok_or_else(|| "team has no source_dir (JSON-only team)".to_string())?;

    // Resolve the actual source file via the pack manifest, matching the
    // same path the launch sync reads. `LoadedPersona.source_path` is the
    // absolute path set by `safe_resolve` against the pack root, so it is
    // correct regardless of the manifest layout.
    let pack = buzz_persona_pkg::pack::load_pack(source_dir)
        .map_err(|e| format!("load_pack {}: {e}", source_dir.display()))?;
    let loaded = pack
        .personas
        .iter()
        .find(|p| p.name == *slug)
        .ok_or_else(|| {
            format!(
                "persona '{slug}' not found in pack at {}",
                source_dir.display()
            )
        })?;
    let path = &loaded.source_path;

    let content =
        std::fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))?;

    let updated = rewrite_persona_md(
        &content,
        persona,
        &loaded.prompt,
        pack.pack_instructions.as_deref(),
    )?;
    if updated == content {
        return Ok(());
    }
    std::fs::write(path, &updated).map_err(|e| format!("write {}: {e}", path.display()))?;
    Ok(())
}

/// Rewrite a `.persona.md` file with updated frontmatter fields and, when safe,
/// an updated body (system prompt). Returns the full rewritten file content, or
/// the original unchanged when the result would be byte-identical.
///
/// **Frontmatter fields rewritten:** `display_name`, `runtime`, `avatar`, and
/// `model` (joined `"provider:model"` per the pack format). All other keys and
/// their order are preserved.
///
/// **Body (system prompt) write-back:**
/// The `persona.system_prompt` field holds the *composed* prompt:
/// `compose_prompt(raw_body, pack_instructions)`. To recover the raw body we
/// reverse `compose_prompt`:
///
/// - If `pack_instructions` is absent or blank: new body = `system_prompt`
///   verbatim (no suffix to strip).
/// - If `pack_instructions` is present and non-blank: the composed prompt ends
///   with `"\n\n---\n# Team Instructions\n{instructions}"`. If
///   `system_prompt` ends with that exact suffix, strip it to get the new raw
///   body. **Safety guard**: if the suffix is absent (user edited inside the
///   Team Instructions block, or instructions drifted), we cannot safely
///   recover the raw body — preserve the existing body and log a skip. This
///   prevents a corrupted file or double-appended instructions.
/// - If `system_prompt` equals `compose_prompt(current_raw_body, instructions)`
///   exactly (user did not edit the prompt), the body is preserved
///   byte-for-byte (no-op for the body section).
fn rewrite_persona_md(
    content: &str,
    persona: &PersonaRecord,
    current_raw_body: &str,
    pack_instructions: Option<&str>,
) -> Result<String, String> {
    let (frontmatter, existing_body) = buzz_persona_pkg::persona::split_frontmatter(content)
        .map_err(|e| format!("split_frontmatter: {e:?}"))?;

    let mut value = serde_yaml::from_str::<serde_yaml::Value>(frontmatter)
        .map_err(|e| format!("yaml parse: {e}"))?;
    let mapping = value
        .as_mapping_mut()
        .ok_or("frontmatter is not a YAML mapping")?;

    // display_name
    mapping.insert(
        serde_yaml::Value::String("display_name".to_string()),
        serde_yaml::Value::String(persona.display_name.clone()),
    );

    // runtime: set when Some, remove when None
    let runtime_key = serde_yaml::Value::String("runtime".to_string());
    match &persona.runtime {
        Some(rt) if !rt.is_empty() => {
            mapping.insert(runtime_key, serde_yaml::Value::String(rt.clone()));
        }
        _ => {
            mapping.remove(&runtime_key);
        }
    }

    // avatar: set when Some, remove when None
    let avatar_key = serde_yaml::Value::String("avatar".to_string());
    match &persona.avatar_url {
        Some(av) if !av.is_empty() => {
            mapping.insert(avatar_key, serde_yaml::Value::String(av.clone()));
        }
        _ => {
            mapping.remove(&avatar_key);
        }
    }

    // model: joined "provider:model" or bare "model"; remove when both absent
    let model_key = serde_yaml::Value::String("model".to_string());
    match (&persona.provider, &persona.model) {
        (Some(prov), Some(mdl)) if !prov.is_empty() && !mdl.is_empty() => {
            mapping.insert(
                model_key,
                serde_yaml::Value::String(format!("{prov}:{mdl}")),
            );
        }
        (_, Some(mdl)) if !mdl.is_empty() => {
            mapping.insert(model_key, serde_yaml::Value::String(mdl.clone()));
        }
        _ => {
            mapping.remove(&model_key);
        }
    }

    let updated_frontmatter =
        serde_yaml::to_string(&value).map_err(|e| format!("yaml serialize: {e}"))?;

    // Determine the body to write back.
    // `compose_prompt` is: body + "\n\n---\n# Team Instructions\n{instructions}"
    // when instructions is non-blank, or body verbatim when absent/blank.
    let effective_instructions = pack_instructions.filter(|s| !s.trim().is_empty());
    let expected_composed = match effective_instructions {
        Some(instr) => format!("{current_raw_body}\n\n---\n# Team Instructions\n{instr}"),
        None => current_raw_body.to_owned(),
    };

    let new_body: &str = if persona.system_prompt == expected_composed {
        // User did not edit the prompt — keep the existing body byte-for-byte.
        existing_body
    } else {
        // User edited the prompt. Recover the raw body by reversing compose_prompt.
        match effective_instructions {
            None => {
                // No pack instructions: composed == raw, write verbatim.
                &persona.system_prompt
            }
            Some(instr) => {
                let suffix = format!("\n\n---\n# Team Instructions\n{instr}");
                if let Some(raw) = persona.system_prompt.strip_suffix(suffix.as_str()) {
                    raw
                } else {
                    // Safety guard: suffix absent — cannot safely recover raw body.
                    // Preserve the existing body to avoid corruption or double-append.
                    eprintln!(
                        "buzz-desktop: persona-writeback: \
                         system_prompt does not end with expected Team Instructions suffix; \
                         preserving existing body to avoid corruption"
                    );
                    existing_body
                }
            }
        }
    };

    Ok(format!("---\n{updated_frontmatter}---\n{new_body}"))
}

#[cfg(test)]
mod tests;
