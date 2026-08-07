//! Canonical `buzz://` deep links for Buzz-hosted git entities.
//!
//! Buzz Desktop renders these links as rich preview cards in chat and
//! navigates in-app when they are clicked. The desktop parser lives in
//! `desktop/src/shared/lib/entityLink.ts` — the two implementations must
//! stay format-compatible (see `golden_format_matches_desktop` below and
//! the mirror test in `entityLink.test.mjs`).
//!
//! Callers are expected to validate inputs first (`validate_hex64`,
//! `validate_repo_id`); the identifier charsets need no URL encoding.

/// Build a `buzz://repo` link for a repository announcement (kind 30617).
pub fn repo_link(owner: &str, repo_id: &str) -> String {
    format!("buzz://repo?owner={owner}&d={repo_id}")
}

/// Build a `buzz://pr` link for a pull request event (kind 1618).
pub fn pull_request_link(event_id: &str, owner: &str, repo_id: &str) -> String {
    format!("buzz://pr?id={event_id}&owner={owner}&d={repo_id}")
}

/// Build a `buzz://issue` link for an issue event (kind 1621).
pub fn issue_link(event_id: &str, owner: &str, repo_id: &str) -> String {
    format!("buzz://issue?id={event_id}&owner={owner}&d={repo_id}")
}

#[cfg(test)]
mod tests {
    use super::*;

    const OWNER: &str = "71d67180ba17e749ee825fc8819c9c6ee7003617e1c126504f9b658070ab9224";
    const EVENT_ID: &str = "c3b589fa5713ba25bad6dc095e2de00a4ac8f50050fdea00fc6444e603be1dd1";

    // Golden strings shared with desktop/src/shared/lib/entityLink.test.mjs
    // ("builders emit the canonical cross-language link format").
    #[test]
    fn golden_format_matches_desktop() {
        assert_eq!(
            pull_request_link(EVENT_ID, OWNER, "buzz-world"),
            format!("buzz://pr?id={EVENT_ID}&owner={OWNER}&d=buzz-world")
        );
        assert_eq!(
            issue_link(EVENT_ID, OWNER, "buzz-world"),
            format!("buzz://issue?id={EVENT_ID}&owner={OWNER}&d=buzz-world")
        );
        assert_eq!(
            repo_link(OWNER, "buzz-world"),
            format!("buzz://repo?owner={OWNER}&d=buzz-world")
        );
    }
}
