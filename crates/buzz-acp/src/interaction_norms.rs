//! App-level interaction norms injected unconditionally into every agent's
//! standing context, on both delivery paths (`session/new` system role for
//! protocol-v2 agents, first-user-message [`StandingContext`] for legacy
//! agents). Unlike the base prompt (replaceable via
//! `BUZZ_ACP_BASE_PROMPT_FILE`, removable via `BUZZ_ACP_NO_BASE_PROMPT`) or
//! the persona (author-controlled), this block has no off switch: it encodes
//! Buzz-the-platform's defaults, not any operator's or author's preferences.
//!
//! Precedence is deliberate: these are defaults, so anything a person states
//! — in the moment, in a persona, or in team instructions — wins. The wording
//! says so explicitly, and both delivery paths place this block before all
//! other standing content so that content reads as the override, not the
//! other way around.
//!
//! Kept tiny. Every norm added here taxes every session of every agent, so
//! entries must be cross-cutting behavioral defaults that cannot live
//! anywhere more targeted (the base prompt, a persona, a skill).
//!
//! # Durability differs by delivery path
//!
//! Enforcement is not equally strong on both paths, and the difference is
//! structural rather than a bug to fix here:
//!
//! - **Protocol-v2 and Claude agents** receive this block in the `session/new`
//!   system role, where it persists for the life of the session and is
//!   re-established on every new session.
//! - **Legacy agents** (`protocol_version < 2`, excluding claude-agent-acp,
//!   plus goose builds without the system-prompt extension) receive it in the
//!   session's *first user message only* — `format_prompt` gates standing
//!   context behind `!standing_context_sent` so a large block is not re-sent
//!   every turn. Fifty turns later these norms are old context competing with
//!   everything since, so adherence degrades over a long session.
//!
//! Re-sending per turn was considered and rejected: it would re-add the entire
//! standing block (base prompt, persona, team instructions, memory, canvas) on
//! the legacy path, since `StandingContext::sections()` renders them together.
//! The honest summary is that these are durable defaults for modern agents and
//! best-effort for legacy ones. If a legacy agent slips late in a long
//! session, this gate is the first place to look.

/// The `[Defaults]` section leading every agent's standing context.
///
/// The first bullet names the inference vectors (name, avatar, persona theme,
/// writing style) rather than only stating the rule, because that is the
/// observed failure: an agent reads a display name whose connotation feels
/// gendered and writes from that with no source. Naming the vector is what
/// makes the norm bite. It deliberately does not ask the agent to announce
/// that it is defaulting — in a channel that would draw attention to a
/// teammate's unstated identity, which is worse than the quiet correct
/// default.
///
/// The second bullet exists because Buzz agents have persistent memory
/// (`core` engrams) shared across all sessions of an agent: one session
/// recording a guessed gender poisons every future session, and the
/// mis-gendering outlives the conversation where it happened. Same-turn
/// correction matches the existing "evict completed work the same turn"
/// memory discipline in the base prompt.
///
/// That bullet also carries the pubkey-keying clause, even though
/// `base_prompt.md` states it at length. Display names are not unique on a
/// relay (`buzz_sdk::mentions::match_names_to_profiles` deliberately returns
/// every pubkey sharing a name), so pronouns read off one profile can attach
/// to a same-named stranger — an error that cites a real source and therefore
/// survives scrutiny a guess would not. The fuller rule lives in the base
/// prompt, which `BUZZ_ACP_BASE_PROMPT_FILE` replaces wholesale; keeping the
/// short form here means an operator with a custom base prompt cannot end up
/// with the pronoun default but not the binding rule, which is exactly the
/// combination that produces confidently-sourced mis-gendering.
pub(crate) const INTERACTION_NORMS_PREAMBLE: &str = "[Defaults]\n\
- Never infer anyone's gender or pronouns from a name, avatar, persona theme, or writing style: use they/them (it/its for agents) unless stated, and stated pronouns always win.\n\
- Record pronouns only as stated and keyed to the person's pubkey, since display names are not unique; correct contradicting memory the same turn.";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preamble_is_framed_as_an_overridable_default() {
        assert!(INTERACTION_NORMS_PREAMBLE.starts_with("[Defaults]\n"));
        assert!(INTERACTION_NORMS_PREAMBLE.contains("Never infer anyone's gender"));
        assert!(INTERACTION_NORMS_PREAMBLE.contains("they/them"));
        assert!(INTERACTION_NORMS_PREAMBLE.contains("stated pronouns always win"));
    }

    #[test]
    fn preamble_names_the_inference_vectors() {
        // Stating the rule alone left the observed failure open: gender read
        // off a display name's connotation. The vectors must be explicit.
        for vector in ["name", "avatar", "persona theme", "writing style"] {
            assert!(
                INTERACTION_NORMS_PREAMBLE.contains(vector),
                "missing inference vector: {vector}"
            );
        }
    }

    #[test]
    fn preamble_does_not_ask_agents_to_announce_the_default() {
        // Announcing "you didn't state pronouns" in a channel spotlights a
        // teammate's unstated identity. The default stays quiet.
        let lowered = INTERACTION_NORMS_PREAMBLE.to_lowercase();
        for phrase in [
            "say so",
            "note that you",
            "explain that you",
            "tell them you",
        ] {
            assert!(
                !lowered.contains(phrase),
                "preamble must not ask agents to announce the default: {phrase}"
            );
        }
    }

    #[test]
    fn preamble_covers_persistent_memory() {
        // Buzz-specific: core memory is shared across sessions, so a guessed
        // gender recorded once would be re-asserted everywhere, forever.
        assert!(INTERACTION_NORMS_PREAMBLE.contains("only as stated"));
        assert!(INTERACTION_NORMS_PREAMBLE.contains("correct contradicting memory the same turn"));
    }

    #[test]
    fn preamble_keys_remembered_pronouns_to_a_pubkey() {
        // base_prompt.md states this too, but it is replaceable via
        // BUZZ_ACP_BASE_PROMPT_FILE. Carrying the clause here keeps an
        // operator from ending up with the pronoun default but not the
        // binding rule — the combination that yields sourced mis-gendering.
        assert!(INTERACTION_NORMS_PREAMBLE.contains("keyed to the person's pubkey"));
        assert!(INTERACTION_NORMS_PREAMBLE.contains("display names are not unique"));
    }

    #[test]
    fn preamble_stays_small() {
        // This block is prepended to every send of every session, so its size
        // is a standing tax. Feedback was that prompts are already too long;
        // the ceiling makes a regression fail here rather than in a bill.
        assert!(
            INTERACTION_NORMS_PREAMBLE.len() < 400,
            "preamble grew to {} bytes — keep it tight or drop a norm",
            INTERACTION_NORMS_PREAMBLE.len()
        );
    }
}
