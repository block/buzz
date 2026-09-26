//! Built-in tools that run in-process, bypassing MCP.
//!
//! Currently: `load_skill` — reads a skill's full SKILL.md body from disk
//! and returns it so the agent can load skill content on demand rather than
//! having every skill inlined into the system prompt at session start.

use serde_json::{json, Value};

use crate::hints::{strip_frontmatter, SkillEntry, MAX_SKILL_BODY_BYTES};
use crate::mcp::truncate_at_boundary;
use crate::types::{ToolDef, ToolResult, ToolResultContent};

pub const LOAD_SKILL_TOOL: &str = "load_skill";

/// Return the `ToolDef` for `load_skill` to include in the LLM tool list.
pub fn load_skill_def() -> ToolDef {
    ToolDef {
        name: LOAD_SKILL_TOOL.to_owned(),
        description: "Load the full content of a skill by name. \
            Call this before using a skill — the system prompt lists skill names \
            and descriptions only; the full instructions are loaded on demand. \
            To load a supporting file within a skill, use the form \
            \"skill-name/relative/path\" (e.g. \"my-skill/references/foo.md\")."
            .to_owned(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "The skill name as listed in the Available Skills section, \
                        or \"skill-name/relative/path\" to load a supporting file."
                }
            },
            "required": ["name"]
        }),
    }
}

/// Execute a `load_skill` call. Returns a `ToolResult` on success or a
/// user-visible error result if the skill is not found or cannot be read.
pub async fn call_load_skill(arguments: &Value, skills: &[SkillEntry]) -> ToolResult {
    let name = match arguments.get("name").and_then(Value::as_str) {
        Some(n) => n,
        None => {
            return error_result("load_skill: missing required argument \"name\"");
        }
    };

    // Two forms:
    //   "skill-name"            → load SKILL.md body + ## Supporting Files section
    //   "skill-name/rel/path"   → load a specific supporting file
    if let Some((skill_name, rel_path)) = name.split_once('/') {
        return load_supporting_file(skill_name, rel_path, skills).await;
    }

    // Plain skill-name form: load SKILL.md body.
    let entry = match skills.iter().find(|s| s.name == name) {
        Some(e) => e,
        None => {
            let available: Vec<&str> = skills.iter().map(|s| s.name.as_str()).collect();
            return error_result(&format!(
                "load_skill: skill {name:?} not found. Available: {available:?}"
            ));
        }
    };

    // Read the file off the async executor to avoid blocking a Tokio worker.
    let skill_path = entry.path.clone();
    let raw = match tokio::task::spawn_blocking(move || std::fs::read_to_string(&skill_path))
        .await
        .unwrap_or_else(|e| Err(std::io::Error::other(e)))
    {
        Ok(s) => s,
        Err(e) => {
            return error_result(&format!("load_skill: could not read {:?}: {e}", entry.path));
        }
    };

    // Strip the YAML frontmatter — the agent already knows name/description
    // from the system prompt; return only the body.
    let body = strip_frontmatter(&raw);

    let mut output = body.to_owned();

    // Append ## Supporting Files section if this skill has any.
    if !entry.supporting_files.is_empty() {
        let skill_dir = entry.path.parent().unwrap_or(&entry.path);
        output.push_str("\n\n## Supporting Files\n\n");
        for file in &entry.supporting_files {
            if let Ok(rel) = file.strip_prefix(skill_dir) {
                let rel_str = rel.to_string_lossy().replace('\\', "/");
                output.push_str(&format!(
                    "- {} (load_skill(name: \"{}/{}\"))\n",
                    rel_str, entry.name, rel_str
                ));
            }
        }
    }

    // Apply the size cap to the full output (body + Supporting Files section)
    // so the total tool result stays within MAX_SKILL_BODY_BYTES.
    let output = truncate_skill_output(output);

    ToolResult {
        provider_id: String::new(),
        content: vec![ToolResultContent::Text(output)],
        is_error: false,
    }
}

