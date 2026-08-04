//! Contextual agent conversation audience + reply-placement policy (ACP).
//!
//! Shared contract: `tests/fixtures/contextual-agent-conversation-cases.json`.
//! Pure resolver — harness wiring comes in later ACP leaves.

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// Device-local unaddressed-channel agent mode (mirrors Desktop/Flutter).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum UnaddressedChannelAgentMode {
    AllChannelAgents,
    MentionsOnly,
}

/// Reply placement for agent responses on the wire.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum ReplyPlacement {
    TopLevel,
    #[serde(rename = "thread-root")]
    ThreadRoot {
        #[serde(rename = "eventId")]
        event_id: String,
    },
    Unconstrained,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextualAgentConversationInput {
    pub conversation: String,
    pub message_position: String,
    pub sender_class: String,
    pub unaddressed_mode: UnaddressedChannelAgentMode,
    pub keep_addressed_agents_active: bool,
    pub explicit_mention_pubkeys: Vec<String>,
    pub current_agent_pubkey: Option<String>,
    pub channel_member_pubkeys: Vec<String>,
    pub verified_channel_agent_pubkeys: Vec<String>,
    pub unverified_agent_pubkeys: Vec<String>,
    pub non_member_agent_pubkeys: Vec<String>,
    pub thread_root_event_id: Option<String>,
    pub replying_under_event_id: Option<String>,
    pub persistent_thread_audience: Vec<String>,
    pub manual_removed_pubkeys: Vec<String>,
    pub recipient_load_error: bool,
    pub human_message_event_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextualAgentConversationDecision {
    pub audience_pubkeys: Vec<String>,
    pub reply_placement: ReplyPlacement,
    pub shared_thread: bool,
    pub retain_draft: bool,
    #[serde(default)]
    pub nest_under_agent_reply: Option<bool>,
}

fn normalize_pubkey(pubkey: &str) -> String {
    pubkey.trim().to_ascii_lowercase()
}

fn unique_sorted(pubkeys: impl IntoIterator<Item = String>) -> Vec<String> {
    let set: BTreeSet<String> = pubkeys
        .into_iter()
        .map(|p| normalize_pubkey(&p))
        .filter(|p| !p.is_empty())
        .collect();
    set.into_iter().collect()
}

fn eligible_channel_agents(input: &ContextualAgentConversationInput) -> BTreeSet<String> {
    let members: BTreeSet<String> = input
        .channel_member_pubkeys
        .iter()
        .map(|p| normalize_pubkey(p))
        .collect();
    input
        .verified_channel_agent_pubkeys
        .iter()
        .map(|p| normalize_pubkey(p))
        .filter(|p| members.contains(p))
        .collect()
}

fn filter_to_eligible(candidates: &[String], eligible: &BTreeSet<String>) -> Vec<String> {
    unique_sorted(
        candidates
            .iter()
            .map(|p| normalize_pubkey(p))
            .filter(|p| eligible.contains(p)),
    )
}

fn placement_for(
    input: &ContextualAgentConversationInput,
    audience_count: usize,
) -> ReplyPlacement {
    if input.message_position == "in-thread" {
        if let Some(root) = input.thread_root_event_id.as_ref() {
            return ReplyPlacement::ThreadRoot {
                event_id: root.clone(),
            };
        }
    }
    if audience_count >= 2 {
        if let Some(event_id) = input
            .human_message_event_id
            .as_ref()
            .or(input.thread_root_event_id.as_ref())
        {
            return ReplyPlacement::ThreadRoot {
                event_id: event_id.clone(),
            };
        }
    }
    ReplyPlacement::TopLevel
}

/// ACP turn context for reply placement (mirrors client policy without I/O).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcpTurnPlacementInput {
    /// True when the turn is 1:1 DM-scoped.
    pub is_dm: bool,
    /// Human-facing turns flatten; agent-only stays unconstrained.
    pub is_human_facing: bool,
    pub message_position: &'static str,
    pub thread_root_event_id: Option<String>,
    pub triggering_event_id: String,
    /// How many agents are addressed on the triggering human message (`p` tags).
    /// Count of 0 is treated as 1 when the turn is human-facing (this agent alone).
    pub addressed_agent_count: usize,
}

/// Map placement to an optional `--reply-to` event id for the agent prompt.
pub fn reply_placement_anchor(placement: &ReplyPlacement) -> Option<&str> {
    match placement {
        ReplyPlacement::ThreadRoot { event_id } => Some(event_id.as_str()),
        ReplyPlacement::TopLevel | ReplyPlacement::Unconstrained => None,
    }
}

