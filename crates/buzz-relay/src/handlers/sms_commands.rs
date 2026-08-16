//! SMS routing commands — `kind:9046` ([`KIND_SMS_SET_ROUTE`]).
//!
//! Closes the loop the sms-operator persona could previously only open. The
//! persona can ask "which project?", but before this there was no code path
//! anywhere that persisted the answer: `sms_identities.default_project` was
//! read by [`crate::api::sms`] and written by nothing, so a sender was asked
//! the same question forever and the operator had to reach for raw SQL.
//!
//! Processed like the 9040-series moderation commands: validated and executed
//! directly, **never stored and never fanned out**. That is a privacy
//! requirement rather than an optimisation — the command carries a
//! subscriber's phone number, and persisting it as an ordinary event would
//! republish that number to every subscriber of the channel and leave it
//! queryable indefinitely.
//!
//! ## Authorization
//!
//! The author must be an active member of the configured SMS-inbox channel.
//! That channel is private and hand-provisioned (its owner has to
//! `channels add-member` each participant), so membership is the narrowest
//! existing signal for "is allowed to operate the SMS bridge" — and it is
//! exactly the set the operator agent already belongs to.
//!
//! The boundary this draws is deliberate and worth stating plainly: any member
//! of the inbox channel can re-route **any** number registered in the
//! community, not just their own. Re-routing is a real capability — it decides
//! which repository a sender's texts dispatch an agent against — so widening
//! the channel's membership widens this too. Per-number ownership (only the
//! pubkey in `linked_pubkey` may re-route its own number) is the obvious next
//! tightening, but `linked_pubkey` is unpopulated in practice today, so
//! enforcing it now would lock everyone out instead of protecting anyone.

use std::sync::Arc;

use buzz_core::tenant::TenantContext;
use tracing::info;

use crate::state::AppState;

/// Tag carrying the E.164 number to route. Same tag name the inbound webhook
/// stamps on synthesized events, so the two halves stay greppable together.
const TAG_PHONE: &str = "sms_from";
/// Tag carrying the NIP-MP project `d`-tag to route to.
const TAG_PROJECT: &str = "project";

fn tag_value<'a>(event: &'a nostr::Event, name: &str) -> Option<&'a str> {
    event
        .tags
        .iter()
        .find(|t| t.as_slice().first().map(String::as_str) == Some(name))
        .and_then(|t| t.as_slice().get(1))
        .map(String::as_str)
}

/// A validated routing command: which number, and what to route it to.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct RouteCommand {
    pub phone_number: String,
    /// `None` clears the route, returning the number to the "ask which
    /// project" state. An empty `project` tag means the same thing — the
    /// difference between "absent" and "empty string" is not one a caller
    /// should have to think about.
    pub project: Option<String>,
}

/// Parse and validate a `kind:9046` command's tags.
///
/// Rejects rather than silently normalizing a bad number: routing the wrong
/// number is worse than refusing an ambiguous one, and a typo'd `To` here
/// would send someone else's agent output to a stranger's handset.
pub(crate) fn parse_route_command(event: &nostr::Event) -> Result<RouteCommand, String> {
    let Some(phone_number) = tag_value(event, TAG_PHONE) else {
        return Err(format!("invalid: missing {TAG_PHONE} tag"));
    };
    let phone_number = phone_number.trim();
    if !is_e164(phone_number) {
        return Err(format!(
            "invalid: {TAG_PHONE} must be an E.164 number like +15551234567"
        ));
    }

    let project = tag_value(event, TAG_PROJECT)
        .map(str::trim)
        .filter(|p| !p.is_empty());
    if let Some(project) = project {
        if !is_valid_project_slug(project) {
            return Err(
                "invalid: project must be 1-64 chars of [a-zA-Z0-9._-] with no whitespace"
                    .to_string(),
            );
        }
    }

    Ok(RouteCommand {
        phone_number: phone_number.to_string(),
        project: project.map(str::to_string),
    })
}

/// `+` followed by 7–15 digits. Deliberately strict: the inbound webhook
/// stores exactly what Twilio sends (always E.164), so anything else would
/// never match a row anyway and is far more likely a mistake than an intent.
fn is_e164(value: &str) -> bool {
    let Some(digits) = value.strip_prefix('+') else {
        return false;
    };
    (7..=15).contains(&digits.len()) && digits.bytes().all(|b| b.is_ascii_digit())
}

