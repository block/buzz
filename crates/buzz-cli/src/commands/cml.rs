//! Local CML validation and canonicalization commands.

use std::{fs, io::Read};

use crate::error::CliError;

/// Validate CML text without performing I/O.
pub fn validate_input(input: &str) -> Result<(), CliError> {
    buzz_core::cml::parse_cml(input)
        .map(|_| ())
        .map_err(|error| CliError::Usage(error.to_string()))
}

/// Parse and return canonical CML without performing I/O.
pub fn canonicalize_input(input: &str) -> Result<String, CliError> {
    buzz_core::cml::parse_cml(input)
        .and_then(|task| task.to_canonical_json())
        .map_err(|error| CliError::Usage(error.to_string()))
}

/// Run `buzz cml validate <path|->` locally.
pub fn cmd_validate(path: &str) -> Result<(), CliError> {
    let input = read_input(path)?;
    validate_input(&input)?;
    println!("Valid.");
    Ok(())
}

/// Run `buzz cml canonicalize <path|-> [--output <path>]` locally.
pub fn cmd_canonicalize(path: &str, output: Option<&str>) -> Result<(), CliError> {
    let input = read_input(path)?;
    let canonical = canonicalize_input(&input)?;
    if let Some(output_path) = output {
        fs::write(output_path, canonical)
            .map_err(|error| CliError::Other(format!("failed to write {output_path}: {error}")))?;
    } else {
        print!("{canonical}");
    }
    Ok(())
}

fn read_input(path: &str) -> Result<String, CliError> {
    if path == "-" {
        let mut input = String::new();
        std::io::stdin()
            .read_to_string(&mut input)
            .map_err(|error| CliError::Other(format!("failed to read stdin: {error}")))?;
        return Ok(input);
    }
    fs::read_to_string(path)
        .map_err(|error| CliError::Usage(format!("failed to read {path}: {error}")))
}

#[cfg(test)]
mod tests {
    use super::{canonicalize_input, validate_input};

    const VALID: &str = r#"{
      "acceptance": [], "blockers": [], "evidence": [],
      "git": {"base_sha":"1111111111111111111111111111111111111111","branch":"feat/cml","head_sha":null,"repo":"block/buzz","worktree_alias":"buzz-cml"},
      "id":"cdd4722d-7481-4d01-9c0a-423b4454c179","lease":null,
      "objective":"One outcome","priority":"P1","protocol":"buzz-cml",
      "review":{"max_rounds":3,"round":0},
      "roles":{"fixer":null,"planner":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","reviewer":null,"worker":null},
      "runtime":{"host_id":null,"last_heartbeat_at":null,"presence":"offline","ttl_seconds":180},
      "status":"proposed","title":"CML","updated_at":1787673000,"version":1
    }"#;

    #[test]
    fn local_validate_accepts_valid_cml_and_rejects_unknown_fields() {
        validate_input(VALID).expect("valid CML");
        let invalid = VALID.replace("\"version\":1", "\"version\":1,\"surprise\":true");
        assert!(validate_input(&invalid).is_err());
    }

    #[test]
    fn canonicalize_is_a_byte_stable_fixed_point() {
        let first = canonicalize_input(VALID).expect("canonicalize");
        let second = canonicalize_input(&first).expect("canonicalize again");
        assert_eq!(first, second);
        assert!(first.ends_with('\n'));
    }
}
