//! Which Huddle members are agents.
//!
//! The channel role `bot` is the first signal. The relay stores it only for
//! keys with a recorded NIP-OA owner, so an agent without an owner
//! attestation joins as `member`. Desktop still knows such an agent when a
//! trusted owner published a kind:30177 managed-agent record for it. Desktop
//! publishes that record for every agent it manages, from every machine.
//!
//! Trusted record authors are the current user and the Huddle channel's
//! owners. A record from anyone else is only that author's claim, so it does
//! not make a member an agent. This is a Huddle compatibility policy, not
//! identity proof: a record mints no owner. It does let an owner mark a
//! member as an agent for every Huddle behavior that reads this roster:
//! spoken text, the voice publisher, remote transcription and floor
//! arbitration (which skip agent audio), cleanup, and the last-person check.
//! An owner already had that power when the client chose the `bot` role.
//!
//! A participant who does not own the channel can still add an agent without
//! NIP-OA. Only that participant's own machines recognize it.

use std::collections::HashSet;

use nostr::Event;

use super::relay_api::fetch_channel_members_with_roles;
use crate::{app_state::AppState, nostr_convert, relay::query_relay};

const KIND_MANAGED_AGENT: u16 = 30177;

/// Huddle channel members, split into agents and people.
///
/// When the managed-agent records cannot be read, `complete` is false. Then
/// `agents` holds only the members proven by role `bot`, and `humans` can
/// include agents without NIP-OA.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct HuddleRoster {
    pub(crate) agents: Vec<String>,
    pub(crate) humans: Vec<String>,
    pub(crate) complete: bool,
}

impl HuddleRoster {
    /// Update a stored agent list from this roster. Returns true when the
    /// list changed.
    ///
    /// A complete roster replaces the list. An incomplete roster adds its
    /// role-proven agents and keeps stored agents that are still members, so
    /// a failed record read can neither hide a `bot` agent nor drop an agent
    /// that only a record proved. Stored agents that left the channel drop out.
    pub(crate) fn apply_agents(self, current: &mut Vec<String>) -> bool {
        let next = if self.complete {
            self.agents
        } else {
            let mut next: Vec<String> = current
                .iter()
                .filter(|pubkey| self.agents.contains(pubkey) || self.humans.contains(pubkey))
                .cloned()
                .collect();
            for agent in self.agents {
                if !next.contains(&agent) {
                    next.push(agent);
                }
            }
            next
        };
        if *current == next {
            return false;
        }
        *current = next;
        true
    }

    /// The number of people, only when every member was classified. An
    /// incomplete roster can count an agent as a person, so it must not
    /// decide that the Huddle can end.
    pub(crate) fn humans_remaining(&self) -> Option<usize> {
        self.complete.then_some(self.humans.len())
    }
}

/// Read the Huddle channel's members and decide which are agents.
pub(crate) async fn fetch_huddle_roster(
    channel_id: &str,
    state: &AppState,
) -> Result<HuddleRoster, String> {
    let members = fetch_channel_members_with_roles(channel_id, state).await?;
    let viewer = state
        .keys
        .lock()
        .map(|keys| keys.public_key().to_hex())
        .map_err(|error| error.to_string())?;
    let authors = trusted_record_authors(&members, &viewer);
    let candidates = record_candidates(&members);
    let records = if candidates.is_empty() {
        Ok(Vec::new())
    } else {
        query_relay(
            state,
            &[serde_json::json!({
                "kinds": [KIND_MANAGED_AGENT],
                "authors": authors,
                "#d": candidates,
                "limit": authors.len() * candidates.len(),
            })],
        )
        .await
    };
    Ok(roster_from(&members, records, &authors))
}

