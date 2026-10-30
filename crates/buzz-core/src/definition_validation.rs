//! Validation for human-reviewed agent definition text.
//!
//! Shared definitions are executable configuration: `system_prompt` is shown
//! to a person, then delivered verbatim to an ACP harness. Characters that
//! consume input bytes without a visible glyph break that review invariant and
//! are rejected rather than silently stripped.

use regex::Regex;
use std::sync::LazyLock;

/// Cap for an agent display name, in characters.
pub const MAX_DISPLAY_NAME_CHARS: usize = 128;
/// Cap for an agent system prompt, in bytes.
pub const MAX_SYSTEM_PROMPT_BYTES: usize = 64 * 1024;
/// Cap for the optional public agent description.
pub const MAX_AGENT_DESCRIPTION_CHARS: usize = 280;
const EMOJI_VARIATION_SELECTOR: char = '\u{FE0F}';
const ZERO_WIDTH_JOINER: char = '\u{200D}';

static EXTENDED_PICTOGRAPHIC: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"^\p{Extended_Pictographic}$").ok());

/// The stock ACP harness command, always portable.
pub const DEFAULT_ACP_COMMAND: &str = "buzz-acp";

/// Whether a transport is a portable stock or conventional wrapper alias.
/// Shared artifacts carry aliases only, never machine paths or command lines.
/// Owner-controlled native commands and owner-device sync retain legacy values.
pub fn is_portable_acp_command(command: &str) -> bool {
    if command == DEFAULT_ACP_COMMAND {
        return true;
    }
    command.len() <= 255
        && command
            .strip_prefix("buzz-")
            .and_then(|name| name.strip_suffix("-acp"))
            .is_some_and(|name| {
                !name.is_empty()
                    && name
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_'))
            })
}

/// Reject nonportable commands at foreign catalog and snapshot boundaries.
pub fn validate_portable_acp_command(command: Option<&str>) -> Result<(), String> {
    if command.is_some_and(|command| !is_portable_acp_command(command)) {
        return Err("ACP command must be buzz-acp or a portable buzz-*-acp alias".to_string());
    }
    Ok(())
}

/// Validate the human-visible fields of an agent definition.
pub fn validate_agent_definition_text(
    display_name: &str,
    system_prompt: &str,
) -> Result<(), String> {
    if display_name.trim().is_empty() {
        return Err("Display name is required".to_string());
    }
    let display_name_chars = display_name.chars().count();
    if display_name_chars > MAX_DISPLAY_NAME_CHARS {
        return Err(format!(
            "Display name is too long ({display_name_chars} characters, max {MAX_DISPLAY_NAME_CHARS})"
        ));
    }
    if system_prompt.len() > MAX_SYSTEM_PROMPT_BYTES {
        return Err(format!(
            "Agent instructions are too long ({} bytes, max {MAX_SYSTEM_PROMPT_BYTES})",
            system_prompt.len()
        ));
    }

    validate_visible_text(display_name, "Display name", false)?;
    validate_visible_text(system_prompt, "Agent instructions", true)
}

/// Validate an optional public agent description: max 280 characters and the
/// same visible-text policy as the other definition fields (invisible, bidi,
/// and control characters are rejected, not stripped). `None` and the empty
/// string are both valid — the description is optional.
pub fn validate_agent_description_text(description: Option<&str>) -> Result<(), String> {
    let Some(description) = description else {
        return Ok(());
    };
    let description_chars = description.chars().count();
    if description_chars > MAX_AGENT_DESCRIPTION_CHARS {
        return Err(format!(
            "Description is too long ({description_chars} characters, max {MAX_AGENT_DESCRIPTION_CHARS})"
        ));
    }
    validate_visible_text(description, "Description", false)
}

