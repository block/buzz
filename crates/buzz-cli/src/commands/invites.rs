//! `buzz invites` — mint, claim, and accept the join policy for relay invites.
//!
//! Invite operations are REST calls, not Nostr events: invite codes are opaque
//! relay-issued credentials rather than public events, and
//! `POST /api/invites/claim` is deliberately exempt from the relay-membership
//! gate — the whole point is that the caller is not a member yet. That makes
//! it the only onboarding path for a fresh identity on a closed relay;
//! `buzz channels join` publishes a kind:9021 join *request*, which a closed
//! relay rejects with `403 relay_membership_required`.
//!
//! - `mint` — `POST /api/invites`, NIP-98 signed, owner/admin only.
//!   Non-idempotent: every call mints a new live credential, so it is sent
//!   exactly once (see [`BuzzClient::post_authed_once`]).
//! - `claim` — `POST /api/invites/claim`, NIP-98 signed by the *joining*
//!   pubkey. Effectively idempotent: re-claiming an already-redeemed code
//!   returns `already_member`, so the standard retry policy applies. One
//!   caveat — the relay checks expiry before existing membership, so a retry
//!   that lands after the code expires reports `invite_expired` even though
//!   the earlier attempt joined. Re-run `buzz channels list` to confirm
//!   membership before treating that as a failure.
//! - `policy` — `GET /api/join-policy`, public and unauthenticated. Answers
//!   `{}` (not 404) when the operator configured no policy.
//! - `accept-policy` — `POST /api/invites/accept-policy`, also unauthenticated:
//!   the receipt it returns is a MAC over `(code, policy_version)`, bound to
//!   the invite rather than to a pubkey, so signing it would prove nothing the
//!   subsequent claim does not already prove.
//!
//! **Acceptance is never automatic.** `accept-policy` requires the caller to
//! name the exact `--policy-version` they read and, where the operator demands
//! an age attestation, to pass `--age-confirmed` explicitly. Neither is
//! inferred from the policy document: both are assertions about a human, and
//! the CLI has no standing to make them. `policy` prints the full terms and
//! privacy Markdown so that human has something to read; the relay also serves
//! the same documents as browser pages at `/api/join-policy/terms` and
//! `/api/join-policy/privacy`.
//!
//! Invite codes and policy receipts are bearer credentials — holding one is
//! the whole authorization. As argv they are written to shell history and are
//! readable from `ps` by any process on the host, so every credential argument
//! accepts the CLI's standard `-` stdin sentinel (`--code -`,
//! `--policy-receipt -`) and that is the documented, preferred form. stdin is
//! one stream, so at most one argument per command may use it.
//!
//! The community (tenant) is selected by the relay host in `--relay` /
//! `BUZZ_RELAY_URL`; a code minted for one community is rejected on another.

use buzz_core::invite::{MAX_INVITE_TTL_SECS, MAX_INVITE_USES, MIN_INVITE_TTL_SECS};

use crate::client::BuzzClient;
use crate::error::CliError;
use crate::validate::read_secret_or_stdin;
use crate::InvitesCmd;

/// Relay path for minting an invite code.
const MINT_PATH: &str = "/api/invites";
/// Relay path for claiming an invite code.
const CLAIM_PATH: &str = "/api/invites/claim";
/// Relay path for the public join-policy document.
const JOIN_POLICY_PATH: &str = "/api/join-policy";
/// Relay path that exchanges explicit acceptance for an invite-bound receipt.
const ACCEPT_POLICY_PATH: &str = "/api/invites/accept-policy";

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

/// The relay's configured join policy, as returned by `GET /api/join-policy`.
#[derive(Debug, Clone, PartialEq, Eq)]
struct JoinPolicy {
    /// Content-derived revision identifier that receipts are bound to.
    version: String,
    /// Whether the operator requires a minimum-age attestation.
    age_attestation_required: bool,
    /// Terms of Service document, when the operator configured one.
    terms_markdown: Option<String>,
    /// Privacy Policy document, when the operator configured one.
    privacy_markdown: Option<String>,
}

