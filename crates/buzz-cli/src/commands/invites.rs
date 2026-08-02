//! `buzz invites` — mint and claim relay invite codes.
//!
//! Both operations are NIP-98-signed REST calls, not Nostr events: invite
//! codes are opaque relay-issued credentials rather than public events, and
//! `POST /api/invites/claim` is deliberately exempt from the relay-membership
//! gate — the whole point is that the caller is not a member yet. That makes
//! it the only onboarding path for a fresh identity on a closed relay;
//! `buzz channels join` publishes a kind:9021 join *request*, which a closed
//! relay rejects with `403 relay_membership_required`.
//!
//! - `mint` — `POST /api/invites`, owner/admin only. Non-idempotent: every
//!   call mints a new live credential, so it is sent exactly once (see
//!   [`BuzzClient::post_authed_once`]).
//! - `claim` — `POST /api/invites/claim`, signed by the *joining* pubkey.
//!   Idempotent: re-claiming an already-redeemed code returns
//!   `already_member`, so the standard retry policy applies.
//!
//! The community (tenant) is selected by the relay host in `--relay` /
//! `BUZZ_RELAY_URL`; a code minted for one community is rejected on another.

use buzz_core::invite::{MAX_INVITE_TTL_SECS, MAX_INVITE_USES, MIN_INVITE_TTL_SECS};

use crate::client::BuzzClient;
use crate::error::CliError;
use crate::InvitesCmd;

/// Relay path for minting an invite code.
const MINT_PATH: &str = "/api/invites";
/// Relay path for claiming an invite code.
const CLAIM_PATH: &str = "/api/invites/claim";

/// Build the `POST /api/invites` body, rejecting out-of-range input locally.
///
/// The relay enforces the same bounds; checking here turns a round-trip 400
/// into an immediate exit-1 usage error. `max_uses` is omitted when unset —
/// the relay reads an absent field as "unlimited uses".
fn mint_request_body(ttl_secs: u64, max_uses: Option<i32>) -> Result<serde_json::Value, CliError> {
    if !(MIN_INVITE_TTL_SECS..=MAX_INVITE_TTL_SECS).contains(&ttl_secs) {
        return Err(CliError::Usage(format!(
            "--ttl-secs must be between {MIN_INVITE_TTL_SECS} and {MAX_INVITE_TTL_SECS}"
        )));
    }
    if let Some(uses) = max_uses {
        if !(1..=MAX_INVITE_USES).contains(&uses) {
            return Err(CliError::Usage(format!(
                "--max-uses must be between 1 and {MAX_INVITE_USES}"
            )));
        }
    }
    let mut body = serde_json::json!({ "ttl_secs": ttl_secs });
    if let Some(uses) = max_uses {
        body["max_uses"] = serde_json::json!(uses);
    }
    Ok(body)
}

/// Build the `POST /api/invites/claim` body.
///
/// The code is trimmed because invite codes are pasted from links, chat
/// messages, and `--code -`-style shell plumbing, all of which pick up
/// surrounding whitespace. Interior whitespace is a genuinely malformed code
/// and is rejected rather than silently repaired.
fn claim_request_body(
    code: &str,
    policy_receipt: Option<&str>,
) -> Result<serde_json::Value, CliError> {
    let code = code.trim();
    if code.is_empty() {
        return Err(CliError::Usage("--code must not be empty".into()));
    }
    if code.chars().any(char::is_whitespace) {
        return Err(CliError::Usage(
            "--code must not contain whitespace — pass the bare token, not the invite URL".into(),
        ));
    }
    let mut body = serde_json::json!({ "code": code });
    if let Some(receipt) = policy_receipt {
        let receipt = receipt.trim();
        if receipt.is_empty() {
            return Err(CliError::Usage("--policy-receipt must not be empty".into()));
        }
        body["policy_receipt"] = serde_json::json!(receipt);
    }
    Ok(body)
}

async fn cmd_mint(
    client: &BuzzClient,
    ttl_secs: u64,
    max_uses: Option<i32>,
) -> Result<(), CliError> {
    let body = mint_request_body(ttl_secs, max_uses)?;
    let resp = client.post_authed_once(MINT_PATH, &body).await?;
    println!("{resp}");
    Ok(())
}

async fn cmd_claim(
    client: &BuzzClient,
    code: &str,
    policy_receipt: Option<&str>,
) -> Result<(), CliError> {
    let body = claim_request_body(code, policy_receipt)?;
    let resp = client.post_authed(CLAIM_PATH, &body).await?;
    println!("{resp}");
    Ok(())
}

