//! The skill index for the system prompt.
//!
//! Discovery, `load_skill`, argument templating, supporting-file loading and
//! the path-escape guard come from [`goose_agent::skills`]. The only thing left here
//! is rendering the index, and only because the piece of goose that renders it
//! (`ops_skills::skill_instructions`) is reachable through `SkillOperation`,
//! which is `pub(super)` — its concrete operations stay internal to goose
//! while the `Operation` trait is public.
//!
//! So buzz gets goose's skills the way an embedder can: the `skills` platform
//! extension supplies the tool, and this supplies the index that tells the
//! model the tool is worth calling. The wording matches goose's own so the two
//! do not drift into describing the same tool differently.

use goose_sdk_types::custom_requests::{SourceEntry, SourceType};

/// Provider-native definition for the in-process skill loader.
pub fn load_skill_tool() -> rmcp::model::Tool {
    let schema = serde_json::json!({
        "type": "object",
        "required": ["name"],
        "properties": {
            "name": { "type": "string", "description": "Skill name, or skill-name/path for a supporting file." },
            "args": { "type": "string", "description": "Optional arguments for a parameterized skill." }
        }
    });
    rmcp::model::Tool::new(
        "load_skill",
        "Load a skill's full content into your context so you can follow its instructions.",
        schema.as_object().cloned().unwrap_or_default(),
    )
}

/// Execute the in-process skill loader using Goose's public skill rendering.
pub fn load_skill(
    skills: &[SourceEntry],
    arguments: &serde_json::Value,
) -> rmcp::model::CallToolResult {
    let name = arguments
        .get("name")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    let args = arguments.get("args").and_then(serde_json::Value::as_str);
    if let Some(skill) = skills.iter().find(|skill| skill.name == name) {
        return match goose_agent::skills::loaded_skill_context_with_args(skill, args) {
            Ok(rendered) => {
                rmcp::model::CallToolResult::success(vec![rmcp::model::ContentBlock::text(
                    rendered,
                )])
            }
            Err(error) => {
                rmcp::model::CallToolResult::error(vec![rmcp::model::ContentBlock::text(format!(
                    "Failed to parse skill arguments: {error}"
                ))])
            }
        };
    }
    rmcp::model::CallToolResult::error(vec![rmcp::model::ContentBlock::text(format!(
        "Skill '{name}' not found."
    ))])
}

/// Render the index of available skills for the system prompt.
///
/// Empty for an empty slate, so the caller can skip the prompt extra entirely
/// rather than spending tokens on a header with nothing under it.
pub fn skill_index(skills: &[SourceEntry]) -> String {
    let mut entries: Vec<&SourceEntry> = skills
        .iter()
        .filter(|skill| {
            matches!(
                skill.source_type,
                SourceType::Skill | SourceType::BuiltinSkill
            )
        })
        .collect();
    if entries.is_empty() {
        return String::new();
    }
    // Sorted on the same key as goose's own renderer, so the section is stable
    // between rounds. An index that reorders itself would invalidate the
    // provider's prompt cache every turn for no benefit.
    entries.sort_by(|a, b| (&a.name, &a.path).cmp(&(&b.name, &b.path)));

    let mut out = String::from(
        "# Skills\n\nYou have these skills at your disposal. Load one when it can help \
         with the task or when the user asks for it:",
    );
    for skill in entries {
        out.push_str(&format!("\n- {}: {}", skill.name, skill.description));
    }
    // Unprefixed on purpose. goose registers `skills` with
    // `unprefixed_tools: true` (`platform_extensions/mod.rs:217`), so the
    // model's tool list carries `load_skill`, not `skills__load_skill`.
    // Naming it the other way would advertise a tool that is not there.
    out.push_str("\n\nLoad a skill with the `load_skill` tool before using it.\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn skill(name: &str, source_type: SourceType) -> SourceEntry {
        SourceEntry {
            source_type,
            name: name.to_string(),
            description: format!("does {name}"),
            content: String::new(),
            path: format!("/tmp/{name}"),
            global: false,
            writable: false,
            supporting_files: Vec::new(),
            properties: Default::default(),
        }
    }

    #[test]
    fn an_empty_slate_renders_nothing() {
        assert!(skill_index(&[]).is_empty());
    }

    #[test]
    fn builtin_and_filesystem_skills_both_appear() {
        let index = skill_index(&[
            skill("widget", SourceType::Skill),
            skill("doc-guide", SourceType::BuiltinSkill),
        ]);
        assert!(index.contains("- widget: does widget"));
        assert!(index.contains("- doc-guide: does doc-guide"));
    }

    #[test]
    fn non_skill_sources_are_left_out() {
        // `discover_skills` returns `SourceEntry`, which also models projects.
        // Listing a project as a skill would advertise a `load_skill` call
        // that cannot succeed.
        let index = skill_index(&[skill("some-project", SourceType::Project)]);
        assert!(index.is_empty(), "unexpected index: {index:?}");
    }

    #[test]
    fn the_tool_is_named_the_way_the_model_sees_it() {
        // goose exposes the skills extension unprefixed; `skills__load_skill`
        // is not in the tool list and would be a dead reference.
        let index = skill_index(&[skill("widget", SourceType::Skill)]);
        assert!(index.contains("`load_skill`"));
        assert!(!index.contains("skills__load_skill"));
    }

    #[test]
    fn the_order_is_stable() {
        let unsorted = [
            skill("zebra", SourceType::Skill),
            skill("alpha", SourceType::Skill),
        ];
        let index = skill_index(&unsorted);
        let alpha = index.find("alpha").expect("alpha listed");
        let zebra = index.find("zebra").expect("zebra listed");
        assert!(alpha < zebra, "index must not reorder between rounds");
    }
}