/// Parse `GET /api/join-policy`.
///
/// `Ok(None)` means the relay has no join policy: the endpoint answers `{}`
/// rather than 404, so an absent `policy` object is the documented "not
/// configured" signal, not a malformed response.
///
/// A `policy` object without `age_attestation_required` fails *closed* — a
/// field we cannot read must never silently downgrade an age gate. The worst
/// case is an unnecessary `--age-confirmed`; the relay re-checks either way.
fn parse_join_policy(raw: &str) -> Result<Option<JoinPolicy>, CliError> {
    let value: serde_json::Value = serde_json::from_str(raw)
        .map_err(|e| CliError::Other(format!("relay returned invalid join-policy JSON: {e}")))?;
    let Some(policy) = value.get("policy") else {
        return Ok(None);
    };
    let version = policy
        .get("version")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| {
            CliError::Other("relay join policy is missing a `version` string".to_string())
        })?;
    let markdown = |field: &str| {
        policy
            .get(field)
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
    };
    Ok(Some(JoinPolicy {
        version: version.to_string(),
        age_attestation_required: policy
            .get("age_attestation_required")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(true),
        terms_markdown: markdown("terms_markdown"),
        privacy_markdown: markdown("privacy_markdown"),
    }))
}

/// Render the join policy as this command's JSON contract.
///
/// The Markdown is emitted verbatim so the caller can read what they would be
/// accepting — `buzz invites policy | jq -r .terms_markdown`. An unconfigured
/// relay is a normal outcome, not an error: it reports
/// `{"configured": false}` and exits 0.
fn policy_output(policy: Option<&JoinPolicy>) -> serde_json::Value {
    match policy {
        None => serde_json::json!({ "configured": false }),
        Some(policy) => serde_json::json!({
            "configured": true,
            "version": policy.version,
            "age_attestation_required": policy.age_attestation_required,
            "terms_markdown": policy.terms_markdown,
            "privacy_markdown": policy.privacy_markdown,
        }),
    }
}

/// Validate an invite-code argument, returning the trimmed token.
///
/// Codes are trimmed because they are pasted from links, chat messages, and
/// `--code -` shell plumbing, all of which pick up surrounding whitespace.
/// Interior whitespace is a genuinely malformed code and is rejected rather
/// than silently repaired.
fn validate_invite_code(flag: &str, code: &str) -> Result<String, CliError> {
    let code = code.trim();
    if code.is_empty() {
        return Err(CliError::Usage(format!("{flag} must not be empty")));
    }
    if code.chars().any(char::is_whitespace) {
        return Err(CliError::Usage(format!(
            "{flag} must not contain whitespace — pass the bare token, not the invite URL"
        )));
    }
    Ok(code.to_string())
}

/// Build the `POST /api/invites/accept-policy` body, refusing to accept on the
/// caller's behalf.
///
/// Three checks run locally, all of them exit-1 usage errors rather than
/// round-trip 400s (the relay answers every one of them with the same opaque
/// `join_policy_not_accepted`, which tells the caller nothing about which
/// input was wrong):
///
/// 1. No configured policy — there is nothing to accept, and `claim` already
///    works without a receipt.
/// 2. `--policy-version` that does not match the relay's current revision —
///    the caller read a stale document, so the acceptance would attest to
///    terms that are no longer in force.
/// 3. `--age-confirmed` missing while the operator requires an attestation.
///    This is the one the CLI must not paper over: it is a claim about a
///    person, and no flag default or policy field can stand in for it.
fn accept_policy_request_body(
    policy: Option<&JoinPolicy>,
    code: &str,
    policy_version: &str,
    age_confirmed: bool,
) -> Result<serde_json::Value, CliError> {
    let Some(policy) = policy else {
        return Err(CliError::Usage(
            "this relay has no join policy configured — there is nothing to accept; \
             run `buzz invites claim --code -` without a receipt"
                .to_string(),
        ));
    };
    let code = validate_invite_code("--code", code)?;
    let policy_version = policy_version.trim();
    if policy_version.is_empty() {
        return Err(CliError::Usage(
            "--policy-version must not be empty — read the current version from \
             `buzz invites policy`"
                .to_string(),
        ));
    }
    if policy_version != policy.version {
        return Err(CliError::Usage(format!(
            "--policy-version {policy_version:?} does not match the relay's current join \
             policy {:?} — the terms changed since you read them; re-read them with \
             `buzz invites policy` and accept the version it reports",
            policy.version
        )));
    }
    if policy.age_attestation_required && !age_confirmed {
        return Err(CliError::Usage(
            "this relay requires an age attestation to join, and the CLI will not make it \
             for you: re-run with --age-confirmed only once a human has read \
             `buzz invites policy` and confirms they meet the stated minimum age"
                .to_string(),
        ));
    }
    Ok(serde_json::json!({
        "code": code,
        "policy_version": policy_version,
        "age_confirmed": age_confirmed,
    }))
}