/// Classify members. A failed record read keeps the role-proven agents and
/// marks the roster incomplete.
fn roster_from(
    members: &[(String, Option<String>)],
    records: Result<Vec<Event>, String>,
    trusted_authors: &[String],
) -> HuddleRoster {
    match records {
        Ok(records) => classify(members, &records, trusted_authors),
        Err(error) => {
            eprintln!("buzz-desktop: Huddle managed-agent query failed: {error}");
            HuddleRoster {
                complete: false,
                ..classify(members, &[], trusted_authors)
            }
        }
    }
}

/// The current user and the channel's owners, normalized and deduplicated.
fn trusted_record_authors(members: &[(String, Option<String>)], viewer: &str) -> Vec<String> {
    let mut authors = vec![viewer.to_ascii_lowercase()];
    for (pubkey, role) in members {
        let pubkey = pubkey.to_ascii_lowercase();
        if role.as_deref() == Some("owner") && !authors.contains(&pubkey) {
            authors.push(pubkey);
        }
    }
    authors
}

/// Members without the `bot` role. Only these need a managed-agent record.
fn record_candidates(members: &[(String, Option<String>)]) -> Vec<String> {
    members
        .iter()
        .filter(|(_, role)| role.as_deref() != Some("bot"))
        .map(|(pubkey, _)| pubkey.to_ascii_lowercase())
        .collect()
}

