//! Validation for human-reviewed agent definition text.
//!
//! The rules live in `buzz_sdk::definition_validation` so Desktop and
//! `buzz personas` accept exactly the same definitions; this module holds the
//! managed-agent-specific wrapper and the Unicode drift guard.

pub(crate) use buzz_sdk_pkg::definition_validation::validate_agent_definition_text;

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

#[cfg(test)]
mod tests {
    use super::*;
    use buzz_sdk_pkg::definition_validation::is_extended_pictographic;
    use regex::Regex;

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

    /// The SDK hardcodes `Extended_Pictographic` to stay off `regex`, which
    /// every Buzz binary would otherwise inherit. Desktop already has `regex`,
    /// so the authoritative comparison runs here: any divergence from the
    /// Unicode property — including a table update in a future `regex` — means
    /// Desktop and the CLI would judge the same emoji differently.
    #[test]
    fn sdk_pictographic_table_matches_the_unicode_property() {
        let pattern = Regex::new(r"^\p{Extended_Pictographic}$").expect("valid pattern");
        let mut buf = [0u8; 4];
        for code_point in 0u32..=0x10FFFF {
            let Some(character) = char::from_u32(code_point) else {
                continue;
            };
            assert_eq!(
                is_extended_pictographic(character),
                pattern.is_match(character.encode_utf8(&mut buf)),
                "U+{code_point:04X} disagrees with \\p{{Extended_Pictographic}}"
            );
        }
    }
}
