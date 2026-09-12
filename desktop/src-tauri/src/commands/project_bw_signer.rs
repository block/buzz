//! Narrow signer selection for BW transitions that the assigned writer, not
//! the coordinator viewing the issue, must authorize.

use super::project_git_workflow::normalize_event_id;
use serde_json::Value;

pub(super) fn explicit_writer_signer(
    record: &str,
    content: &Value,
    signer_pubkey: Option<&str>,
) -> Result<Option<String>, String> {
    let signer = signer_pubkey
        .map(|value| {
            normalize_event_id(value).ok_or_else(|| "Invalid BW signer public key.".to_string())
        })
        .transpose()?;
    if signer.is_some()
        && !(record == "issue-state"
            && matches!(
                content.get("state").and_then(Value::as_str),
                Some("in-development" | "implemented")
            ))
    {
        return Err(
            "An explicit BW signer is only allowed for writer-owned state transitions.".to_string(),
        );
    }
    Ok(signer)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn explicit_signers_are_narrowly_limited_to_writer_owned_transitions() {
        let writer = "D".repeat(64);
        assert_eq!(
            explicit_writer_signer(
                "issue-state",
                &json!({"state":"in-development"}),
                Some(&writer),
            )
            .expect("writer signer"),
            Some("d".repeat(64)),
        );
        assert!(explicit_writer_signer(
            "issue-state",
            &json!({"state":"implemented"}),
            Some(&writer),
        )
        .is_ok());
        assert!(
            explicit_writer_signer("issue-state", &json!({"state":"ready"}), Some(&writer),)
                .is_err()
        );
        assert!(explicit_writer_signer(
            "issue-relation",
            &json!({"operation":"add"}),
            Some(&writer),
        )
        .is_err());
        assert!(explicit_writer_signer(
            "issue-state",
            &json!({"state":"in-development"}),
            Some("not-a-pubkey"),
        )
        .is_err());
    }
}