/// Turn the relay's opaque `403 join_policy_required` into a path forward.
///
/// The relay returns that one string for both "no receipt was sent" and "the
/// receipt does not verify", which leaves a caller on a join-policy relay with
/// no idea that `invites policy` and `invites accept-policy` exist. Only the
/// message is rewritten: the 403 is preserved so the exit code stays 3 and
/// scripts keyed to relay status codes are unaffected.
fn explain_claim_error(err: CliError, receipt_passed: bool) -> CliError {
    let CliError::Relay { status: 403, body } = &err else {
        return err;
    };
    if !body.contains("join_policy_required") {
        return err;
    }
    let guidance = if receipt_passed {
        "this policy receipt was rejected — it is bound to a different invite code, or to a \
         policy version the relay has since replaced. Re-read `buzz invites policy`, then \
         `buzz invites accept-policy --code - --policy-version <VERSION>` for the version it \
         reports"
    } else {
        "this relay requires accepting its join policy before an invite can be claimed. \
         Read it with `buzz invites policy`, mint a receipt with \
         `buzz invites accept-policy --code - --policy-version <VERSION>`, then re-run \
         `buzz invites claim` with `--policy-receipt -`"
    };
    CliError::Relay {
        status: 403,
        body: format!("{body}: {guidance}"),
    }
}

/// Reject a command that asks stdin for more than one credential.
///
/// `--code -` and `--policy-receipt -` both read the same single stream, so
/// the second would either block or read the tail of the first. Catching it
/// here turns a hang or a mangled token into an exit-1 usage error naming both
/// flags.
fn check_single_stdin_arg(args: &[(&str, &str)]) -> Result<(), CliError> {
    let piped: Vec<&str> = args
        .iter()
        .filter(|(_, value)| *value == "-")
        .map(|(flag, _)| *flag)
        .collect();
    if piped.len() > 1 {
        return Err(CliError::Usage(format!(
            "only one argument may read stdin, but {} both did — \
             pipe one and pass the other as a value",
            piped.join(" and ")
        )));
    }
    Ok(())
}

/// Build the `POST /api/invites/claim` body.
fn claim_request_body(
    code: &str,
    policy_receipt: Option<&str>,
) -> Result<serde_json::Value, CliError> {
    let code = validate_invite_code("--code", code)?;
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
    let resp = client
        .post_authed(CLAIM_PATH, &body)
        .await
        .map_err(|e| explain_claim_error(e, policy_receipt.is_some()))?;
    println!("{resp}");
    Ok(())
}

/// Fetch the relay's join policy. Unauthenticated — a caller who is not a
/// member yet has to be able to read the terms before agreeing to them.
async fn fetch_join_policy(client: &BuzzClient) -> Result<Option<JoinPolicy>, CliError> {
    let raw = client.get_public(JOIN_POLICY_PATH).await?;
    parse_join_policy(&raw)
}

async fn cmd_policy(client: &BuzzClient) -> Result<(), CliError> {
    let policy = fetch_join_policy(client).await?;
    println!("{}", policy_output(policy.as_ref()));
    Ok(())
}

async fn cmd_accept_policy(
    client: &BuzzClient,
    code: &str,
    policy_version: &str,
    age_confirmed: bool,
) -> Result<(), CliError> {
    // Read the live policy first: it is what makes "you accepted version X"
    // checkable locally, and it is where `age_attestation_required` comes from.
    let policy = fetch_join_policy(client).await?;
    let body = accept_policy_request_body(policy.as_ref(), code, policy_version, age_confirmed)?;
    let resp = client.post_public(ACCEPT_POLICY_PATH, &body).await?;
    println!("{resp}");
    Ok(())
}