/// Load a supporting file identified by `skill_name/rel_path`.
/// Matches against the pre-enumerated `supporting_files` list and applies a
/// canonicalize-based traversal guard before reading.
async fn load_supporting_file(
    skill_name: &str,
    rel_path: &str,
    skills: &[SkillEntry],
) -> ToolResult {
    let rel_path = rel_path.replace('\\', "/");

    let entry = match skills.iter().find(|s| s.name == skill_name) {
        Some(e) => e,
        None => {
            let available: Vec<&str> = skills.iter().map(|s| s.name.as_str()).collect();
            return error_result(&format!(
                "load_skill: skill {skill_name:?} not found. Available: {available:?}"
            ));
        }
    };

    let skill_dir = match entry.path.parent() {
        Some(d) => d,
        None => {
            return error_result(&format!(
                "load_skill: could not determine skill directory for {skill_name:?}"
            ));
        }
    };

    // Match rel_path against the pre-enumerated supporting_files list.
    let matched = entry.supporting_files.iter().find(|f| {
        f.strip_prefix(skill_dir)
            .map(|r| r.to_string_lossy().replace('\\', "/") == rel_path)
            .unwrap_or(false)
    });

    let file_path = match matched {
        Some(p) => p,
        None => {
            let available: Vec<String> = entry
                .supporting_files
                .iter()
                .filter_map(|f| {
                    f.strip_prefix(skill_dir)
                        .ok()
                        .map(|r| r.to_string_lossy().replace('\\', "/"))
                })
                .collect();
            if available.is_empty() {
                return error_result(&format!(
                    "load_skill: skill {skill_name:?} has no supporting files."
                ));
            }
            return error_result(&format!(
                "load_skill: file {rel_path:?} not found in skill {skill_name:?}. \
                 Available: {available:?}"
            ));
        }
    };

    // Traversal guard: canonicalize both paths and verify the file stays inside
    // the skill directory. Fail hard if the skill directory itself can't be
    // canonicalized — a degraded guard is worse than no guard.
    let canonical_skill_dir = match skill_dir.canonicalize() {
        Ok(p) => p,
        Err(e) => {
            return error_result(&format!(
                "load_skill: could not canonicalize skill directory for {skill_name:?}: {e}"
            ));
        }
    };

    // Clone the path so we can move it into spawn_blocking.
    let file_path = file_path.clone();
    let skill_name = skill_name.to_owned();
    let rel_path_owned = rel_path.clone();

    match tokio::task::spawn_blocking(move || file_path.canonicalize().map(|c| (c, file_path)))
        .await
        .unwrap_or_else(|e| Err(std::io::Error::other(e)))
    {
        Ok((canonical_file, resolved_path)) if canonical_file.starts_with(&canonical_skill_dir) => {
            match tokio::task::spawn_blocking(move || std::fs::read_to_string(&resolved_path))
                .await
                .unwrap_or_else(|e| Err(std::io::Error::other(e)))
            {
                Ok(content) => {
                    let output = format!(
                        "# Loaded: {}/{}\n\n{}\n\n---\nFile loaded into context.",
                        skill_name, rel_path_owned, content
                    );
                    let output = truncate_skill_output(output);
                    ToolResult {
                        provider_id: String::new(),
                        content: vec![ToolResultContent::Text(output)],
                        is_error: false,
                    }
                }
                Err(e) => error_result(&format!(
                    "load_skill: could not read {skill_name:?}/{rel_path_owned}: {e}"
                )),
            }
        }
        Ok(_) => error_result(&format!(
            "load_skill: refusing to load {skill_name:?}/{rel_path_owned}: \
             resolves outside the skill directory"
        )),
        Err(e) => error_result(&format!(
            "load_skill: could not resolve {skill_name:?}/{rel_path_owned}: {e}"
        )),
    }
}

/// Marker appended when `load_skill` output is truncated at the size cap.
///
/// The marker is in-band so the model can see that content was lost and the
/// caller can detect it without a separate channel. It is deliberately included
/// in the truncation budget: the visible content is shortened so the marker
/// still fits within `MAX_SKILL_BODY_BYTES`.
const TRUNCATION_MARKER: &str = "\n\n[…truncated…]";

/// Apply the `MAX_SKILL_BODY_BYTES` cap to a `load_skill` output, leaving an
/// in-band marker when truncation occurred (#4163).
///
/// Previously the truncated text was returned silently — the model received
/// the body as if it were complete, so instructions in the tail of a SKILL.md
/// (output format, refusal rules, verification checklist) were silently gone.
fn truncate_skill_output(output: String) -> String {
    if output.len() <= MAX_SKILL_BODY_BYTES {
        return output;
    }
    // Reserve room for the marker so the total stays within the cap.
    let budget = MAX_SKILL_BODY_BYTES.saturating_sub(TRUNCATION_MARKER.len());
    let truncated = truncate_at_boundary(&output, budget);
    let mut result = String::with_capacity(truncated.len() + TRUNCATION_MARKER.len());
    result.push_str(truncated);
    result.push_str(TRUNCATION_MARKER);
    result
}