fn classify(
    members: &[(String, Option<String>)],
    records: &[Event],
    trusted_authors: &[String],
) -> HuddleRoster {
    let trusted: Vec<Event> = records
        .iter()
        .filter(|event| {
            trusted_authors
                .iter()
                .any(|author| author.eq_ignore_ascii_case(&event.pubkey.to_hex()))
        })
        .cloned()
        .collect();
    let managed: HashSet<String> = nostr_convert::managed_agent_pubkeys_from_events(&trusted);
    let mut roster = HuddleRoster {
        complete: true,
        ..HuddleRoster::default()
    };
    for (pubkey, role) in members {
        if role.as_deref() == Some("bot") || managed.contains(&pubkey.to_ascii_lowercase()) {
            roster.agents.push(pubkey.clone());
        } else {
            roster.humans.push(pubkey.clone());
        }
    }
    roster
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{EventBuilder, JsonUtil, Keys, Kind, Tag};

    fn record(owner: &Keys, agent: &str) -> Event {
        EventBuilder::new(Kind::Custom(KIND_MANAGED_AGENT), "{}")
            .tags([Tag::parse(["d", agent]).unwrap()])
            .sign_with_keys(owner)
            .unwrap()
    }

    fn member(pubkey: &str, role: &str) -> (String, Option<String>) {
        (pubkey.to_string(), Some(role.to_string()))
    }

    #[test]
    fn bot_role_and_trusted_records_mark_agents() {
        let viewer = Keys::generate();
        let creator = Keys::generate();
        let stranger = Keys::generate();
        let human = Keys::generate().public_key().to_hex();
        let attested = Keys::generate().public_key().to_hex();
        let mine = Keys::generate().public_key().to_hex();
        let creators = Keys::generate().public_key().to_hex();
        let claimed = Keys::generate().public_key().to_hex();
        let members = vec![
            member(&creator.public_key().to_hex(), "owner"),
            member(&viewer.public_key().to_hex(), "member"),
            member(&human, "member"),
            member(&attested, "bot"),
            member(&mine, "member"),
            member(&creators, "member"),
            member(&claimed, "member"),
            (stranger.public_key().to_hex(), None),
        ];
        let authors = trusted_record_authors(&members, &viewer.public_key().to_hex());
        let records = vec![
            record(&viewer, &mine),
            record(&creator, &creators),
            // A member who does not own the channel cannot vouch.
            record(&stranger, &claimed),
        ];

        let roster = classify(&members, &records, &authors);

        assert_eq!(roster.agents, vec![attested, mine, creators]);
        assert_eq!(
            roster.humans,
            vec![
                creator.public_key().to_hex(),
                viewer.public_key().to_hex(),
                human,
                claimed,
                stranger.public_key().to_hex(),
            ]
        );
    }

    #[test]
    fn untrusted_or_forged_records_are_ignored() {
        let viewer = Keys::generate();
        let other = Keys::generate();
        let agent = Keys::generate().public_key().to_hex();
        let members = vec![member(&agent, "member")];
        let authors = trusted_record_authors(&members, &viewer.public_key().to_hex());

        // The relay may return an event from an author outside the filter.
        let untrusted = record(&other, &agent);
        assert!(classify(&members, &[untrusted], &authors).agents.is_empty());

        // A record whose signature does not verify is not the viewer's claim.
        let mut forged: serde_json::Value =
            serde_json::from_str(&record(&other, &agent).as_json()).unwrap();
        forged["pubkey"] = serde_json::json!(viewer.public_key().to_hex());
        let forged: Event = serde_json::from_value(forged).unwrap();
        assert!(classify(&members, &[forged], &authors).agents.is_empty());
    }

    #[test]
    fn failed_record_read_keeps_bot_agents_and_blocks_auto_end() {
        let viewer = Keys::generate();
        let attested = Keys::generate().public_key().to_hex();
        let legacy = Keys::generate().public_key().to_hex();
        let members = vec![
            member(&viewer.public_key().to_hex(), "owner"),
            member(&attested, "bot"),
            member(&legacy, "member"),
        ];
        let authors = trusted_record_authors(&members, &viewer.public_key().to_hex());

        let failed = roster_from(&members, Err("relay down".to_string()), &authors);
        assert_eq!(failed.agents, vec![attested.clone()]);
        assert!(!failed.complete);
        // Viewer + legacy agent look like two people, but the roster is
        // incomplete, so the leave path must not treat this as "last person".
        assert_eq!(failed.humans_remaining(), None);

        // The next successful read classifies the legacy agent again.
        let recovered = roster_from(&members, Ok(vec![record(&viewer, &legacy)]), &authors);
        assert_eq!(recovered.humans_remaining(), Some(1));
        assert_eq!(recovered.agents, vec![attested, legacy]);
    }

    #[test]
    fn failed_record_read_adds_bot_agents_to_stored_state() {
        let viewer = Keys::generate();
        let attested = Keys::generate().public_key().to_hex();
        let legacy = Keys::generate().public_key().to_hex();
        let members = vec![
            member(&viewer.public_key().to_hex(), "owner"),
            member(&attested, "bot"),
            member(&legacy, "member"),
        ];
        let authors = trusted_record_authors(&members, &viewer.public_key().to_hex());
        let failed = || roster_from(&members, Err("relay down".to_string()), &authors);

        // A joiner starts with no stored agents. The role-proven agent must
        // still reach state, while the roster cannot end the Huddle.
        let mut joiner = Vec::new();
        assert_eq!(failed().humans_remaining(), None);
        assert!(failed().apply_agents(&mut joiner));
        assert_eq!(joiner, vec![attested.clone()]);

        // An agent that only a record proved stays through a failed read.
        let mut stored = vec![legacy.clone(), attested.clone()];
        assert!(!failed().apply_agents(&mut stored));
        assert_eq!(stored, vec![legacy.clone(), attested.clone()]);

        // A stored agent that left the channel drops out.
        let departed = Keys::generate().public_key().to_hex();
        let mut stale = vec![departed, legacy.clone()];
        assert!(failed().apply_agents(&mut stale));
        assert_eq!(stale, vec![legacy.clone(), attested.clone()]);

        // A complete roster replaces the list.
        let complete = roster_from(&members, Ok(Vec::new()), &authors);
        assert!(complete.apply_agents(&mut stored));
        assert_eq!(stored, vec![attested]);
    }

    #[test]
    fn candidates_skip_bot_members() {
        let members = vec![
            member("aa", "bot"),
            member("BB", "member"),
            member("cc", "owner"),
        ];
        assert_eq!(record_candidates(&members), vec!["bb", "cc"]);
        assert_eq!(trusted_record_authors(&members, "DD"), vec!["dd", "cc"]);
    }
}