pub async fn dispatch(cmd: InvitesCmd, client: &BuzzClient) -> Result<(), CliError> {
    match cmd {
        InvitesCmd::Mint { ttl_secs, max_uses } => cmd_mint(client, ttl_secs, max_uses).await,
        InvitesCmd::Claim {
            code,
            policy_receipt,
        } => {
            // Resolve the bearer credentials before anything else: `-` reads
            // stdin, which is the form callers should prefer over argv.
            check_single_stdin_arg(&[
                ("--code", &code),
                ("--policy-receipt", policy_receipt.as_deref().unwrap_or("")),
            ])?;
            let code = read_secret_or_stdin(&code, "--code")?;
            let policy_receipt = policy_receipt
                .as_deref()
                .map(|receipt| read_secret_or_stdin(receipt, "--policy-receipt"))
                .transpose()?;
            cmd_claim(client, &code, policy_receipt.as_deref()).await
        }
        InvitesCmd::Policy => cmd_policy(client).await,
        InvitesCmd::AcceptPolicy {
            code,
            policy_version,
            age_confirmed,
        } => {
            let code = read_secret_or_stdin(&code, "--code")?;
            cmd_accept_policy(client, &code, &policy_version, age_confirmed).await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        accept_policy_request_body, check_single_stdin_arg, claim_request_body,
        explain_claim_error, mint_request_body, parse_join_policy, policy_output, JoinPolicy,
    };
    use crate::error::CliError;
    use buzz_core::invite::{
        DEFAULT_INVITE_TTL_SECS, MAX_INVITE_TTL_SECS, MAX_INVITE_USES, MIN_INVITE_TTL_SECS,
    };

    /// A fully configured policy, matching the relay's response shape.
    fn configured_policy(age_attestation_required: bool) -> JoinPolicy {
        JoinPolicy {
            version: "v1-abc".to_string(),
            age_attestation_required,
            terms_markdown: Some("# Terms\nBe kind.".to_string()),
            privacy_markdown: Some("# Privacy\nWe keep your events.".to_string()),
        }
    }

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

    // --- join policy ---

    #[test]
    fn parse_join_policy_reads_every_field() {
        let policy = parse_join_policy(
            r##"{"policy":{"terms_markdown":"# Terms","privacy_markdown":"# Privacy",
                "age_attestation_required":true,"version":"v1-abc"}}"##,
        )
        .expect("valid policy JSON")
        .expect("a configured policy");
        assert_eq!(
            policy,
            JoinPolicy {
                version: "v1-abc".to_string(),
                age_attestation_required: true,
                terms_markdown: Some("# Terms".to_string()),
                privacy_markdown: Some("# Privacy".to_string()),
            }
        );
    }

    #[test]
    fn parse_join_policy_treats_empty_object_as_unconfigured() {
        assert_eq!(
            parse_join_policy("{}").expect("an empty object is a valid response"),
            None,
            "the relay answers {{}} rather than 404 when no policy is configured"
        );
    }

    #[test]
    fn parse_join_policy_allows_absent_documents() {
        let policy = parse_join_policy(
            r#"{"policy":{"age_attestation_required":false,"version":"v2","terms_markdown":null}}"#,
        )
        .expect("valid policy JSON")
        .expect("a configured policy");
        assert_eq!(policy.terms_markdown, None);
        assert_eq!(policy.privacy_markdown, None);
        assert!(!policy.age_attestation_required);
    }

    #[test]
    fn parse_join_policy_fails_closed_on_a_missing_age_field() {
        let policy = parse_join_policy(r#"{"policy":{"version":"v1"}}"#)
            .expect("valid policy JSON")
            .expect("a configured policy");
        assert!(
            policy.age_attestation_required,
            "an unreadable age field must never silently downgrade the attestation"
        );
    }

    #[test]
    fn parse_join_policy_rejects_a_policy_without_a_version() {
        for raw in [
            r#"{"policy":{"age_attestation_required":true}}"#,
            r#"{"policy":{"version":42}}"#,
        ] {
            let err = parse_join_policy(raw).expect_err("a receipt cannot be bound to no version");
            assert!(matches!(err, CliError::Other(_)), "got {err:?}");
        }
    }

    #[test]
    fn parse_join_policy_rejects_non_json() {
        let err = parse_join_policy("<html>502</html>").expect_err("not JSON");
        assert!(matches!(err, CliError::Other(_)), "got {err:?}");
    }

    #[test]
    fn policy_output_reports_an_unconfigured_relay_without_erroring() {
        assert_eq!(
            policy_output(None),
            serde_json::json!({ "configured": false })
        );
    }

    #[test]
    fn policy_output_includes_the_markdown_a_human_must_read() {
        assert_eq!(
            policy_output(Some(&configured_policy(true))),
            serde_json::json!({
                "configured": true,
                "version": "v1-abc",
                "age_attestation_required": true,
                "terms_markdown": "# Terms\nBe kind.",
                "privacy_markdown": "# Privacy\nWe keep your events.",
            })
        );
    }

    // --- accept-policy ---

    #[test]
    fn accept_policy_body_carries_the_explicit_acceptance() {
        let body = accept_policy_request_body(
            Some(&configured_policy(true)),
            "  v2.abc123\n",
            "v1-abc",
            true,
        )
        .expect("an explicit, current, age-confirmed acceptance");
        assert_eq!(
            body,
            serde_json::json!({
                "code": "v2.abc123",
                "policy_version": "v1-abc",
                "age_confirmed": true,
            })
        );
    }

    #[test]
    fn accept_policy_body_sends_age_confirmed_false_when_not_required() {
        let body =
            accept_policy_request_body(Some(&configured_policy(false)), "v2.abc", "v1-abc", false)
                .expect("no attestation is required, so none is asserted");
        assert_eq!(
            body["age_confirmed"],
            serde_json::json!(false),
            "the field is always explicit — an omitted flag asserts nothing"
        );
    }

    #[test]
    fn accept_policy_refuses_to_attest_age_on_the_callers_behalf() {
        let err =
            accept_policy_request_body(Some(&configured_policy(true)), "v2.abc", "v1-abc", false)
                .expect_err("--age-confirmed is an assertion about a human and must be explicit");
        let CliError::Usage(message) = err else {
            panic!("a missing attestation is a usage error (exit 1), got {err:?}");
        };
        assert!(
            message.contains("--age-confirmed"),
            "the error must name the flag that unblocks it: {message}"
        );
    }

    #[test]
    fn accept_policy_rejects_a_stale_policy_version() {
        let err =
            accept_policy_request_body(Some(&configured_policy(false)), "v2.abc", "v0-old", false)
                .expect_err("accepting terms you did not read is the failure mode to prevent");
        let CliError::Usage(message) = err else {
            panic!("expected a usage error, got {err:?}");
        };
        assert!(
            message.contains("v0-old") && message.contains("v1-abc"),
            "both the offered and the current version belong in the message: {message}"
        );
    }

    #[test]
    fn accept_policy_rejects_empty_input() {
        for (code, version) in [("", "v1-abc"), ("   ", "v1-abc"), ("v2.abc", "  ")] {
            let err =
                accept_policy_request_body(Some(&configured_policy(false)), code, version, false)
                    .expect_err("blank arguments are flag mistakes");
            assert!(matches!(err, CliError::Usage(_)), "got {err:?}");
        }
    }

    #[test]
    fn accept_policy_explains_that_an_unconfigured_relay_needs_no_receipt() {
        let err = accept_policy_request_body(None, "v2.abc", "v1-abc", true)
            .expect_err("there is nothing to accept");
        let CliError::Usage(message) = err else {
            panic!("expected a usage error, got {err:?}");
        };
        assert!(
            message.contains("claim"),
            "point at the command that does work: {message}"
        );
    }

    // --- claim error guidance ---

    #[test]
    fn claim_error_points_a_receiptless_caller_at_the_policy_commands() {
        let err = explain_claim_error(
            CliError::Relay {
                status: 403,
                body: "join_policy_required".to_string(),
            },
            false,
        );
        let CliError::Relay { status, body } = err else {
            panic!("the relay status must survive so the exit code stays 3");
        };
        assert_eq!(status, 403);
        assert!(
            body.contains("buzz invites policy") && body.contains("buzz invites accept-policy"),
            "a dead end must become a path: {body}"
        );
    }

    #[test]
    fn claim_error_tells_a_receipt_holder_the_receipt_itself_was_rejected() {
        let err = explain_claim_error(
            CliError::Relay {
                status: 403,
                body: "join_policy_required".to_string(),
            },
            true,
        );
        let CliError::Relay { body, .. } = err else {
            panic!("expected a relay error");
        };
        assert!(
            body.contains("rejected"),
            "a caller who already passed a receipt needs a different next step: {body}"
        );
    }

    // --- stdin credentials ---

    #[test]
    fn one_argument_may_read_stdin() {
        for args in [
            vec![("--code", "-"), ("--policy-receipt", "receipt.mac")],
            vec![("--code", "v2.abc"), ("--policy-receipt", "-")],
            vec![("--code", "v2.abc"), ("--policy-receipt", "")],
            vec![("--code", "-")],
        ] {
            assert!(
                check_single_stdin_arg(&args).is_ok(),
                "{args:?} uses stdin at most once"
            );
        }
    }

    #[test]
    fn two_arguments_may_not_both_read_stdin() {
        let err = check_single_stdin_arg(&[("--code", "-"), ("--policy-receipt", "-")])
            .expect_err("stdin is one stream — the second read would block or steal the first");
        let CliError::Usage(message) = err else {
            panic!("expected a usage error, got {err:?}");
        };
        assert!(
            message.contains("--code") && message.contains("--policy-receipt"),
            "name both flags so the caller knows which to change: {message}"
        );
    }

    #[test]
    fn claim_error_leaves_every_other_failure_untouched() {
        for err in [
            CliError::Relay {
                status: 403,
                body: "invite_invalid".to_string(),
            },
            CliError::Relay {
                status: 429,
                body: "join_policy_required".to_string(),
            },
            CliError::Usage("--code must not be empty".to_string()),
        ] {
            let before = format!("{err:?}");
            let after = format!("{:?}", explain_claim_error(err, false));
            assert_eq!(
                before, after,
                "only 403 join_policy_required is rewritten — everything else is the relay's word"
            );
        }
    }
}