fn error_result(msg: &str) -> ToolResult {
    ToolResult {
        provider_id: String::new(),
        content: vec![ToolResultContent::Text(msg.to_owned())],
        is_error: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use tempfile::TempDir;

    fn text_content(result: &ToolResult) -> String {
        match &result.content[0] {
            ToolResultContent::Text(t) => t.clone(),
            ToolResultContent::Image { .. } => panic!("unexpected Image content in test"),
        }
    }

    fn make_skill(name: &str, description: &str, path: PathBuf) -> SkillEntry {
        SkillEntry {
            name: name.to_owned(),
            description: description.to_owned(),
            path,
            supporting_files: Vec::new(),
        }
    }

    fn make_skill_with_files(
        name: &str,
        description: &str,
        path: PathBuf,
        supporting_files: Vec<PathBuf>,
    ) -> SkillEntry {
        SkillEntry {
            name: name.to_owned(),
            description: description.to_owned(),
            path,
            supporting_files,
        }
    }

    #[tokio::test]
    async fn call_load_skill_missing_name_arg() {
        let result = call_load_skill(&serde_json::json!({}), &[]).await;
        assert!(result.is_error);
        let text = text_content(&result);
        assert!(text.contains("missing required argument"), "got: {text}");
    }

    #[tokio::test]
    async fn call_load_skill_skill_not_found() {
        let result = call_load_skill(&serde_json::json!({"name": "no-such"}), &[]).await;
        assert!(result.is_error);
        let text = text_content(&result);
        assert!(text.contains("not found"), "got: {text}");
    }

    #[tokio::test]
    async fn call_load_skill_returns_body_strips_frontmatter() {
        let tmp = TempDir::new().unwrap();
        let skill_md = tmp.path().join("SKILL.md");
        std::fs::write(
            &skill_md,
            "---\nname: test\ndescription: A test\n---\nSkill body here.\n",
        )
        .unwrap();
        let skills = vec![make_skill("test", "A test", skill_md)];
        let result = call_load_skill(&serde_json::json!({"name": "test"}), &skills).await;
        assert!(!result.is_error);
        let text = text_content(&result);
        assert!(text.contains("Skill body here."), "got: {text}");
        assert!(
            !text.contains("---"),
            "frontmatter should be stripped: {text}"
        );
    }

    #[tokio::test]
    async fn call_load_skill_appends_supporting_files_section() {
        let tmp = TempDir::new().unwrap();
        let skill_dir = tmp.path();
        let skill_md = skill_dir.join("SKILL.md");
        std::fs::write(
            &skill_md,
            "---\nname: my-skill\ndescription: desc\n---\nBody.\n",
        )
        .unwrap();
        let refs_dir = skill_dir.join("references");
        std::fs::create_dir_all(&refs_dir).unwrap();
        let ref_file = refs_dir.join("foo.md");
        std::fs::write(&ref_file, "Reference content.").unwrap();

        let skills = vec![make_skill_with_files(
            "my-skill",
            "desc",
            skill_md,
            vec![ref_file],
        )];
        let result = call_load_skill(&serde_json::json!({"name": "my-skill"}), &skills).await;
        assert!(!result.is_error);
        let text = text_content(&result);
        assert!(text.contains("Body."), "body missing: {text}");
        assert!(
            text.contains("## Supporting Files"),
            "missing Supporting Files section: {text}"
        );
        assert!(
            text.contains("references/foo.md"),
            "missing file listing: {text}"
        );
        assert!(
            text.contains("load_skill(name: \"my-skill/references/foo.md\")"),
            "missing load_skill hint: {text}"
        );
    }

    #[tokio::test]
    async fn call_load_skill_no_supporting_files_section_when_empty() {
        let tmp = TempDir::new().unwrap();
        let skill_md = tmp.path().join("SKILL.md");
        std::fs::write(
            &skill_md,
            "---\nname: bare\ndescription: desc\n---\nBody.\n",
        )
        .unwrap();
        let skills = vec![make_skill("bare", "desc", skill_md)];
        let result = call_load_skill(&serde_json::json!({"name": "bare"}), &skills).await;
        assert!(!result.is_error);
        let text = text_content(&result);
        assert!(
            !text.contains("## Supporting Files"),
            "should not have Supporting Files section when none: {text}"
        );
    }

    #[tokio::test]
    async fn call_load_skill_supporting_file_returns_content() {
        let tmp = TempDir::new().unwrap();
        let skill_dir = tmp.path();
        let skill_md = skill_dir.join("SKILL.md");
        std::fs::write(
            &skill_md,
            "---\nname: my-skill\ndescription: desc\n---\nBody.\n",
        )
        .unwrap();
        let refs_dir = skill_dir.join("references");
        std::fs::create_dir_all(&refs_dir).unwrap();
        let ref_file = refs_dir.join("foo.md");
        std::fs::write(&ref_file, "Reference content here.").unwrap();

        let skills = vec![make_skill_with_files(
            "my-skill",
            "desc",
            skill_md,
            vec![ref_file],
        )];
        let result = call_load_skill(
            &serde_json::json!({"name": "my-skill/references/foo.md"}),
            &skills,
        )
        .await;
        assert!(!result.is_error, "expected success, got error");
        let text = text_content(&result);
        assert!(
            text.contains("Reference content here."),
            "file content missing: {text}"
        );
        assert!(
            text.contains("# Loaded: my-skill/references/foo.md"),
            "missing header: {text}"
        );
    }

    #[tokio::test]
    async fn call_load_skill_supporting_file_not_found_lists_available() {
        let tmp = TempDir::new().unwrap();
        let skill_dir = tmp.path();
        let skill_md = skill_dir.join("SKILL.md");
        std::fs::write(
            &skill_md,
            "---\nname: my-skill\ndescription: desc\n---\nBody.\n",
        )
        .unwrap();
        let refs_dir = skill_dir.join("references");
        std::fs::create_dir_all(&refs_dir).unwrap();
        let ref_file = refs_dir.join("foo.md");
        std::fs::write(&ref_file, "content").unwrap();

        let skills = vec![make_skill_with_files(
            "my-skill",
            "desc",
            skill_md,
            vec![ref_file],
        )];
        let result = call_load_skill(
            &serde_json::json!({"name": "my-skill/references/missing.md"}),
            &skills,
        )
        .await;
        assert!(result.is_error);
        let text = text_content(&result);
        assert!(text.contains("not found"), "got: {text}");
        assert!(
            text.contains("references/foo.md"),
            "should list available: {text}"
        );
    }

    #[tokio::test]
    async fn call_load_skill_no_supporting_files_error_message() {
        let tmp = TempDir::new().unwrap();
        let skill_md = tmp.path().join("SKILL.md");
        std::fs::write(
            &skill_md,
            "---\nname: bare\ndescription: desc\n---\nBody.\n",
        )
        .unwrap();
        let skills = vec![make_skill("bare", "desc", skill_md)];
        let result =
            call_load_skill(&serde_json::json!({"name": "bare/anything.md"}), &skills).await;
        assert!(result.is_error);
        let text = text_content(&result);
        assert!(text.contains("no supporting files"), "got: {text}");
    }

    #[tokio::test]
    async fn call_load_skill_traversal_guard_rejects_escape() {
        let tmp = TempDir::new().unwrap();
        let skill_dir = tmp.path().join("my-skill");
        std::fs::create_dir_all(&skill_dir).unwrap();
        let skill_md = skill_dir.join("SKILL.md");
        std::fs::write(
            &skill_md,
            "---\nname: my-skill\ndescription: desc\n---\nBody.\n",
        )
        .unwrap();

        // Create a file outside the skill dir that we'll try to reference.
        let outside_file = tmp.path().join("secret.txt");
        std::fs::write(&outside_file, "secret content").unwrap();

        // Manually construct a SkillEntry with a supporting_files entry that
        // points outside the skill dir — simulating a crafted/malicious entry.
        // The traversal guard should catch this.
        let skills = vec![make_skill_with_files(
            "my-skill",
            "desc",
            skill_md.clone(),
            vec![outside_file.clone()],
        )];

        // The slash form splits "my-skill/../secret.txt" into skill_name="my-skill"
        // and rel_path="../secret.txt". strip_prefix(skill_dir) on outside_file
        // fails, so it won't match any supporting_files entry — the pre-enumeration
        // guard rejects it before the canonicalize guard even fires.
        let result = call_load_skill(
            &serde_json::json!({"name": "my-skill/../secret.txt"}),
            &skills,
        )
        .await;
        assert!(result.is_error, "traversal attempt should be rejected");
        let text = text_content(&result);
        assert!(
            !text.contains("secret content"),
            "secret content must not be returned: {text}"
        );
    }

    #[tokio::test]
    async fn call_load_skill_truncates_large_body() {
        let tmp = TempDir::new().unwrap();
        let skill_dir = tmp.path();
        let skill_md = skill_dir.join("SKILL.md");
        // Build a body that exceeds MAX_SKILL_BODY_BYTES (32 KiB).
        let large_body = "x".repeat(40 * 1024);
        std::fs::write(
            &skill_md,
            format!("---\nname: big\ndescription: desc\n---\n{large_body}\n"),
        )
        .unwrap();
        // Add a supporting file so the Supporting Files section is also appended
        // before the cap is applied.
        let refs_dir = skill_dir.join("references");
        std::fs::create_dir_all(&refs_dir).unwrap();
        let ref_file = refs_dir.join("extra.md");
        std::fs::write(&ref_file, "extra content").unwrap();

        let skills = vec![make_skill_with_files(
            "big",
            "desc",
            skill_md,
            vec![ref_file],
        )];
        let result = call_load_skill(&serde_json::json!({"name": "big"}), &skills).await;
        assert!(!result.is_error);
        let text = text_content(&result);
        assert!(
            text.len() <= MAX_SKILL_BODY_BYTES,
            "output length {} exceeds MAX_SKILL_BODY_BYTES {}",
            text.len(),
            MAX_SKILL_BODY_BYTES
        );
    }

    #[tokio::test]
    async fn call_load_skill_truncates_large_supporting_file() {
        let tmp = TempDir::new().unwrap();
        let skill_dir = tmp.path();
        let skill_md = skill_dir.join("SKILL.md");
        std::fs::write(&skill_md, "---\nname: big\ndescription: desc\n---\nBody.\n").unwrap();

        let refs_dir = skill_dir.join("references");
        std::fs::create_dir_all(&refs_dir).unwrap();
        let ref_file = refs_dir.join("huge.md");
        std::fs::write(&ref_file, "x".repeat(MAX_SKILL_BODY_BYTES * 2)).unwrap();

        let skills = vec![make_skill_with_files(
            "big",
            "desc",
            skill_md,
            vec![ref_file],
        )];
        let result = call_load_skill(
            &serde_json::json!({"name": "big/references/huge.md"}),
            &skills,
        )
        .await;
        assert!(!result.is_error);
        let text = text_content(&result);
        assert!(
            text.len() <= MAX_SKILL_BODY_BYTES,
            "output length {} exceeds MAX_SKILL_BODY_BYTES {}",
            text.len(),
            MAX_SKILL_BODY_BYTES
        );
        assert!(
            text.starts_with("# Loaded: big/references/huge.md"),
            "missing supporting-file header: {text}"
        );
        assert!(
            text.ends_with(TRUNCATION_MARKER.trim_end()),
            "truncated supporting-file output must carry the in-band marker: {text}"
        );
    }

    #[test]
    fn truncate_skill_output_under_cap_is_unchanged() {
        let input = "short skill body".to_owned();
        let out = truncate_skill_output(input.clone());
        assert_eq!(out, input);
        assert!(!out.contains("truncated"));
    }

    #[test]
    fn truncate_skill_output_over_cap_appends_marker_and_stays_within_cap() {
        // Build a body well over the cap with a sentinel at the very end.
        let mut body = String::with_capacity(MAX_SKILL_BODY_BYTES + 1024);
        while body.len() < MAX_SKILL_BODY_BYTES + 1024 {
            body.push_str("padding line of moderate length\n");
        }
        body.push_str("SENTINEL_TAIL_DO_NOT_DROP_MARKER");

        let out = truncate_skill_output(body);
        assert!(
            out.len() <= MAX_SKILL_BODY_BYTES,
            "truncated output {} exceeds cap {}",
            out.len(),
            MAX_SKILL_BODY_BYTES
        );
        assert!(
            out.ends_with(TRUNCATION_MARKER.trim_end()),
            "output must end with the truncation marker, got tail: {:?}",
            &out[out.len().saturating_sub(64)..]
        );
        // The tail sentinel was truncated away, but the marker signals the loss.
        assert!(!out.contains("SENTINEL_TAIL_DO_NOT_DROP_MARKER"));
    }

    #[test]
    fn truncate_skill_output_emits_valid_utf8_at_boundary() {
        // Multi-byte chars straddling the budget must not split — the boundary
        // fn handles it, and the marker must follow valid UTF-8.
        let mut body = String::new();
        while body.len() < MAX_SKILL_BODY_BYTES {
            body.push('é'); // 2 bytes in UTF-8
        }
        body.push_str("extra to exceed the cap");

        let out = truncate_skill_output(body);
        assert!(out.len() <= MAX_SKILL_BODY_BYTES);
        assert!(out.ends_with(TRUNCATION_MARKER.trim_end()));
        // If we'd split a char this would panic on access; reaching here proves validity.
        assert!(out.is_char_boundary(out.len() - TRUNCATION_MARKER.trim_end().len()));
    }
}
