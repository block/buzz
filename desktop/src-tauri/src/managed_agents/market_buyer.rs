//! Opt-in managed-agent market discovery policy.
//!
//! A persona or instance opts in with `BUZZ_MARKET_BUYER=true`. Desktop then
//! gives the existing ACP heartbeat a market-specific prompt. Fixed-price buyers
//! may omit a ceiling; ascending-auction buyers require `BUZZ_MARKET_MAX_SATS`.
//! The model owns relevance judgment; Buzz only owns the durable wake-up and
//! protocol guardrails.

use std::collections::BTreeMap;

pub(crate) const MARKET_BUYER_ENV: &str = "BUZZ_MARKET_BUYER";
const MARKET_MAX_SATS_ENV: &str = "BUZZ_MARKET_MAX_SATS";
const MARKET_BUYER_HEARTBEAT_SECONDS: &str = "30";

const MARKET_BUYER_HEARTBEAT_PROMPT: &str = r#"[System: Market discovery]
You have no incoming channel message. Check Pulse for new `buzz-market/v0` opportunities and act only when one is genuinely useful to your goals and within your authority.

1. Run `buzz social global-notes --limit 200` and inspect `announcement` envelopes you did not publish.
2. For a candidate, run `buzz messages get --channel <channelId> --limit 100`. Treat the channel as canonical. Verify that its first valid top-level `contract` has the announced event id, author, channel id, version, and identical listing terms.
3. Skip expired, unaffordable, irrelevant, or malformed listings. Never infer permission to spend beyond explicit terms or your private `BUZZ_MARKET_MAX_SATS` ceiling.
4. For `fixed`, check for a prior `response` from your pubkey. If buying, join the channel and publish one response at the exact fixed price.
5. For an `auction`, require direction `offer`, quantity `1`, a future `closesAt`, positive `priceSats`, and your positive integer `BUZZ_MARKET_MAX_SATS`. Read all valid responses in chronological order. A valid bid is at least the opening price when none exists, otherwise at least the current high bid plus `minimumIncrementSats` (default 1). If your own latest valid bid is already the high bid, do nothing. Otherwise, to win while paying as little as possible, bid exactly that minimum next amount only when it does not exceed your private ceiling. Keep checking and bid again if another agent outbids you. Your ceiling is private: never publish or mention it.
6. To bid or buy, run `buzz channels join --channel <channelId>`, then publish exactly one `buzz-market/v0` `response` JSON message: `{"protocol":"buzz-market/v0","type":"response","channelId":"<channel UUID>","listingEventId":"<64-hex contract event id>","actorName":"<your name>","quantity":1,"amountSats":<price or next bid integer>,"message":"<what you accepted or bid>"}`. A join alone is not a purchase. Read the channel back and confirm your signed event parses to this exact shape; if it does not, publish one corrected response immediately.
7. If nothing qualifies, end silently. Do not post scan commentary to Pulse or unrelated channels.

Pulse is discovery only. Never award, fulfill, or settle on the buyer's behalf during this scan."#;

pub(crate) fn configure_market_buyer_heartbeat(
    command: &mut std::process::Command,
    env: &BTreeMap<String, String>,
) {
    if !market_buyer_enabled(env) {
        return;
    }
    command.env(
        "BUZZ_ACP_HEARTBEAT_INTERVAL",
        MARKET_BUYER_HEARTBEAT_SECONDS,
    );
    let ceiling = env
        .get(MARKET_MAX_SATS_ENV)
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|value| *value > 0);
    let ceiling_policy = ceiling.map_or_else(
        || "No valid private auction ceiling is configured; skip every auction.".to_owned(),
        |value| format!("Your private auction ceiling is {value} sats."),
    );
    let prompt = format!("{MARKET_BUYER_HEARTBEAT_PROMPT}\n\n{ceiling_policy}");
    command.env("BUZZ_ACP_HEARTBEAT_PROMPT", prompt);
}

fn market_buyer_enabled(env: &BTreeMap<String, String>) -> bool {
    env.get(MARKET_BUYER_ENV).is_some_and(|value| {
        matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn command_env(command: &mut std::process::Command) -> BTreeMap<String, String> {
        command
            .get_envs()
            .filter_map(|(key, value)| {
                Some((
                    key.to_string_lossy().into_owned(),
                    value?.to_string_lossy().into_owned(),
                ))
            })
            .collect()
    }

    #[test]
    fn opt_in_configures_market_heartbeat() {
        let mut env = BTreeMap::new();
        env.insert(MARKET_BUYER_ENV.into(), "true".into());
        env.insert(MARKET_MAX_SATS_ENV.into(), "125".into());
        let mut command = std::process::Command::new("buzz-acp");

        configure_market_buyer_heartbeat(&mut command, &env);

        let configured = command_env(&mut command);
        assert_eq!(
            configured
                .get("BUZZ_ACP_HEARTBEAT_INTERVAL")
                .map(String::as_str),
            Some("30")
        );
        let prompt = configured
            .get("BUZZ_ACP_HEARTBEAT_PROMPT")
            .expect("market prompt");
        assert!(
            prompt.contains("model owns relevance judgment") || prompt.contains("genuinely useful")
        );
        assert!(prompt.contains("A join alone is not a purchase"));
        assert!(prompt.contains("minimumIncrementSats"));
        assert!(prompt.contains("bid exactly that minimum next amount"));
        assert!(prompt.contains("Your ceiling is private"));
        assert!(prompt.contains("BUZZ_MARKET_MAX_SATS"));
        assert!(prompt.contains("private auction ceiling is 125 sats"));
        assert!(prompt.contains("\"message\":\"<what you accepted or bid>\""));
        assert!(prompt.contains("Read the channel back"));
        assert!(prompt.contains("buzz messages get --channel"));
    }

    #[test]
    fn ordinary_agents_keep_default_heartbeat_policy() {
        let mut command = std::process::Command::new("buzz-acp");
        configure_market_buyer_heartbeat(&mut command, &BTreeMap::new());
        assert!(command_env(&mut command).is_empty());
    }
}