/// Validate a shared persona's JSON body the way the desktop does when it
/// adopts a catalog entry: definition text, optional description, and a
/// portable ACP command. Type mismatches on those fields are rejections.
pub fn validate_persona_object(
    object: &serde_json::Map<String, serde_json::Value>,
) -> Result<(), String> {
    use serde_json::Value;
    let display_name = object
        .get("display_name")
        .and_then(Value::as_str)
        .ok_or_else(|| "display_name must be a string".to_string())?;
    let system_prompt = object
        .get("system_prompt")
        .and_then(Value::as_str)
        .unwrap_or_default();
    validate_agent_definition_text(display_name, system_prompt)?;
    let description = match object.get("description") {
        None | Some(Value::Null) => None,
        Some(Value::String(value)) => Some(value.as_str()),
        Some(_) => return Err("description must be a string".to_string()),
    };
    validate_agent_description_text(description)?;
    let acp_command = match object.get("acp_command") {
        None | Some(Value::Null) => None,
        Some(Value::String(command)) => Some(command.as_str()),
        Some(_) => return Err("acp_command must be a string".to_string()),
    };
    validate_portable_acp_command(acp_command)
}

/// Validate the human-reviewed definition text carried by a managed agent.
///
/// Definition-linked agents resolve their executable prompt through the
/// separately validated persona, so only their instance name is checked here.
/// Definition-less agents carry their executable prompt directly and must
/// validate both fields at every local, inbound, and publication boundary.
pub fn validate_managed_agent_definition_text(
    name: &str,
    persona_id: Option<&str>,
    system_prompt: Option<&str>,
) -> Result<(), String> {
    let executable_prompt = if persona_id.is_none() {
        system_prompt.unwrap_or_default()
    } else {
        ""
    };
    validate_agent_definition_text(name, executable_prompt)
}

/// Reject control and default-ignorable characters in human-reviewed text.
///
/// The shared executable-definition invariant: a recipient reviews a visible
/// string, then it is delivered verbatim to an ACP harness. Invisible,
/// default-ignorable, and bidi-override characters make what executes differ
/// from what was reviewed, so they are refused rather than silently stripped.
/// `allow_layout_controls` permits `\n`/`\t` for multiline fields.
pub fn validate_visible_text(
    value: &str,
    label: &str,
    allow_layout_controls: bool,
) -> Result<(), String> {
    let characters = value.chars().collect::<Vec<_>>();
    for (index, &character) in characters.iter().enumerate() {
        let allowed_layout_control = allow_layout_controls && matches!(character, '\n' | '\t');
        let allowed_emoji_format = is_allowed_emoji_format(&characters, index);
        if (!allowed_layout_control && character.is_control())
            || (is_default_ignorable(character) && !allowed_emoji_format)
        {
            return Err(format!(
                "{label} contains prohibited invisible or formatting character U+{:04X}",
                character as u32
            ));
        }
    }
    Ok(())
}

fn is_allowed_emoji_format(characters: &[char], index: usize) -> bool {
    match characters[index] {
        EMOJI_VARIATION_SELECTOR => index
            .checked_sub(1)
            .and_then(|previous| characters.get(previous))
            .is_some_and(|&character| is_emoji_variation_base(character)),
        ZERO_WIDTH_JOINER => {
            has_preceding_emoji_base(characters, index)
                && characters
                    .get(index + 1)
                    .is_some_and(|&character| is_extended_pictographic(character))
        }
        _ => false,
    }
}

fn has_preceding_emoji_base(characters: &[char], index: usize) -> bool {
    let mut previous = index.checked_sub(1);
    while let Some(previous_index) = previous {
        let character = characters[previous_index];
        if character != EMOJI_VARIATION_SELECTOR && !is_emoji_modifier(character) {
            return is_extended_pictographic(character);
        }
        previous = previous_index.checked_sub(1);
    }
    false
}

fn is_emoji_variation_base(character: char) -> bool {
    matches!(character, '#' | '*' | '0'..='9') || is_extended_pictographic(character)
}

fn is_emoji_modifier(character: char) -> bool {
    matches!(character as u32, 0x1F3FB..=0x1F3FF)
}

fn is_extended_pictographic(character: char) -> bool {
    let mut encoded = [0; 4];
    let character = character.encode_utf8(&mut encoded);
    EXTENDED_PICTOGRAPHIC
        .as_ref()
        .is_some_and(|pattern| pattern.is_match(character))
}