pub async fn dispatch(cmd: InvitesCmd, client: &BuzzClient) -> Result<(), CliError> {
    match cmd {
        InvitesCmd::Mint { ttl_secs, max_uses } => cmd_mint(client, ttl_secs, max_uses).await,
        InvitesCmd::Claim {
            code,
            policy_receipt,
        } => cmd_claim(client, &code, policy_receipt.as_deref()).await,
    }
}

#[cfg(test)]
mod tests {
    use super::{claim_request_body, mint_request_body};
    use buzz_core::invite::{
        DEFAULT_INVITE_TTL_SECS, MAX_INVITE_TTL_SECS, MAX_INVITE_USES, MIN_INVITE_TTL_SECS,
    };

    #[test]
    fn mint_body_omits_max_uses_when_unset() {
        let body = mint_request_body(DEFAULT_INVITE_TTL_SECS, None).expect("valid mint request");
        assert_eq!(
            body,
            serde_json::json!({ "ttl_secs": DEFAULT_INVITE_TTL_SECS }),
            "an absent max_uses must stay absent — the relay reads it as unlimited"
        );
    }

    #[test]
    fn mint_body_includes_max_uses_when_set() {
        let body = mint_request_body(3600, Some(1)).expect("valid mint request");
        assert_eq!(body, serde_json::json!({ "ttl_secs": 3600, "max_uses": 1 }));
    }

    #[test]
    fn mint_body_accepts_the_relays_boundary_values() {
        for (ttl, max_uses) in [
            (MIN_INVITE_TTL_SECS, Some(1)),
            (MAX_INVITE_TTL_SECS, Some(MAX_INVITE_USES)),
        ] {
            assert!(
                mint_request_body(ttl, max_uses).is_ok(),
                "ttl={ttl} max_uses={max_uses:?} is accepted by the relay"
            );
        }
    }

    #[test]
    fn mint_body_rejects_out_of_range_input_locally() {
        for (ttl, max_uses) in [
            (MIN_INVITE_TTL_SECS - 1, None),
            (MAX_INVITE_TTL_SECS + 1, None),
            (DEFAULT_INVITE_TTL_SECS, Some(0)),
            (DEFAULT_INVITE_TTL_SECS, Some(-1)),
            (DEFAULT_INVITE_TTL_SECS, Some(MAX_INVITE_USES + 1)),
        ] {
            let err = mint_request_body(ttl, max_uses)
                .expect_err("out-of-range mint request must not reach the relay");
            assert!(
                matches!(err, crate::error::CliError::Usage(_)),
                "bad flag values are usage errors (exit 1), got {err:?}"
            );
        }
    }

    #[test]
    fn claim_body_trims_surrounding_whitespace() {
        let body = claim_request_body("  v2.abc123\n", None).expect("valid claim request");
        assert_eq!(body, serde_json::json!({ "code": "v2.abc123" }));
    }

    #[test]
    fn claim_body_includes_policy_receipt_when_set() {
        let body = claim_request_body("v2.abc123", Some("receipt.mac")).expect("valid claim");
        assert_eq!(
            body,
            serde_json::json!({ "code": "v2.abc123", "policy_receipt": "receipt.mac" })
        );
    }

    #[test]
    fn claim_body_rejects_empty_and_whitespace_split_codes() {
        for code in [
            "",
            "   ",
            "v2.abc 123",
            "https://relay.example/invite/v2.abc",
        ] {
            // The URL form contains no whitespace, so only the empty and
            // interior-whitespace cases are rejected here; the URL is left to
            // the relay, which answers `invite_invalid`.
            let result = claim_request_body(code, None);
            if code.contains(char::is_whitespace) || code.trim().is_empty() {
                assert!(
                    matches!(result, Err(crate::error::CliError::Usage(_))),
                    "{code:?} must be a usage error"
                );
            } else {
                assert!(
                    result.is_ok(),
                    "{code:?} is forwarded to the relay verbatim"
                );
            }
        }
    }

    #[test]
    fn claim_body_rejects_blank_policy_receipt() {
        let err = claim_request_body("v2.abc123", Some("  "))
            .expect_err("a blank receipt is a flag mistake, not a relay decision");
        assert!(matches!(err, crate::error::CliError::Usage(_)));
    }
}
