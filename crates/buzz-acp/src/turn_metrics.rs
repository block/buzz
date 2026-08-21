//! Encrypted per-turn usage and context-manifest publication.

use std::time::Duration;

use buzz_core::agent_turn_metric::{
    AgentTurnMetricPayload, ContextManifest, StopReason, TokenCounts,
};
use nostr::{EventBuilder, Kind, Tag};
use uuid::Uuid;

use crate::pool::PromptContext;
use crate::usage::TurnUsage;

/// Build the per-turn and cumulative token counts for a NIP-AM payload.
pub(crate) fn build_turn_metric_counts(
    usage: &TurnUsage,
) -> (Option<TokenCounts>, Option<TokenCounts>) {
    let turn_counts = usage.delta_reliable.then_some(TokenCounts {
        input_tokens: usage.turn_input_tokens,
        output_tokens: usage.turn_output_tokens,
        total_tokens: usage.turn_total_tokens,
        cost_usd: usage.turn_cost_usd,
        cache_read_tokens: usage.turn_cache_read_tokens,
        cache_write_tokens: usage.turn_cache_write_tokens,
    });
    let cumulative_counts = Some(TokenCounts {
        input_tokens: usage.cumulative_input_tokens,
        output_tokens: usage.cumulative_output_tokens,
        total_tokens: usage.cumulative_total_tokens,
        cost_usd: usage.cumulative_cost_usd,
        cache_read_tokens: usage.cumulative_cache_read_tokens,
        cache_write_tokens: usage.cumulative_cache_write_tokens,
    });
    (turn_counts, cumulative_counts)
}

/// Best-effort encrypted NIP-AM publication; metric errors never fail a turn.
pub(crate) async fn publish_agent_turn_metric(
    ctx: &PromptContext,
    usage: Option<TurnUsage>,
    channel_id: Option<Uuid>,
    session_id: &str,
    turn_id: &str,
    stop_reason: Option<StopReason>,
    context_manifest: Option<ContextManifest>,
) {
    let (usage, owner_pk) = match (usage, ctx.agent_owner_pubkey.as_ref()) {
        (Some(usage), Some(owner_pk)) => (usage, owner_pk),
        _ => return,
    };
    let (turn, cumulative) = build_turn_metric_counts(&usage);
    let payload = AgentTurnMetricPayload {
        harness: ctx.harness_name.clone(),
        model: usage.model.clone(),
        channel_id: channel_id.map(|id| id.to_string()),
        session_id: Some(usage.session_id.clone()),
        turn_id: Some(turn_id.to_string()),
        turn_seq: Some(usage.turn_seq),
        timestamp: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        turn,
        cumulative,
        delta_reliable: usage.delta_reliable,
        stop_reason,
        pricing_identity: usage.pricing_identity.clone(),
        context_manifest,
    };
    let ciphertext = match buzz_core::agent_turn_metric::encrypt_agent_turn_metric(
        &ctx.agent_keys,
        owner_pk,
        &payload,
    ) {
        Ok(ciphertext) => ciphertext,
        Err(error) => {
            tracing::warn!(target: "pool::metrics", session_id, turn_id, "NIP-AM: encrypt failed: {error}");
            return;
        }
    };
    let agent_hex = ctx.agent_keys.public_key().to_hex();
    let owner_hex = owner_pk.to_hex();
    let tags = match (
        Tag::parse(["p", &owner_hex]),
        Tag::parse(["agent", &agent_hex]),
    ) {
        (Ok(owner), Ok(agent)) => [owner, agent],
        (Err(error), _) | (_, Err(error)) => {
            tracing::warn!(target: "pool::metrics", session_id, turn_id, "NIP-AM: tag failed: {error}");
            return;
        }
    };
    let event = match EventBuilder::new(
        Kind::Custom(buzz_core::kind::KIND_AGENT_TURN_METRIC as u16),
        ciphertext,
    )
    .tags(tags)
    .sign_with_keys(&ctx.agent_keys)
    {
        Ok(event) => event,
        Err(error) => {
            tracing::warn!(target: "pool::metrics", session_id, turn_id, "NIP-AM: sign failed: {error}");
            return;
        }
    };
    const METRIC_TIMEOUT: Duration = Duration::from_secs(3);
    match tokio::time::timeout(METRIC_TIMEOUT, ctx.rest_client.submit_event(&event)).await {
        Ok(Ok(_)) => {}
        Ok(Err(error)) => tracing::warn!(
            target: "pool::metrics",
            session_id,
            turn_id,
            "NIP-AM: publish failed: {error}"
        ),
        Err(_) => tracing::warn!(
            target: "pool::metrics",
            session_id,
            turn_id,
            "NIP-AM: publish timed out"
        ),
    }
}
