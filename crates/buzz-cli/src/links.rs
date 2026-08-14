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
//!
//! Coordinate links additionally accept an optional `&tab=<tab>` parameter
//! (`files|commits|issues|prs|contributors|channels`) selecting a workspace tab on
//! the receiving side. The CLI builders emit the canonical no-tab form
//! (overview); the parameter exists for the desktop's tab-aware copy-link
//! button.

/// Whether a d-tag can be expressed in a `buzz://` link.
///
/// Project slugs accept up to 1024 bytes of arbitrary UTF-8, but the link
/// format is restricted to `[a-zA-Z0-9._-]{1,64}` (no leading dot, no `..`)
/// so links need no escaping and stay safe to paste. Callers must check
/// before building a link and omit the field when it returns false, rather
/// than emitting a link no client can parse. Mirrors `isValidDtag` in
/// `desktop/src/shared/lib/entityLink.ts`.
pub fn is_linkable_dtag(dtag: &str) -> bool {
    !dtag.is_empty()
        && dtag.len() <= 64
        && !dtag.starts_with('.')
        && !dtag.contains("..")
        && dtag
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-')
}

/// Build a `buzz://repo` link for a repository announcement (kind 30617).
pub fn repo_link(owner: &str, repo_id: &str) -> String {
    format!("buzz://repo?owner={owner}&d={repo_id}")
}

/// Build a `buzz://project` link for a project announcement (kind 30621).
pub fn project_link(owner: &str, project_id: &str) -> String {
    format!("buzz://project?owner={owner}&d={project_id}")
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
    use serde_json::Value;

    fn golden() -> Value {
        serde_json::from_str(include_str!("../../../test-fixtures/entity-links.json"))
            .expect("valid entity-links golden fixture")
    }

    #[test]
    fn golden_format_matches_desktop() {
        let golden = golden();
        let owner = golden["owner"].as_str().unwrap();
        let event_id = golden["eventId"].as_str().unwrap();
        let dtag = golden["dtag"].as_str().unwrap();
        assert_eq!(
            pull_request_link(event_id, owner, dtag),
            golden["links"]["pullRequest"].as_str().unwrap()
        );
        assert_eq!(
            issue_link(event_id, owner, dtag),
            golden["links"]["issue"].as_str().unwrap()
        );
        assert_eq!(
            repo_link(owner, dtag),
            golden["links"]["repository"].as_str().unwrap()
        );
        assert_eq!(
            project_link(owner, dtag),
            golden["links"]["project"].as_str().unwrap()
        );
    }

    #[test]
    fn linkable_dtag_matches_the_desktop_charset() {
        let golden = golden();
        for ok in golden["validDtags"].as_array().unwrap() {
            let ok = ok.as_str().unwrap();
            assert!(is_linkable_dtag(ok), "{ok:?} should be linkable");
        }
        assert!(is_linkable_dtag(&"a".repeat(64)));
        for bad in golden["invalidDtags"].as_array().unwrap() {
            let bad = bad.as_str().unwrap();
            assert!(!is_linkable_dtag(bad), "{bad:?} should not be linkable");
        }
        assert!(!is_linkable_dtag(&"a".repeat(65)));
    }
}