/// Project slugs become a NIP-MP `d`-tag and are surfaced to the persona, so
/// keep them to an unambiguous character set.
fn is_valid_project_slug(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

/// Validate, authorize, and execute a `kind:9046` routing command.
///
/// `Err` is a rejection message returned to the submitter, matching the
/// moderation-command handler's contract.
pub(crate) async fn handle_sms_command(
    tenant: &TenantContext,
    state: &Arc<AppState>,
    event: &nostr::Event,
) -> Result<(), String> {
    let Some(inbox_channel) = state.config.twilio_sms_inbox_channel else {
        return Err("restricted: SMS bridge is not configured on this relay".to_string());
    };

    // Authorize before parsing, so an unauthorized caller cannot use the
    // difference between "invalid tags" and "not permitted" to probe which
    // numbers are registered.
    let author = event.pubkey.to_bytes();
    let is_member = state
        .db
        .is_member(tenant.community(), inbox_channel, &author)
        .await
        .map_err(|e| {
            tracing::warn!(error = %e, "sms_commands: membership lookup failed");
            "error: could not verify SMS bridge membership".to_string()
        })?;
    if !is_member {
        return Err("restricted: not a member of the SMS inbox channel".to_string());
    }

    let command = parse_route_command(event)?;

    let updated = state
        .db
        .set_sms_default_project(
            tenant.community(),
            &command.phone_number,
            command.project.as_deref(),
        )
        .await
        .map_err(|e| {
            tracing::warn!(error = %e, "sms_commands: default_project update failed");
            "error: could not update SMS route".to_string()
        })?;

    if !updated {
        // Same wording whether the number is unknown or belongs to another
        // community — this must not become an oracle for which numbers are
        // allow-listed, the same property `api::sms` maintains on the way in.
        return Err("invalid: no such allow-listed number in this community".to_string());
    }

    // The number itself is intentionally absent from the log line: this is a
    // routine operation and the relay's logs are a much wider audience than
    // the `sms_identities` table.
    info!(
        project = command.project.as_deref().unwrap_or("<cleared>"),
        "sms_commands: updated SMS default project route"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn command_event(tags: Vec<Vec<&str>>) -> nostr::Event {
        let keys = nostr::Keys::generate();
        let tags: Vec<nostr::Tag> = tags
            .into_iter()
            .map(|t| nostr::Tag::parse(t).expect("valid tag"))
            .collect();
        nostr::EventBuilder::new(
            nostr::Kind::from(buzz_core::kind::KIND_SMS_SET_ROUTE as u16),
            "",
        )
        .tags(tags)
        .sign_with_keys(&keys)
        .expect("sign test event")
    }

    #[test]
    fn parses_a_set_route_command() {
        let event = command_event(vec![
            vec!["sms_from", "+18655550123"],
            vec!["project", "bidcraft"],
        ]);
        assert_eq!(
            parse_route_command(&event).expect("should parse"),
            RouteCommand {
                phone_number: "+18655550123".to_string(),
                project: Some("bidcraft".to_string()),
            }
        );
    }

    #[test]
    fn absent_project_tag_clears_the_route() {
        let event = command_event(vec![vec!["sms_from", "+18655550123"]]);
        assert_eq!(
            parse_route_command(&event).expect("should parse").project,
            None
        );
    }

    /// An empty tag value is how a client naturally expresses "unset this";
    /// it must mean the same thing as omitting the tag, not store `""` as a
    /// project name that no dispatch will ever match.
    #[test]
    fn empty_project_tag_clears_the_route() {
        for empty in ["", "   "] {
            let event = command_event(vec![
                vec!["sms_from", "+18655550123"],
                vec!["project", empty],
            ]);
            assert_eq!(
                parse_route_command(&event).expect("should parse").project,
                None,
                "project tag {empty:?} should clear rather than store"
            );
        }
    }

    #[test]
    fn rejects_a_missing_phone_tag() {
        let event = command_event(vec![vec!["project", "bidcraft"]]);
        assert!(parse_route_command(&event)
            .expect_err("should reject")
            .contains("missing sms_from"));
    }

    #[test]
    fn rejects_non_e164_numbers() {
        // Routing the wrong number sends one person's agent output to another
        // person's handset, so anything ambiguous is refused outright.
        for bad in [
            "8655550123",        // no country code
            "+1 865 555 0123",   // spaces
            "+1-865-555-0123",   // dashes
            "+18655550123x99",   // extension
            "+123456",           // too short
            "+1234567890123456", // too long
            "++18655550123",
            "",
        ] {
            let event = command_event(vec![vec!["sms_from", bad]]);
            assert!(
                parse_route_command(&event).is_err(),
                "{bad:?} must not be accepted as a phone number"
            );
        }
    }

    #[test]
    fn accepts_surrounding_whitespace_on_the_number() {
        let event = command_event(vec![vec!["sms_from", "  +18655550123  "]]);
        assert_eq!(
            parse_route_command(&event)
                .expect("should parse")
                .phone_number,
            "+18655550123"
        );
    }

    #[test]
    fn rejects_project_slugs_that_are_not_plain_identifiers() {
        for bad in [
            "has space",
            "semi;colon",
            "quote'd",
            "../escape",
            &"x".repeat(65),
        ] {
            let event = command_event(vec![vec!["sms_from", "+18655550123"], vec!["project", bad]]);
            assert!(
                parse_route_command(&event).is_err(),
                "{bad:?} must not be accepted as a project slug"
            );
        }
    }

    #[test]
    fn accepts_ordinary_project_slugs() {
        for good in ["bidcraft", "construct-pro", "buzz_v2", "repo.name", "a"] {
            let event = command_event(vec![
                vec!["sms_from", "+18655550123"],
                vec!["project", good],
            ]);
            assert_eq!(
                parse_route_command(&event)
                    .expect("should parse")
                    .project
                    .as_deref(),
                Some(good)
            );
        }
    }
}
