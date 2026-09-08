//! Executable-text concealment gate at the 30178 catalog boundary (Carl P1).
//!
//! A member display name/prompt, name-pool entry, and the team instructions
//! are copied verbatim into local stores and delivered to the ACP harness
//! (`BUZZ_ACP_SYSTEM_PROMPT` / `BUZZ_ACP_TEAM_INSTRUCTIONS`). A signed, shared
//! head could otherwise smuggle invisible or bidi-override characters into that
//! executable configuration, making what runs differ from the reviewed text.
//! `validate_team_catalog_content` is the single chokepoint both publish
//! (`build_team_catalog_content`) and adopt (`team_catalog_content_from_event`)
//! pass through, so one gate covers both directions.

use super::super::{build_team_catalog_content, team_catalog_content_from_event};
use super::{member, signed_event_with_content, team};

#[test]
fn test_concealed_executable_text_is_rejected_at_the_catalog_boundary() {
    for (label, body) in [
        (
            "default-ignorable in member display name",
            r#"{"v":1,"name":"T","members":[{"member_key":"k","display_name":"Review\u200Ber","system_prompt":"Do the work."}]}"#,
        ),
        (
            "bidi override in member prompt",
            r#"{"v":1,"name":"T","members":[{"member_key":"k","display_name":"One","system_prompt":"Run\u2066hidden"}]}"#,
        ),
        (
            "bidi override in name-pool entry",
            r#"{"v":1,"name":"T","members":[{"member_key":"k","display_name":"One","name_pool":["Al\u202Eias"]}]}"#,
        ),
        (
            "bidi override in team instructions",
            r#"{"v":1,"name":"T","instructions":"Ignore\u202E all review","members":[{"member_key":"k","display_name":"One"}]}"#,
        ),
        (
            "default-ignorable in team name",
            r#"{"v":1,"name":"Sq\u200Buad","members":[{"member_key":"k","display_name":"One"}]}"#,
        ),
        (
            "bidi override in team description",
            r#"{"v":1,"name":"T","description":"Trusted\u202E reviewers","members":[{"member_key":"k","display_name":"One"}]}"#,
        ),
    ] {
        assert!(
            team_catalog_content_from_event(&signed_event_with_content(body)).is_err(),
            "{label} must be refused at the parse boundary"
        );
    }
}

#[test]
fn test_visible_executable_text_still_passes_the_catalog_boundary() {
    // The gate must not reject legitimate teams: an emoji-bearing display name
    // (the validator allows VS16/ZWJ emoji sequences) and multiline
    // instructions (layout controls allowed) are within the contract.
    let body = "{\"v\":1,\"name\":\"T\",\"instructions\":\"Line one.\\nLine two.\",\"members\":[{\"member_key\":\"k\",\"display_name\":\"Shipwright \u{1F6E5}\u{FE0F}\",\"system_prompt\":\"Do the work.\\n\\tCarefully.\",\"name_pool\":[\"Ada\"]}]}";
    assert!(
        team_catalog_content_from_event(&signed_event_with_content(body)).is_ok(),
        "a visible emoji name plus multiline instructions is within the contract"
    );
}

#[test]
fn test_publisher_side_refuses_concealed_executable_text() {
    // `build_team_catalog_content` shares the same chokepoint, so a locally
    // corrupted definition fails the share attempt synchronously.
    let mut m = member("m1", "One");
    m.system_prompt = "Run\u{2066}hidden".to_string();
    assert!(
        build_team_catalog_content(&team(), &[m]).is_err(),
        "a member prompt with a bidi override must fail publication"
    );
}