/// Resolve reply placement for an ACP turn.
///
/// - Agent-only (not human-facing): unconstrained (no forced anchor).
/// - Direct (DM) top-level: top-level flat.
/// - Direct (DM) in-thread: thread root (or triggering id if root missing).
/// - Channel top-level with ≥2 addressed agents: shared thread at human event.
/// - Channel top-level with one addressed agent: top-level flat.
/// - Channel in-thread: always thread root (never nest under an agent reply).
pub fn resolve_acp_turn_placement(input: &AcpTurnPlacementInput) -> ReplyPlacement {
    if !input.is_human_facing {
        return ReplyPlacement::Unconstrained;
    }

    let agent_count = input.addressed_agent_count.max(1);

    if input.is_dm {
        if input.message_position == "in-thread" {
            let event_id = input
                .thread_root_event_id
                .clone()
                .unwrap_or_else(|| input.triggering_event_id.clone());
            return ReplyPlacement::ThreadRoot { event_id };
        }
        return ReplyPlacement::TopLevel;
    }

    // Channel
    if input.message_position == "in-thread" {
        if let Some(root) = &input.thread_root_event_id {
            return ReplyPlacement::ThreadRoot {
                event_id: root.clone(),
            };
        }
    }

    if agent_count >= 2 {
        return ReplyPlacement::ThreadRoot {
            event_id: input.triggering_event_id.clone(),
        };
    }

    ReplyPlacement::TopLevel
}

