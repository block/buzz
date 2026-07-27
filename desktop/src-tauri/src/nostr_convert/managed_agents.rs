//! Managed-agent directory projection (kind:30177).
//!
//! Split out of `nostr_convert` so the generic event converters stay readable
//! and this authorization-facing projection has an obvious home.

use nostr::{Event, ToBech32};
use serde_json::{json, Value};

use super::first_tag_value;

/// Convert public managed-agent projections into the relay-agent directory
/// shape consumed by the desktop.
///
/// The event author is the human owner. The managed agent's public key is the
/// parameterized replaceable event's `d` tag, so a content-supplied key is
/// never authoritative. Build a narrow output object instead of forwarding
/// the complete content projection so future fields cannot accidentally cross
/// this frontend boundary.
pub fn managed_agents_from_events(events: &[Event]) -> Value {
    let arr: Vec<Value> = events
        .iter()
        .filter_map(|event| {
            let agent_pubkey = nostr::PublicKey::from_hex(first_tag_value(event, "d")?).ok()?;
            let pubkey = agent_pubkey.to_hex();
            let npub = agent_pubkey.to_bech32().unwrap_or_else(|_| pubkey.clone());
            let content: Value = serde_json::from_str(&event.content).ok()?;
            let object = content.as_object()?;
            let name = object
                .get("name")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .unwrap_or(&npub);
            // Only the modes `RespondTo` can represent may be projected. The
            // harness also supports `nobody`, but the desktop enum deliberately
            // omits it, so emitting it here would fail deserialization for the
            // WHOLE directory response — one agent publishing an unsupported
            // mode would hide every other agent. An unrepresentable mode
            // becomes absent, which every eligibility check treats as
            // not-invocable, so the projection degrades closed.
            let respond_to = object
                .get("respond_to")
                .and_then(Value::as_str)
                .filter(|mode| matches!(*mode, "owner-only" | "allowlist" | "anyone"));

            let mut respond_to_allowlist = Vec::new();
            if let Some(values) = object.get("respond_to_allowlist").and_then(Value::as_array) {
                for value in values {
                    let Some(raw) = value.as_str() else {
                        continue;
                    };
                    let Ok(key) = nostr::PublicKey::from_hex(raw.trim()) else {
                        continue;
                    };
                    let normalized = key.to_hex();
                    if !respond_to_allowlist.contains(&normalized) {
                        respond_to_allowlist.push(normalized);
                    }
                }
            }

            Some(json!({
                "pubkey": pubkey,
                "name": name,
                "agent_type": "agent",
                "channels": [],
                "channel_ids": [],
                "capabilities": [],
                "status": "offline",
                "respond_to": respond_to,
                "respond_to_allowlist": respond_to_allowlist,
            }))
        })
        .collect();

    json!({ "agents": arr })
}
#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{EventBuilder, Keys, Kind, Tag};

    fn ev(kind: u16, content: &str, tags: Vec<Vec<&str>>) -> Event {
        let keys = Keys::generate();
        let tags: Vec<Tag> = tags
            .into_iter()
            .map(|t| Tag::parse(t.iter().map(|s| s.to_string()).collect::<Vec<_>>()).unwrap())
            .collect();
        EventBuilder::new(Kind::from(kind), content)
            .tags(tags)
            .sign_with_keys(&keys)
            .unwrap()
    }

    #[test]
    fn managed_agents_use_d_tag_identity_and_preserve_allowlist_metadata() {
        let agent_pubkey = "02".repeat(32);
        let allowed_pubkey = "03".repeat(32);
        let e = ev(
            30177,
            &format!(
                r#"{{"pubkey":"forged","name":"Scout","parallelism":1,"respond_to":"allowlist","respond_to_allowlist":["{allowed_pubkey}"],"env_vars":{{"SECRET":"do-not-project"}}}}"#
            ),
            vec![vec!["d", &agent_pubkey]],
        );
        let v = managed_agents_from_events(std::slice::from_ref(&e));
        let agents = v.get("agents").cloned().unwrap();
        let parsed: Vec<crate::managed_agents::RelayAgentInfo> =
            serde_json::from_value(agents).unwrap();

        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].pubkey, agent_pubkey);
        assert_eq!(parsed[0].name, "Scout");
        assert_eq!(
            parsed[0].respond_to,
            Some(crate::managed_agents::RespondTo::Allowlist)
        );
        assert_eq!(parsed[0].respond_to_allowlist, vec![allowed_pubkey]);
        assert!(
            !v.to_string().contains("SECRET"),
            "the directory projection must remain an explicit public-field allowlist"
        );
    }

    /// One agent publishing a mode the desktop enum cannot represent must not
    /// take down the entire directory. `respond_to` deserializes into
    /// `RespondTo`, which has no `nobody` variant, so projecting that string
    /// would fail the whole `Vec<RelayAgentInfo>` and hide every other agent.
    #[test]
    fn managed_agents_unsupported_mode_does_not_break_other_entries() {
        let good_pubkey = "02".repeat(32);
        let nobody_pubkey = "03".repeat(32);
        let good = ev(
            30177,
            r#"{"name":"Good","respond_to":"owner-only"}"#,
            vec![vec!["d", &good_pubkey]],
        );
        let nobody = ev(
            30177,
            r#"{"name":"Nope","respond_to":"nobody"}"#,
            vec![vec!["d", &nobody_pubkey]],
        );
        let garbage = ev(
            30177,
            r#"{"name":"Junk","respond_to":"not-a-mode"}"#,
            vec![vec!["d", &"04".repeat(32)]],
        );

        let v = managed_agents_from_events(&[good, nobody, garbage]);
        let parsed: Vec<crate::managed_agents::RelayAgentInfo> =
            serde_json::from_value(v.get("agents").cloned().unwrap())
                .expect("an unsupported mode must not fail the whole directory");

        assert_eq!(parsed.len(), 3, "every entry survives");
        assert_eq!(
            parsed[0].respond_to,
            Some(crate::managed_agents::RespondTo::OwnerOnly)
        );
        // Unrepresentable modes degrade closed: absent, never invocable.
        assert_eq!(parsed[1].respond_to, None);
        assert_eq!(parsed[2].respond_to, None);
    }

    #[test]
    fn managed_agents_drop_events_without_a_valid_agent_d_tag() {
        let missing = ev(
            30177,
            r#"{"name":"Missing","parallelism":1,"respond_to":"owner-only"}"#,
            vec![],
        );
        let invalid = ev(
            30177,
            r#"{"name":"Invalid","parallelism":1,"respond_to":"owner-only"}"#,
            vec![vec!["d", "not-a-pubkey"]],
        );
        let v = managed_agents_from_events(&[missing, invalid]);

        assert_eq!(v.get("agents").and_then(Value::as_array).unwrap().len(), 0);
    }
}