/// Unicode `Default_Ignorable_Code_Point` ranges (DerivedCoreProperties).
///
/// Joiners and variation selectors remain in this set. The validation pass
/// makes a narrow contextual exception for rendered emoji composition while
/// rejecting detached instances and every other default-ignorable character.
fn is_default_ignorable(character: char) -> bool {
    matches!(
        character as u32,
        0x00AD
            | 0x034F
            | 0x061C
            | 0x115F..=0x1160
            | 0x17B4..=0x17B5
            | 0x180B..=0x180F
            | 0x200B..=0x200F
            | 0x202A..=0x202E
            | 0x2060..=0x206F
            | 0x3164
            | 0xFE00..=0xFE0F
            | 0xFEFF
            | 0xFFA0
            | 0xFFF0..=0xFFF8
            | 0x1BCA0..=0x1BCA3
            | 0x1D173..=0x1D17A
            | 0xE0000..=0xE0FFF
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_plain_multiline_instructions() {
        assert!(validate_agent_definition_text(
            "Code Reviewer 🐝",
            "Review changes.\n\tCall out security risks."
        )
        .is_ok());
    }

    #[test]
    fn accepts_rendered_emoji_sequences_in_names_and_prompts() {
        for emoji in ["❤️", "☕️", "👩‍💻", "🧑🏽‍💻", "👨‍👩‍👧‍👦", "1️⃣"]
        {
            assert!(validate_agent_definition_text(
                &format!("Reviewer {emoji}"),
                &format!("Review changes {emoji}")
            )
            .is_ok());
        }
    }

    #[test]
    fn rejects_default_ignorable_characters_in_name_or_prompt() {
        for character in [
            '\u{00AD}',
            '\u{034F}',
            '\u{200B}',
            '\u{202E}',
            '\u{2060}',
            '\u{2066}',
            '\u{3164}',
            '\u{E007F}',
        ] {
            let name = format!("Review{character}er");
            let prompt = format!("Review code.{character}");
            assert!(validate_agent_definition_text(&name, "Review code.").is_err());
            assert!(validate_agent_definition_text("Reviewer", &prompt).is_err());
        }
    }

    #[test]
    fn rejects_detached_or_text_embedded_emoji_formatting() {
        for value in [
            "Review\u{FE0F}er",
            "Review\u{200D}er",
            "Review code.\u{200D}",
        ] {
            assert!(validate_agent_definition_text(value, "Review code.").is_err());
            assert!(validate_agent_definition_text("Reviewer", value).is_err());
        }
    }

    #[test]
    fn rejects_emoji_tag_sequences() {
        let tagged_flag = "\u{1F3F4}\u{E0067}\u{E0062}\u{E0073}\u{E0063}\u{E0074}\u{E007F}";
        assert!(
            validate_agent_definition_text(&format!("Reviewer {tagged_flag}"), "Review code.")
                .is_err()
        );
        assert!(
            validate_agent_definition_text("Reviewer", &format!("Review code. {tagged_flag}"))
                .is_err()
        );
    }

    #[test]
    fn rejects_non_layout_control_characters() {
        for character in ['\0', '\r', '\u{0007}', '\u{0085}'] {
            let prompt = format!("Review{character}code");
            assert!(validate_agent_definition_text("Reviewer", &prompt).is_err());
        }
    }

    #[test]
    fn enforces_display_name_and_prompt_bounds() {
        assert!(validate_agent_definition_text(&"a".repeat(129), "prompt").is_err());
        assert!(validate_agent_definition_text("Reviewer", &"a".repeat(64 * 1024 + 1)).is_err());
    }

    #[test]
    fn description_accepts_none_empty_and_plain_text() {
        assert!(validate_agent_description_text(None).is_ok());
        assert!(validate_agent_description_text(Some("")).is_ok());
        assert!(validate_agent_description_text(Some("Buttercup, a software engineer 🐝")).is_ok());
        assert!(
            validate_agent_description_text(Some(&"a".repeat(MAX_AGENT_DESCRIPTION_CHARS))).is_ok()
        );
    }

    #[test]
    fn description_rejects_over_280_chars() {
        assert!(validate_agent_description_text(Some(
            &"a".repeat(MAX_AGENT_DESCRIPTION_CHARS + 1)
        ))
        .is_err());
    }

    #[test]
    fn description_rejects_invisible_bidi_and_control_characters() {
        for character in ['\u{200B}', '\u{202E}', '\u{2066}', '\0', '\r', '\u{0007}'] {
            for description in [
                format!("A helpful{character}agent"),
                format!("{character}A helpful agent"),
                format!("A helpful agent{character}"),
            ] {
                assert!(validate_agent_description_text(Some(&description)).is_err());
            }
        }
    }

    #[test]
    fn definition_less_managed_agent_validates_its_own_name_and_prompt() {
        assert!(validate_managed_agent_definition_text(
            "Review\u{200B}er",
            None,
            Some("Review code."),
        )
        .is_err());
        assert!(validate_managed_agent_definition_text(
            "Reviewer",
            None,
            Some("Review\u{200B} code."),
        )
        .is_err());
        assert!(validate_managed_agent_definition_text(
            "Reviewer 🐝",
            None,
            Some("Review changes.\n\tCall out risks."),
        )
        .is_ok());
    }

    #[test]
    fn definition_linked_managed_agent_ignores_inert_record_prompt() {
        assert!(validate_managed_agent_definition_text(
            "Reviewer",
            Some("custom:reviewer"),
            Some("stale\u{200B} prompt"),
        )
        .is_ok());
    }

    #[test]
    fn persona_object_validation_matches_the_adoption_rules() {
        use serde_json::json;
        let long_description = "a".repeat(MAX_AGENT_DESCRIPTION_CHARS + 1);
        let cases = [
            (json!({"display_name": "Zed", "system_prompt": "p"}), None),
            (json!({"display_name": "Zed"}), None),
            (
                json!({"display_name": "Zed", "description": null, "acp_command": "buzz-janet-acp"}),
                None,
            ),
            (
                json!({"display_name": 7}),
                Some("display_name must be a string"),
            ),
            (
                json!({"display_name": "  "}),
                Some("Display name is required"),
            ),
            (
                json!({"display_name": "Z\u{200B}ed"}),
                Some("prohibited invisible"),
            ),
            (
                json!({"display_name": "Zed", "system_prompt": "p\u{202E}"}),
                Some("Agent instructions contains"),
            ),
            (
                json!({"display_name": "Zed", "description": 5}),
                Some("description must be a string"),
            ),
            (
                json!({"display_name": "Zed", "description": long_description}),
                Some("Description is too long"),
            ),
            (
                json!({"display_name": "Zed", "acp_command": 5}),
                Some("acp_command must be a string"),
            ),
            (
                json!({"display_name": "Zed", "acp_command": "/bin/sh"}),
                Some("ACP command must be"),
            ),
        ];
        for (value, expected) in cases {
            let result = validate_persona_object(value.as_object().unwrap());
            match expected {
                None => assert!(result.is_ok(), "{value}: {result:?}"),
                Some(fragment) => {
                    let error = result.expect_err(&value.to_string());
                    assert!(error.contains(fragment), "{value}: {error}");
                }
            }
        }
    }

    #[test]
    fn portable_acp_aliases_exclude_paths_arguments_and_platform_metacharacters() {
        for command in ["buzz-acp", "buzz-janet-acp", "buzz-Team_2-acp"] {
            assert!(is_portable_acp_command(command), "{command}");
            assert!(validate_portable_acp_command(Some(command)).is_ok());
        }
        assert!(validate_portable_acp_command(None).is_ok());
        for command in [
            "",
            "buzz--acp",
            "/tmp/buzz-janet-acp",
            r"C:\buzz-janet-acp.cmd",
            "buzz-../evil-acp",
            "buzz-a b-acp",
            "buzz-a&b-acp",
            "buzz-a%PATH%-acp",
            "buzz-a\nb-acp",
            "buzz-a\u{202e}b-acp",
            "other-command",
            "buzz-janet-acp.exe",
        ] {
            assert!(!is_portable_acp_command(command), "{command:?}");
            assert_eq!(
                validate_portable_acp_command(Some(command)).unwrap_err(),
                "ACP command must be buzz-acp or a portable buzz-*-acp alias"
            );
        }
    }
}
