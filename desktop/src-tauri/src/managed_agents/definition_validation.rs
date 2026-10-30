//! Validation for human-reviewed agent definition text. The rules live in
//! `buzz_core::definition_validation` so the CLI applies the same checks.

pub(crate) use buzz_core_pkg::definition_validation::{
    is_portable_acp_command, validate_agent_definition_text, validate_agent_description_text,
    validate_managed_agent_definition_text, validate_portable_acp_command,
};
