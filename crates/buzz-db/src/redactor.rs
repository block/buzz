#![deny(unsafe_code)]
//! Secret redaction engine for Orbit memory storage.
//!
//! Prevents API keys, access tokens, private keys, and environment secrets
//! from being stored into persistent SQLite, vector, or graph storage.

/// Known token prefixes to redact.
const TOKEN_PREFIXES: &[(&str, &str)] = &[
    ("sk-", "OPENAI_KEY"),
    ("ghp_", "GITHUB_PAT"),
    ("gho_", "GITHUB_OAUTH"),
    ("ghu_", "GITHUB_USER"),
    ("ghs_", "GITHUB_SERVER"),
    ("ghr_", "GITHUB_REFRESH"),
    ("github_pat_", "GITHUB_FINE_GRAINED_PAT"),
    ("AKIA", "AWS_ACCESS_KEY"),
    ("ASIA", "AWS_TEMP_KEY"),
    ("xoxb-", "SLACK_BOT_TOKEN"),
    ("xoxp-", "SLACK_USER_TOKEN"),
    ("xoxa-", "SLACK_APP_TOKEN"),
    ("xoxs-", "SLACK_SESSION_TOKEN"),
    ("xoxr-", "SLACK_REFRESH_TOKEN"),
    ("nsec1", "NOSTR_PRIVATE_KEY"),
    ("sprt_tok_", "SPROUT_TOKEN"),
];

/// Redacts secrets, tokens, private keys, and sensitive credentials from input text.
pub struct SecretRedactor;

impl SecretRedactor {
    /// Redact known token types and private key blocks from text.
    pub fn redact(input: &str) -> String {
        Self::redact_with_extras(input, &[])
    }

    /// Redact known token types, private key blocks, and verbatim extra secret strings.
    pub fn redact_with_extras(input: &str, extras: &[&str]) -> String {
        let mut result = input.to_string();

        // 1. Scrub extra verbatim secrets (e.g. from environment variables)
        let mut sorted_extras: Vec<&str> = extras
            .iter()
            .copied()
            .filter(|s| s.trim().len() >= 4)
            .collect();
        // Sort descending by length to prevent partial scrubbing of longer substrings
        sorted_extras.sort_by(|a, b| b.len().cmp(&a.len()));

        for extra in sorted_extras {
            result = result.replace(extra, "[REDACTED:USER_SECRET]");
        }

        // 2. Scrub PEM private key blocks
        result = Self::scrub_pem_blocks(&result);

        // 3. Scrub known token prefixes
        for (prefix, label) in TOKEN_PREFIXES {
            result = Self::scrub_token_prefix(&result, prefix, label);
        }

        // 4. Scrub Bearer JWT tokens
        result = Self::scrub_bearer_tokens(&result);

        result
    }

    /// Checks if the text contains any recognizable secret.
    pub fn contains_secrets(input: &str) -> bool {
        if input.contains("-----BEGIN") && input.contains("PRIVATE KEY-----") {
            return true;
        }

        for (prefix, _) in TOKEN_PREFIXES {
            if let Some(pos) = input.find(prefix) {
                // Verify it's followed by at least 10 valid token chars
                let rest = &input[pos + prefix.len()..];
                if rest.chars().take(8).all(|c| c.is_alphanumeric() || c == '_' || c == '-') {
                    return true;
                }
            }
        }

        false
    }

    fn scrub_pem_blocks(input: &str) -> String {
        let mut out = String::with_capacity(input.len());
        let mut cursor = input;

        while let Some(start_idx) = cursor.find("-----BEGIN") {
            out.push_str(&cursor[..start_idx]);
            let tail = &cursor[start_idx..];

            if let Some(end_idx) = tail.find("-----END") {
                if let Some(after_end) = tail[end_idx..].find("-----\n").or_else(|| tail[end_idx..].find("-----")) {
                    let total_end = end_idx + after_end + 5;
                    out.push_str("[REDACTED:PRIVATE_KEY]");
                    cursor = &tail[total_end.min(tail.len())..];
                    continue;
                }
            }
            out.push_str(&tail[..5]);
            cursor = &tail[5..];
        }

        out.push_str(cursor);
        out
    }

    fn scrub_token_prefix(input: &str, prefix: &str, label: &str) -> String {
        let mut out = String::with_capacity(input.len());
        let mut cursor = input;

        while let Some(pos) = cursor.find(prefix) {
            out.push_str(&cursor[..pos]);
            let tail = &cursor[pos..];

            // Scan until whitespace, quote, or punctuation terminator
            let token_len = tail
                .find(|c: char| c.is_whitespace() || c == '"' || c == '\'' || c == '`' || c == ',' || c == ';' || c == ')' || c == '}')
                .unwrap_or(tail.len());

            if token_len >= prefix.len() + 6 {
                out.push_str(&format!("[REDACTED:{label}]"));
            } else {
                out.push_str(&tail[..token_len]);
            }

            cursor = &tail[token_len..];
        }

        out.push_str(cursor);
        out
    }

    fn scrub_bearer_tokens(input: &str) -> String {
        let mut out = String::with_capacity(input.len());
        let mut cursor = input;

        while let Some(pos) = cursor.find("Bearer ") {
            out.push_str(&cursor[..pos]);
            out.push_str("Bearer ");
            let tail = &cursor[pos + 7..];

            // If it starts with eyJ (standard JWT prefix)
            if tail.starts_with("eyJ") {
                let token_len = tail
                    .find(|c: char| c.is_whitespace() || c == '"' || c == '\'' || c == '`')
                    .unwrap_or(tail.len());
                out.push_str("[REDACTED:JWT]");
                cursor = &tail[token_len..];
            } else {
                cursor = tail;
            }
        }

        out.push_str(cursor);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_redact_openai_key() {
        let text = "My key is sk-proj1234567890abcdef123456 in config";
        let redacted = SecretRedactor::redact(text);
        assert_eq!(redacted, "My key is [REDACTED:OPENAI_KEY] in config");
        assert!(SecretRedactor::contains_secrets(text));
    }

    #[test]
    fn test_redact_github_pat() {
        let text = "token: ghp_1234567890abcdefghijklmnopqrstuvwxyz";
        let redacted = SecretRedactor::redact(text);
        assert_eq!(redacted, "token: [REDACTED:GITHUB_PAT]");
    }

    #[test]
    fn test_redact_nostr_nsec() {
        let text = "export BUZZ_PRIVATE_KEY=nsec1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqq";
        let redacted = SecretRedactor::redact(text);
        assert_eq!(redacted, "export BUZZ_PRIVATE_KEY=[REDACTED:NOSTR_PRIVATE_KEY]");
    }

    #[test]
    fn test_redact_pem_block() {
        let text = "Here is key:\n-----BEGIN RSA PRIVATE KEY-----\nMIIEowIBAAKCAQEA...\n-----END RSA PRIVATE KEY-----\nDone";
        let redacted = SecretRedactor::redact(text);
        assert!(redacted.contains("[REDACTED:PRIVATE_KEY]"));
        assert!(!redacted.contains("MIIEow"));
    }

    #[test]
    fn test_redact_extras() {
        let text = "connection string: postgres://user:super_secret_password_123@db.internal:5432";
        let redacted = SecretRedactor::redact_with_extras(text, &["super_secret_password_123"]);
        assert!(redacted.contains("[REDACTED:USER_SECRET]"));
        assert!(!redacted.contains("super_secret_password_123"));
    }
}
