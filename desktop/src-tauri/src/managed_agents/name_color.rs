//! Fixed 16-color palette a user can assign to an agent's display name.
//! Purely a local display preference — never published to a relay.

pub const AGENT_NAME_COLORS: [&str; 16] = [
    "red", "orange", "amber", "yellow", "lime", "green", "emerald", "teal",
    "cyan", "sky", "blue", "indigo", "violet", "purple", "fuchsia", "pink",
];

/// Validate a candidate `name_color`. `None` (no color chosen) is always
/// valid. `Some(id)` must be one of the 16 fixed palette ids — an unknown id
/// is rejected rather than silently dropped, so a typo'd frontend payload
/// fails loudly instead of the agent silently losing its chosen color.
pub fn validate_agent_name_color(value: Option<String>) -> Result<Option<String>, String> {
    match value {
        None => Ok(None),
        Some(id) if AGENT_NAME_COLORS.contains(&id.as_str()) => Ok(Some(id)),
        Some(id) => Err(format!(
            "name_color '{id}' is not a recognized color (expected one of: {})",
            AGENT_NAME_COLORS.join(", ")
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_none() {
        assert_eq!(validate_agent_name_color(None), Ok(None));
    }

    #[test]
    fn accepts_a_palette_id() {
        assert_eq!(
            validate_agent_name_color(Some("blue".to_string())),
            Ok(Some("blue".to_string()))
        );
    }

    #[test]
    fn rejects_an_unknown_id() {
        let result = validate_agent_name_color(Some("burnt-sienna".to_string()));
        assert!(result.is_err());
    }
}
