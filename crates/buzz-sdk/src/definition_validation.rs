//! Validation for human-reviewed agent definition text.
//!
//! Shared definitions are executable configuration: `system_prompt` is shown to
//! a person, then delivered verbatim to an ACP harness. Characters that consume
//! input bytes without a visible glyph break that review invariant and are
//! rejected rather than silently stripped.
//!
//! This lives in the SDK so every publisher — Buzz Desktop and `buzz personas`
//! alike — applies the same rules. A definition one accepts and the other
//! rejects is a definition that publishes fine and then fails to launch.

/// Maximum display name length, in characters.
pub const MAX_DISPLAY_NAME_CHARS: usize = 128;

/// Maximum system prompt length, in bytes.
pub const MAX_SYSTEM_PROMPT_BYTES: usize = 64 * 1024;

const EMOJI_VARIATION_SELECTOR: char = '\u{FE0F}';
const ZERO_WIDTH_JOINER: char = '\u{200D}';

/// Validate the human-visible fields of an agent definition.
///
/// Returns a human-readable reason on rejection. Empty or over-long names, an
/// over-long prompt, control characters, and detached invisible formatting are
/// all errors; emoji sequences that render as a single glyph are allowed.
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

/// Validate `value` as human-reviewed text.
///
/// `allow_layout_controls` permits newlines and tabs — true for a prompt body,
/// false for a single-line name.
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

/// Unicode `Extended_Pictographic` (UTS #51), as a literal range table.
///
/// The SDK is on the dependency path of every Buzz binary, so this avoids
/// pulling `regex` in for one property lookup. Buzz Desktop already depends on
/// `regex` and cross-checks this table against `\p{Extended_Pictographic}` over
/// the whole code space, which is what catches drift when the tables move.
pub fn is_extended_pictographic(character: char) -> bool {
    matches!(
        character as u32,
        0x00A9
            | 0x00AE
            | 0x203C
            | 0x2049
            | 0x2122
            | 0x2139
            | 0x2194..=0x2199
            | 0x21A9..=0x21AA
            | 0x231A..=0x231B
            | 0x2328
            | 0x2388
            | 0x23CF
            | 0x23E9..=0x23F3
            | 0x23F8..=0x23FA
            | 0x24C2
            | 0x25AA..=0x25AB
            | 0x25B6
            | 0x25C0
            | 0x25FB..=0x25FE
            | 0x2600..=0x2605
            | 0x2607..=0x2612
            | 0x2614..=0x2685
            | 0x2690..=0x2705
            | 0x2708..=0x2712
            | 0x2714
            | 0x2716
            | 0x271D
            | 0x2721
            | 0x2728
            | 0x2733..=0x2734
            | 0x2744
            | 0x2747
            | 0x274C
            | 0x274E
            | 0x2753..=0x2755
            | 0x2757
            | 0x2763..=0x2767
            | 0x2795..=0x2797
            | 0x27A1
            | 0x27B0
            | 0x27BF
            | 0x2934..=0x2935
            | 0x2B05..=0x2B07
            | 0x2B1B..=0x2B1C
            | 0x2B50
            | 0x2B55
            | 0x3030
            | 0x303D
            | 0x3297
            | 0x3299
            | 0x1F000..=0x1F0FF
            | 0x1F10D..=0x1F10F
            | 0x1F12F
            | 0x1F16C..=0x1F171
            | 0x1F17E..=0x1F17F
            | 0x1F18E
            | 0x1F191..=0x1F19A
            | 0x1F1AD..=0x1F1E5
            | 0x1F201..=0x1F20F
            | 0x1F21A
            | 0x1F22F
            | 0x1F232..=0x1F23A
            | 0x1F23C..=0x1F23F
            | 0x1F249..=0x1F3FA
            | 0x1F400..=0x1F53D
            | 0x1F546..=0x1F64F
            | 0x1F680..=0x1F6FF
            | 0x1F774..=0x1F77F
            | 0x1F7D5..=0x1F7FF
            | 0x1F80C..=0x1F80F
            | 0x1F848..=0x1F84F
            | 0x1F85A..=0x1F85F
            | 0x1F888..=0x1F88F
            | 0x1F8AE..=0x1F8FF
            | 0x1F90C..=0x1F93A
            | 0x1F93C..=0x1F945
            | 0x1F947..=0x1FAFF
            | 0x1FC00..=0x1FFFD
    )
}

/// Unicode `Default_Ignorable_Code_Point` ranges (DerivedCoreProperties).
///
/// Joiners and variation selectors remain in this set. The validation pass
/// makes a narrow contextual exception for rendered emoji composition while
/// rejecting detached instances and every other default-ignorable character.
pub fn is_default_ignorable(character: char) -> bool {
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

    /// Spot-check the range table's edges — the desktop crate holds the
    /// exhaustive cross-check against the Unicode property itself.
    #[test]
    fn extended_pictographic_table_covers_its_boundaries() {
        for c in ['\u{00A9}', '🐝', '\u{1FC00}', '\u{1FFFD}', '❤'] {
            assert!(is_extended_pictographic(c), "{c:?} should be pictographic");
        }
        for c in ['a', '\u{00A8}', '\u{00AA}', '\u{1FFFE}', '\u{FE0F}'] {
            assert!(
                !is_extended_pictographic(c),
                "{c:?} should not be pictographic"
            );
        }
    }
}
