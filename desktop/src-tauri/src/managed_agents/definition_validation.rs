//! Validation for human-reviewed agent definition text.
//!
//! Shared definitions are executable configuration: `system_prompt` is shown
//! to a person, then delivered verbatim to an ACP harness. Characters that
//! consume input bytes without a visible glyph break that review invariant and
//! are rejected rather than silently stripped.

use regex::Regex;
use std::sync::LazyLock;

const MAX_DISPLAY_NAME_CHARS: usize = 128;
const MAX_SYSTEM_PROMPT_BYTES: usize = 64 * 1024;
/// Cap for the optional public agent description.
pub(crate) const MAX_AGENT_DESCRIPTION_CHARS: usize = 280;
const EMOJI_VARIATION_SELECTOR: char = '\u{FE0F}';
const ZERO_WIDTH_JOINER: char = '\u{200D}';

static EXTENDED_PICTOGRAPHIC: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"^\p{Extended_Pictographic}$").ok());

/// Validate the human-visible fields of an agent definition.
pub(crate) fn validate_agent_definition_text(
    display_name: &str,
    system_prompt: &str,
) -> Result<(), String> {
    validate_reviewed_text(
        display_name,
        "Display name",
        system_prompt,
        "Agent instructions",
    )
}

/// Validate the human-reviewed text carried by a team.
///
/// A team's `instructions` are runtime-layered into every member deployment,
/// so they are executable text under the same review contract as an agent
/// definition. A team carrying no instructions has no executable text and only
/// its name is checked.
pub(crate) fn validate_team_definition_text(
    name: &str,
    instructions: Option<&str>,
) -> Result<(), String> {
    validate_reviewed_text(
        name,
        "Team name",
        instructions.unwrap_or_default(),
        "Team instructions",
    )
}

/// Shared contract for reviewed-then-executed text. The labels differ per
/// surface so the error a person sees names the field they were editing; the
/// limits and the invisible-character rules are deliberately identical.
fn validate_reviewed_text(
    name: &str,
    name_label: &str,
    instructions: &str,
    instructions_label: &str,
) -> Result<(), String> {
    if name.trim().is_empty() {
        return Err(format!("{name_label} is required"));
    }
    let name_chars = name.chars().count();
    if name_chars > MAX_DISPLAY_NAME_CHARS {
        return Err(format!(
            "{name_label} is too long ({name_chars} characters, max {MAX_DISPLAY_NAME_CHARS})"
        ));
    }
    if instructions.len() > MAX_SYSTEM_PROMPT_BYTES {
        return Err(format!(
            "{instructions_label} are too long ({} bytes, max {MAX_SYSTEM_PROMPT_BYTES})",
            instructions.len()
        ));
    }

    validate_visible_text(name, name_label, false)?;
    validate_visible_text(instructions, instructions_label, true)
}

/// Validate an optional public agent description: max 280 characters and the
/// same visible-text policy as the other definition fields (invisible, bidi,
/// and control characters are rejected, not stripped). `None` and the empty
/// string are both valid — the description is optional.
pub(crate) fn validate_agent_description_text(description: Option<&str>) -> Result<(), String> {
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

/// Validate the human-reviewed definition text carried by a managed agent.
///
/// Definition-linked agents resolve their executable prompt through the
/// separately validated persona, so only their instance name is checked here.
/// Definition-less agents carry their executable prompt directly and must
/// validate both fields at every local, inbound, and publication boundary.
pub(crate) fn validate_managed_agent_definition_text(
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
pub(crate) fn validate_visible_text(
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

    // ── Team text: same contract, team-shaped labels ─────────────────────────
    //
    // Team `instructions` are runtime-layered into every member deployment, so
    // they are executed exactly like an agent's `system_prompt`. Before this
    // they were the one shared executable text with no review contract at all,
    // which made the same hidden characters safer in a team than in the agent
    // wrapped by it.

    #[test]
    fn team_text_rejects_the_same_invisible_characters_as_an_agent() {
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
            let name = format!("Release{character} Team");
            let instructions = format!("Ship the release.{character}");
            assert!(validate_team_definition_text(&name, Some("Ship it.")).is_err());
            assert!(validate_team_definition_text("Release Team", Some(&instructions)).is_err());
        }
    }

    #[test]
    fn team_text_accepts_ordinary_whitespace_and_emoji() {
        assert!(validate_team_definition_text(
            "Release Team 🚀",
            Some("Ship the release.\n\tPost the ledger row 🚀")
        )
        .is_ok());
    }

    #[test]
    fn a_team_without_instructions_carries_no_executable_text() {
        assert!(validate_team_definition_text("Release Team", None).is_ok());
    }

    #[test]
    fn team_errors_name_the_team_field_the_person_was_editing() {
        let name_error = validate_team_definition_text("", Some("Ship it.")).unwrap_err();
        assert!(
            name_error.starts_with("Team name"),
            "expected a team-shaped error, got {name_error}"
        );

        let long_instructions = "x".repeat(MAX_SYSTEM_PROMPT_BYTES + 1);
        let instructions_error =
            validate_team_definition_text("Release Team", Some(&long_instructions)).unwrap_err();
        assert!(
            instructions_error.starts_with("Team instructions"),
            "expected a team-shaped error, got {instructions_error}"
        );
    }

    #[test]
    fn agent_error_wording_is_unchanged_by_the_shared_contract() {
        assert_eq!(
            validate_agent_definition_text("", "Review code.").unwrap_err(),
            "Display name is required"
        );
        let long_prompt = "x".repeat(MAX_SYSTEM_PROMPT_BYTES + 1);
        assert!(validate_agent_definition_text("Reviewer", &long_prompt)
            .unwrap_err()
            .starts_with("Agent instructions are too long"));
    }

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
}
