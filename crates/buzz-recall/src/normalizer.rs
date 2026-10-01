#![deny(unsafe_code)]
//! Normalizer for transcript turns into clean markdown with secret redaction.

use buzz_db::redactor::SecretRedactor;
use buzz_plugins::{SessionTurn, TurnRole};

/// Formats a sequence of session turns into a normalized Markdown document,
/// applying secret redaction to prevent credentials from leaking into storage or embeddings.
pub fn normalize_and_redact_transcript(turns: &[SessionTurn]) -> String {
    let mut out = String::new();

    for (idx, turn) in turns.iter().enumerate() {
        if idx > 0 {
            out.push_str("\n\n");
        }

        match turn.role {
            TurnRole::User => {
                out.push_str("### User\n");
                out.push_str(turn.content.trim());
            }
            TurnRole::Assistant => {
                out.push_str("### Assistant\n");
                if let Some(ref thinking) = turn.thinking {
                    if !thinking.trim().is_empty() {
                        out.push_str("> **Reasoning / Thought Process:**\n");
                        for line in thinking.lines() {
                            out.push_str("> ");
                            out.push_str(line);
                            out.push('\n');
                        }
                        out.push('\n');
                    }
                }
                out.push_str(turn.content.trim());

                for call in &turn.tool_calls {
                    out.push_str("\n\n#### Tool Invocation\n```json\n");
                    out.push_str(&serde_json::to_string_pretty(call).unwrap_or_default());
                    out.push_str("\n```");
                }
            }
            TurnRole::Tool => {
                out.push_str("### Tool Output\n");
                if !turn.content.trim().is_empty() {
                    out.push_str(turn.content.trim());
                }
                for res in &turn.tool_results {
                    out.push_str("\n```json\n");
                    out.push_str(&serde_json::to_string_pretty(res).unwrap_or_default());
                    out.push_str("\n```");
                }
            }
            TurnRole::System => {
                out.push_str("### System Note\n");
                out.push_str(turn.content.trim());
            }
        }
    }

    // Apply strict secret redaction to the entire assembled text
    SecretRedactor::redact(&out)
}

/// Extracts key architectural decisions or conclusions from session turns.
pub fn extract_session_decisions(turns: &[SessionTurn]) -> Vec<String> {
    let mut decisions = Vec::new();
    for turn in turns {
        if turn.role == TurnRole::Assistant {
            for line in turn.content.lines() {
                let trimmed = line.trim();
                if (trimmed.starts_with("- [x]") || trimmed.starts_with("Decision:") || trimmed.starts_with("Decided:"))
                    && trimmed.len() > 10
                {
                    decisions.push(SecretRedactor::redact(trimmed));
                }
            }
        }
    }
    decisions
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalizer_and_secret_redaction() {
        let turns = vec![
            SessionTurn::user("Please connect to AWS with key AKIA1234567890ABCDEF"),
            SessionTurn::assistant(
                "Connecting to AWS now.",
                Some("Using user-provided credentials to access S3 bucket.".to_string()),
            ),
        ];

        let markdown = normalize_and_redact_transcript(&turns);
        assert!(markdown.contains("### User"));
        assert!(markdown.contains("### Assistant"));
        assert!(markdown.contains("> **Reasoning / Thought Process:**"));
        assert!(!markdown.contains("AKIA1234567890ABCDEF"));
        assert!(markdown.contains("[REDACTED:AWS_ACCESS_KEY]"));
    }
}