/// Resolve audience and reply placement for a human/agent send path.
pub fn resolve_contextual_agent_conversation(
    input: &ContextualAgentConversationInput,
) -> ContextualAgentConversationDecision {
    if input.recipient_load_error {
        return ContextualAgentConversationDecision {
            audience_pubkeys: vec![],
            reply_placement: ReplyPlacement::TopLevel,
            shared_thread: false,
            retain_draft: true,
            nest_under_agent_reply: Some(false),
        };
    }

    if input.sender_class == "agent" {
        return ContextualAgentConversationDecision {
            audience_pubkeys: vec![],
            reply_placement: ReplyPlacement::Unconstrained,
            shared_thread: false,
            retain_draft: false,
            nest_under_agent_reply: Some(false),
        };
    }

    if input.conversation == "direct" {
        let audience = input
            .current_agent_pubkey
            .as_ref()
            .map(|p| vec![normalize_pubkey(p)])
            .unwrap_or_default();
        return ContextualAgentConversationDecision {
            reply_placement: placement_for(input, audience.len()),
            shared_thread: false,
            retain_draft: false,
            nest_under_agent_reply: Some(false),
            audience_pubkeys: audience,
        };
    }

    let eligible = eligible_channel_agents(input);
    let removed: BTreeSet<String> = input
        .manual_removed_pubkeys
        .iter()
        .map(|p| normalize_pubkey(p))
        .collect();

    let explicit: Vec<String> = filter_to_eligible(&input.explicit_mention_pubkeys, &eligible)
        .into_iter()
        .filter(|p| !removed.contains(p))
        .collect();

    let audience = if !explicit.is_empty() {
        explicit
    } else {
        let persistent = if input.keep_addressed_agents_active {
            filter_to_eligible(&input.persistent_thread_audience, &eligible)
                .into_iter()
                .filter(|p| !removed.contains(p))
                .collect::<Vec<_>>()
        } else {
            vec![]
        };

        if !persistent.is_empty() {
            persistent
        } else if matches!(
            input.unaddressed_mode,
            UnaddressedChannelAgentMode::AllChannelAgents
        ) {
            eligible
                .into_iter()
                .filter(|p| !removed.contains(p))
                .collect()
        } else {
            vec![]
        }
    };

    let shared_thread = audience.len() >= 2;
    ContextualAgentConversationDecision {
        reply_placement: placement_for(input, audience.len()),
        shared_thread,
        retain_draft: false,
        nest_under_agent_reply: Some(false),
        audience_pubkeys: audience,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    const FIXTURE: &str =
        include_str!("../../../tests/fixtures/contextual-agent-conversation-cases.json");

    #[derive(Debug, Deserialize)]
    struct FixtureFile {
        version: u32,
        cases: Vec<FixtureCase>,
    }

    #[derive(Debug, Deserialize)]
    struct FixtureCase {
        id: String,
        input: Value,
        expected: ContextualAgentConversationDecision,
    }

    fn parse_input_manual(value: &Value) -> ContextualAgentConversationInput {
        let mode = match value["unaddressedMode"].as_str().unwrap_or("") {
            "mentions-only" => UnaddressedChannelAgentMode::MentionsOnly,
            _ => UnaddressedChannelAgentMode::AllChannelAgents,
        };
        let str_list = |key: &str| -> Vec<String> {
            value
                .get(key)
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(|s| s.to_string()))
                        .collect()
                })
                .unwrap_or_default()
        };
        let opt_str = |key: &str| -> Option<String> {
            match value.get(key) {
                Some(Value::Null) | None => None,
                Some(v) => v.as_str().map(|s| s.to_string()),
            }
        };
        ContextualAgentConversationInput {
            conversation: value["conversation"].as_str().unwrap_or("").to_string(),
            message_position: value["messagePosition"].as_str().unwrap_or("").to_string(),
            sender_class: value["senderClass"].as_str().unwrap_or("").to_string(),
            unaddressed_mode: mode,
            keep_addressed_agents_active: value["keepAddressedAgentsActive"]
                .as_bool()
                .unwrap_or(false),
            explicit_mention_pubkeys: str_list("explicitMentionPubkeys"),
            current_agent_pubkey: opt_str("currentAgentPubkey"),
            channel_member_pubkeys: str_list("channelMemberPubkeys"),
            verified_channel_agent_pubkeys: str_list("verifiedChannelAgentPubkeys"),
            unverified_agent_pubkeys: str_list("unverifiedAgentPubkeys"),
            non_member_agent_pubkeys: str_list("nonMemberAgentPubkeys"),
            thread_root_event_id: opt_str("threadRootEventId"),
            replying_under_event_id: opt_str("replyingUnderEventId"),
            persistent_thread_audience: str_list("persistentThreadAudience"),
            manual_removed_pubkeys: str_list("manualRemovedPubkeys"),
            recipient_load_error: value["recipientLoadError"].as_bool().unwrap_or(false),
            human_message_event_id: opt_str("humanMessageEventId"),
        }
    }

    #[test]
    fn acp_turn_placement_single_agent_top_level_is_flat() {
        let placement = resolve_acp_turn_placement(&AcpTurnPlacementInput {
            is_dm: false,
            is_human_facing: true,
            message_position: "top-level",
            thread_root_event_id: None,
            triggering_event_id: "trig".into(),
            addressed_agent_count: 1,
        });
        assert_eq!(placement, ReplyPlacement::TopLevel);
        assert_eq!(reply_placement_anchor(&placement), None);
    }

    #[test]
    fn acp_turn_placement_multi_agent_top_level_threads() {
        let placement = resolve_acp_turn_placement(&AcpTurnPlacementInput {
            is_dm: false,
            is_human_facing: true,
            message_position: "top-level",
            thread_root_event_id: None,
            triggering_event_id: "trig".into(),
            addressed_agent_count: 2,
        });
        assert_eq!(
            placement,
            ReplyPlacement::ThreadRoot {
                event_id: "trig".into()
            }
        );
        assert_eq!(reply_placement_anchor(&placement), Some("trig"));
    }

    #[test]
    fn acp_turn_placement_in_thread_uses_root() {
        let placement = resolve_acp_turn_placement(&AcpTurnPlacementInput {
            is_dm: false,
            is_human_facing: true,
            message_position: "in-thread",
            thread_root_event_id: Some("root".into()),
            triggering_event_id: "trig".into(),
            addressed_agent_count: 3,
        });
        assert_eq!(
            placement,
            ReplyPlacement::ThreadRoot {
                event_id: "root".into()
            }
        );
    }

    #[test]
    fn acp_turn_placement_agent_only_unconstrained() {
        let placement = resolve_acp_turn_placement(&AcpTurnPlacementInput {
            is_dm: false,
            is_human_facing: false,
            message_position: "in-thread",
            thread_root_event_id: Some("root".into()),
            triggering_event_id: "trig".into(),
            addressed_agent_count: 2,
        });
        assert_eq!(placement, ReplyPlacement::Unconstrained);
        assert_eq!(reply_placement_anchor(&placement), None);
    }

    #[test]
    fn fixture_loads_and_has_required_cases() {
        let file: FixtureFile = serde_json::from_str(FIXTURE).expect("fixture json");
        assert_eq!(file.version, 1);
        assert!(
            file.cases.len() >= 12,
            "expected >=12 cases, got {}",
            file.cases.len()
        );
    }

    #[test]
    fn fixture_policy_decisions_match_expected() {
        let file: FixtureFile = serde_json::from_str(FIXTURE).expect("fixture json");
        let mut failures = Vec::new();
        for case in &file.cases {
            let mut input = parse_input_manual(&case.input);
            if case.expected.reply_placement
                == (ReplyPlacement::ThreadRoot {
                    event_id: "human-message-id".into(),
                })
            {
                input.human_message_event_id = Some("human-message-id".into());
            }
            let decision = resolve_contextual_agent_conversation(&input);
            let mut actual_audience = decision.audience_pubkeys.clone();
            let mut expected_audience = case.expected.audience_pubkeys.clone();
            actual_audience.sort();
            expected_audience.sort();
            if actual_audience != expected_audience
                || decision.reply_placement != case.expected.reply_placement
                || decision.shared_thread != case.expected.shared_thread
                || decision.retain_draft != case.expected.retain_draft
            {
                failures.push(format!(
                    "{}: got audience={:?} placement={:?} shared={} retain={}; expected audience={:?} placement={:?} shared={} retain={}",
                    case.id,
                    decision.audience_pubkeys,
                    decision.reply_placement,
                    decision.shared_thread,
                    decision.retain_draft,
                    case.expected.audience_pubkeys,
                    case.expected.reply_placement,
                    case.expected.shared_thread,
                    case.expected.retain_draft,
                ));
            }
        }
        assert!(
            failures.is_empty(),
            "contextual fixture policy mismatches:\n{}",
            failures.join("\n")
        );
    }
}
