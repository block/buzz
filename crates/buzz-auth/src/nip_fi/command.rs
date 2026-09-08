//! NIP-FI command JWT verification — `VerifyCommandJwt`.
//!
//! Implements the `VerifyCommandJwt` procedure from
//! [NIP-FI.md](../../../../docs/nips/NIP-FI.md) §Admin disconnect API.
//!
//! ## Procedure (steps numbered as in the spec)
//!
//! 1. Bounded decode + `typ` check (`nip-fi-command+jwt` only).
//! 2. Select issuer policy; verify signature with authenticated JWKS.
//! 3. Validate all pure claims: iss, aud, time bounds, method/path/cmd,
//!    target_pubkey, and until ceiling.
//! 4. Principal authorization (issuer-configured authorized `sub` list).
//! 5. Signed-target / request-body agreement.
//! 6. Atomic jti reservation + deny-entry insertion (both-or-neither).
//! 7. Return [`CommandResult`].
//!
//! Fail-closed: any failure returns an error without side effects.  The jti is
//! burned and the deny entry is inserted only on success.

use chrono::{DateTime, Utc};
use jsonwebtoken::{decode, Algorithm, DecodingKey, Validation};
use nostr::PublicKey;
use serde_json::{Map, Value};

use super::config::{IssuerPolicy, IssuerRegistry, MAX_SUBJECT_BYTES, MAX_TOKEN_BYTES};
use super::deny_map::{NipFiDenyMap, ReserveError};
use super::verifier::{
    enforce_compact_structure, enforce_signature_shape, parse_header, parse_numeric_date,
    parse_unique_claims, select_unique_jwk, validate_jwk, AssertionKeySet, IssuerKeySource,
    VerifierError,
};

/// The expected `typ` value for command JWTs ([NIP-FI.md §Command JWT]).
pub const COMMAND_JWT_TYP: &str = "nip-fi-command+jwt";
/// The expected `cmd` claim value.
const COMMAND_CMD: &str = "disconnect";
/// Normative upper bound on `maximum_command_age_seconds` per the spec.
pub const MAX_COMMAND_AGE_SECONDS: u64 = 60;

// ── Per-issuer command policy ─────────────────────────────────────────────────

/// The per-issuer configuration additions required by the command API.
///
/// These supplement the base [`IssuerPolicy`]: `maximum_command_age` and the
/// set of authorized issuer principals (the `sub` values allowed to send
/// commands).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandIssuerPolicy {
    issuer: String,
    /// `0 < maximum_command_age_seconds <= 60`.
    maximum_command_age_seconds: u64,
    /// Non-empty set of authorized `sub` values.
    authorized_principals: Vec<String>,
    /// Hard ceiling on the number of live deny entries for this issuer.
    deny_set_capacity: usize,
}

/// Why a [`CommandIssuerPolicy`] could not be constructed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CommandPolicyError {
    /// `maximum_command_age_seconds` was 0 or > 60 (normative bound).
    #[error("maximum_command_age_seconds must be in [1, 60]")]
    InvalidCommandAge,
    /// The authorized principals list was empty.
    #[error("authorized_principals must be non-empty")]
    EmptyAuthorizedPrincipals,
    /// A principal string was empty or too long.
    #[error("an authorized principal value is invalid")]
    InvalidPrincipal,
    /// `deny_set_capacity` was 0.
    #[error("deny_set_capacity must be positive")]
    ZeroCapacity,
    /// The issuer string was empty.
    #[error("issuer must be non-empty")]
    EmptyIssuer,
}

impl CommandIssuerPolicy {
    /// Validate and construct a command policy.
    pub fn new(
        issuer: String,
        maximum_command_age_seconds: u64,
        authorized_principals: Vec<String>,
        deny_set_capacity: usize,
    ) -> Result<Self, CommandPolicyError> {
        if issuer.is_empty() {
            return Err(CommandPolicyError::EmptyIssuer);
        }
        if maximum_command_age_seconds == 0 || maximum_command_age_seconds > MAX_COMMAND_AGE_SECONDS
        {
            return Err(CommandPolicyError::InvalidCommandAge);
        }
        if authorized_principals.is_empty() {
            return Err(CommandPolicyError::EmptyAuthorizedPrincipals);
        }
        if authorized_principals
            .iter()
            .any(|p| p.is_empty() || p.len() > MAX_SUBJECT_BYTES)
        {
            return Err(CommandPolicyError::InvalidPrincipal);
        }
        if deny_set_capacity == 0 {
            return Err(CommandPolicyError::ZeroCapacity);
        }
        Ok(Self {
            issuer,
            maximum_command_age_seconds,
            authorized_principals,
            deny_set_capacity,
        })
    }

    /// The exact `iss` this policy applies to.
    pub fn issuer(&self) -> &str {
        &self.issuer
    }

    /// `0 < maximum_command_age_seconds <= 60`.
    pub const fn maximum_command_age_seconds(&self) -> u64 {
        self.maximum_command_age_seconds
    }

    /// Non-empty set of authorized `sub` values.
    pub fn authorized_principals(&self) -> &[String] {
        &self.authorized_principals
    }

    /// Hard ceiling on the number of live deny entries for this issuer.
    pub const fn deny_set_capacity(&self) -> usize {
        self.deny_set_capacity
    }
}

// ── Command verifier ──────────────────────────────────────────────────────────

/// The sealed result of a successful `VerifyCommandJwt` call.
///
/// Side effects (jti reservation + deny entry) have been committed atomically
/// before this is returned.  The caller should proceed to close matching
/// sessions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandResult {
    /// The target pubkey from the signed JWT (and verified against the body).
    pub target_pubkey: PublicKey,
    /// Issuer URI of the authorized caller.
    pub caller_iss: String,
    /// `sub` of the authorized caller.
    pub caller_sub: String,
    /// The `until` timestamp from the signed JWT.
    pub until: DateTime<Utc>,
}

/// Errors from [`CommandVerifier::verify`].
///
/// Each variant maps to an exact HTTP status and response body per the spec.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CommandError {
    /// Malformed, invalid, or expired command JWT → 403.
    #[error("evidence rejected")]
    EvidenceRejected,
    /// Principal not authorized; signed-target mismatch; replayed jti → 403.
    #[error("authorization denied")]
    AuthorizationDenied,
    /// Per-issuer deny-set capacity exceeded → 503.  Neither the jti nor the
    /// deny entry was recorded; the caller may safely retry the same command.
    #[error("deny set full")]
    DenySetFull,
    /// JWKS snapshot unavailable → 503.
    #[error("authorization unavailable")]
    AuthorizationUnavailable,
    /// `until` exceeds the allowed ceiling → 400.
    #[error("until exceeds ceiling")]
    UntilExceedsCeiling,
    /// Malformed request body → 400.
    #[error("malformed request")]
    MalformedRequest,
}

impl CommandError {
    /// HTTP status code for the command API endpoint response.
    pub const fn http_status(self) -> u16 {
        match self {
            Self::EvidenceRejected | Self::AuthorizationDenied => 403,
            Self::DenySetFull | Self::AuthorizationUnavailable => 503,
            Self::UntilExceedsCeiling | Self::MalformedRequest => 400,
        }
    }

    /// Spec-exact response body bytes (trailing `\n` included).
    pub const fn response_body(self) -> &'static str {
        match self {
            Self::EvidenceRejected => "evidence rejected\n",
            Self::AuthorizationDenied => "authorization denied\n",
            Self::DenySetFull => "deny set full\n",
            Self::AuthorizationUnavailable => "authorization unavailable\n",
            Self::UntilExceedsCeiling | Self::MalformedRequest => "bad request\n",
        }
    }
}

/// The NIP-FI command JWT verifier.
///
/// Holds a reference to the shared issuer registry (for policy + JWKS lookup),
/// the key source, the per-issuer command policies, and the deny map it writes
/// to.
///
/// `S` must be `Clone` so the verifier can be shared cheaply via `Arc::clone`.
pub struct CommandVerifier<S: IssuerKeySource> {
    registry: IssuerRegistry,
    key_source: S,
    /// Indexed by exact `iss`.
    command_policies: std::collections::HashMap<String, CommandIssuerPolicy>,
    deny_map: NipFiDenyMap,
}

impl<S: IssuerKeySource + Clone> CommandVerifier<S> {
    /// Construct a command verifier.
    ///
    /// `command_policies` must cover every issuer that may send commands;
    /// an unlisted issuer is rejected as `EvidenceRejected`.
    pub fn new(
        registry: IssuerRegistry,
        key_source: S,
        command_policies: Vec<CommandIssuerPolicy>,
        deny_map: NipFiDenyMap,
    ) -> Self {
        let map = command_policies
            .into_iter()
            .map(|p| (p.issuer.clone(), p))
            .collect();
        Self {
            registry,
            key_source,
            command_policies: map,
            deny_map,
        }
    }

    /// A shared reference to the deny map this verifier writes to.
    ///
    /// S5 (HTTP enforcement) reads this same map from the shared `AppState`
    /// without going through the verifier.  The reference here is for callers
    /// that need to pass the map into the WS admission check.
    pub fn deny_map(&self) -> &NipFiDenyMap {
        &self.deny_map
    }

    /// Execute `VerifyCommandJwt` at current clock time.
    ///
    /// Parameters:
    /// * `token`          — compact JWS from `Nostr-Federated-Identity: Bearer`.
    /// * `request_method` — HTTP method (expected `"POST"`).
    /// * `request_path`   — HTTP path (expected `"/api/nip-fi/disconnect"`).
    /// * `body_pubkey`    — the `pubkey` field parsed from the JSON body.
    ///
    /// On `Ok`, the jti is reserved and the deny entry is inserted.
    /// On `Err`, no side effects have occurred (or, on `DenySetFull`, neither
    /// mutation was applied, so retry is safe).
    pub fn verify(
        &self,
        token: &str,
        request_method: &str,
        request_path: &str,
        body_pubkey: &PublicKey,
    ) -> Result<CommandResult, CommandError> {
        self.verify_at(token, request_method, request_path, body_pubkey, Utc::now())
    }

    /// Verify with an injectable clock for deterministic testing.
    pub fn verify_at(
        &self,
        token: &str,
        request_method: &str,
        request_path: &str,
        body_pubkey: &PublicKey,
        now: DateTime<Utc>,
    ) -> Result<CommandResult, CommandError> {
        // ── Step 1: bounded decode + typ check ───────────────────────────────
        if token.is_empty() || token.len() > MAX_TOKEN_BYTES {
            return Err(CommandError::EvidenceRejected);
        }
        enforce_compact_structure(token).map_err(|_| CommandError::EvidenceRejected)?;
        let header = parse_header(token).map_err(|_| CommandError::EvidenceRejected)?;
        enforce_signature_shape(token).map_err(|_| CommandError::EvidenceRejected)?;

        // typ MUST be exactly "nip-fi-command+jwt".
        if header.typ.as_deref() != Some(COMMAND_JWT_TYP) {
            return Err(CommandError::EvidenceRejected);
        }

        // ── Step 2: select issuer policy; verify signature ────────────────────
        let claims = parse_unique_claims(token).map_err(|_| CommandError::EvidenceRejected)?;

        let signed_iss = claim_str(&claims, "iss").ok_or(CommandError::EvidenceRejected)?;

        let base_policy = self
            .registry
            .policy_for_issuer(signed_iss)
            .ok_or(CommandError::EvidenceRejected)?;

        let cmd_policy = self
            .command_policies
            .get(signed_iss)
            .ok_or(CommandError::EvidenceRejected)?;

        if !base_policy.algorithms().contains(&header.algorithm) {
            return Err(CommandError::EvidenceRejected);
        }

        let key_set = self
            .key_source
            .key_set(base_policy.issuer())
            .ok_or(CommandError::AuthorizationUnavailable)?;
        if key_set.issuer() != base_policy.issuer() {
            return Err(CommandError::EvidenceRejected);
        }

        verify_jwt_signature(token, base_policy, &key_set, header.algorithm).map_err(
            |e| match e {
                VerifierError::KeySourceUnavailable | VerifierError::StatusWitnessUnavailable => {
                    CommandError::AuthorizationUnavailable
                }
                _ => CommandError::EvidenceRejected,
            },
        )?;

        // ── Step 3: validate pure claims ──────────────────────────────────────

        // aud: at least one of the policy audiences must match.
        let aud_ok = match claims.get("aud") {
            Some(Value::String(s)) => base_policy.audiences().iter().any(|a| a == s),
            Some(Value::Array(arr)) => arr.iter().any(|v| {
                v.as_str()
                    .map(|s| base_policy.audiences().iter().any(|a| a == s))
                    .unwrap_or(false)
            }),
            _ => false,
        };
        if !aud_ok {
            return Err(CommandError::EvidenceRejected);
        }

        // Time bounds.
        let iat = numeric_date(&claims, "iat")?;
        let exp = numeric_date(&claims, "exp")?;
        let skew = chrono::Duration::seconds(base_policy.skew_seconds() as i64);
        let max_cmd_age = chrono::Duration::seconds(cmd_policy.maximum_command_age_seconds as i64);
        let iat_plus_cmd_age = iat
            .checked_add_signed(max_cmd_age)
            .ok_or(CommandError::EvidenceRejected)?;

        // now < exp (equality is expired).
        if now >= exp {
            return Err(CommandError::EvidenceRejected);
        }
        // iat <= now + skew.
        let now_plus_skew = now
            .checked_add_signed(skew)
            .ok_or(CommandError::EvidenceRejected)?;
        if iat > now_plus_skew {
            return Err(CommandError::EvidenceRejected);
        }
        // now < iat + maximum_command_age.
        if now >= iat_plus_cmd_age {
            return Err(CommandError::EvidenceRejected);
        }

        // method / path / cmd (exact literal matches required by spec).
        let method = claim_str(&claims, "method").ok_or(CommandError::EvidenceRejected)?;
        let path = claim_str(&claims, "path").ok_or(CommandError::EvidenceRejected)?;
        let cmd = claim_str(&claims, "cmd").ok_or(CommandError::EvidenceRejected)?;
        if method != request_method {
            return Err(CommandError::EvidenceRejected);
        }
        if path != request_path {
            return Err(CommandError::EvidenceRejected);
        }
        if cmd != COMMAND_CMD {
            return Err(CommandError::EvidenceRejected);
        }

        // target_pubkey: lowercase hex of exactly 32 bytes.
        let target_hex =
            claim_str(&claims, "target_pubkey").ok_or(CommandError::EvidenceRejected)?;
        let target_pubkey = parse_hex_pubkey(target_hex).ok_or(CommandError::EvidenceRejected)?;

        // until: NumericDate.
        let until = numeric_date(&claims, "until")?;

        // Validate `until` ceiling: until <= now + skew + maximum_assertion_age.
        let max_assertion_age =
            chrono::Duration::seconds(base_policy.maximum_assertion_age_seconds() as i64);
        let deny_ceiling = now_plus_skew
            .checked_add_signed(max_assertion_age)
            .ok_or(CommandError::EvidenceRejected)?;
        if until > deny_ceiling {
            return Err(CommandError::UntilExceedsCeiling);
        }

        // jti: must be present and non-empty.
        let jti = claim_str(&claims, "jti").ok_or(CommandError::EvidenceRejected)?;

        // ── Step 4: principal authorization ───────────────────────────────────
        // AssertAuthorizedIssuerPrincipal(claims.iss, claims.sub).
        let sub = claim_str(&claims, "sub").ok_or(CommandError::EvidenceRejected)?;
        if !cmd_policy.authorized_principals.iter().any(|p| p == sub) {
            return Err(CommandError::AuthorizationDenied);
        }

        // ── Step 5: signed-target / request-body agreement ───────────────────
        if &target_pubkey != body_pubkey {
            return Err(CommandError::AuthorizationDenied);
        }

        // ── Steps 6+7: atomic jti reservation + deny-entry insertion ─────────
        //
        // effective_expiry = min(exp, iat + maximum_command_age).
        let effective_expiry = exp.min(iat_plus_cmd_age);

        match self.deny_map.atomic_reserve_and_insert(
            base_policy.issuer(),
            jti,
            effective_expiry,
            &target_pubkey,
            until,
            now,
        ) {
            Ok(()) => {}
            Err(ReserveError::JtiAlreadyReserved) => return Err(CommandError::AuthorizationDenied),
            Err(ReserveError::CapacityExceeded) => return Err(CommandError::DenySetFull),
        }

        Ok(CommandResult {
            target_pubkey,
            caller_iss: base_policy.issuer().to_owned(),
            caller_sub: sub.to_owned(),
            until,
        })
    }
}

// ── Internal helpers ──────────────────────────────────────────────────────────

fn claim_str<'a>(claims: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    claims.get(key)?.as_str().filter(|s| !s.is_empty())
}

fn numeric_date(claims: &Map<String, Value>, key: &str) -> Result<DateTime<Utc>, CommandError> {
    let value = claims.get(key).ok_or(CommandError::EvidenceRejected)?;
    parse_numeric_date(value).map_err(|_| CommandError::EvidenceRejected)
}

fn parse_hex_pubkey(raw: &str) -> Option<PublicKey> {
    if raw.len() != 64
        || !raw
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    {
        return None;
    }
    PublicKey::from_hex(raw).ok()
}

/// Verify the JWS signature using the JWKS from `key_set`.
///
/// This reuses the same `select_unique_jwk_pub` / `validate_jwk_pub` helpers as
/// the assertion verifier so key selection and validation semantics are identical.
fn verify_jwt_signature(
    token: &str,
    policy: &IssuerPolicy,
    key_set: &AssertionKeySet,
    algorithm: Algorithm,
) -> Result<(), VerifierError> {
    use base64::Engine;

    // Decode the header segment to extract `kid`.
    let header_seg = token
        .split('.')
        .next()
        .ok_or(VerifierError::MalformedToken)?;
    let header_bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(header_seg)
        .map_err(|_| VerifierError::MalformedToken)?;
    let header_obj: serde_json::Map<String, Value> =
        serde_json::from_slice(&header_bytes).map_err(|_| VerifierError::MalformedToken)?;
    let kid = header_obj
        .get("kid")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or(VerifierError::MissingKeyId)?;

    let jwk = select_unique_jwk(key_set.jwks(), kid)?;
    validate_jwk(jwk, algorithm)?;
    let key = DecodingKey::from_jwk(jwk).map_err(|_| VerifierError::InvalidKey)?;

    let mut validation = Validation::new(algorithm);
    validation.set_issuer(&[policy.issuer()]);
    validation.set_audience(policy.audiences());
    validation.set_required_spec_claims(&["exp", "iat", "iss", "aud"]);
    validation.validate_exp = false;
    validation.validate_nbf = false;
    decode::<Map<String, Value>>(token, &key, &validation)
        .map_err(|_| VerifierError::InvalidSignatureOrClaims)?;
    Ok(())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nip_fi::deny_map::IssuerCapacity;
    use chrono::{Duration, Utc};

    // ── CommandIssuerPolicy validation ────────────────────────────────────────
    // These tests exercise the construction-time validation contract:
    // every misconfiguration is caught before any command is processed.

    #[test]
    fn command_policy_rejects_zero_age() {
        let result = CommandIssuerPolicy::new(
            "https://issuer.example.com".into(),
            0,
            vec!["admin@example.com".into()],
            1000,
        );
        assert_eq!(
            result,
            Err(CommandPolicyError::InvalidCommandAge),
            "maximum_command_age=0 must be rejected"
        );
    }

    #[test]
    fn command_policy_rejects_age_exceeding_60() {
        let result = CommandIssuerPolicy::new(
            "https://issuer.example.com".into(),
            61,
            vec!["admin@example.com".into()],
            1000,
        );
        assert_eq!(
            result,
            Err(CommandPolicyError::InvalidCommandAge),
            "maximum_command_age=61 must be rejected (normative bound is 60)"
        );
    }

    #[test]
    fn command_policy_accepts_max_age_60() {
        CommandIssuerPolicy::new(
            "https://issuer.example.com".into(),
            60,
            vec!["admin@example.com".into()],
            1000,
        )
        .expect("maximum_command_age=60 must be accepted (normative upper bound)");
    }

    #[test]
    fn command_policy_accepts_min_age_1() {
        CommandIssuerPolicy::new(
            "https://issuer.example.com".into(),
            1,
            vec!["admin@example.com".into()],
            1000,
        )
        .expect("maximum_command_age=1 must be accepted (normative lower bound)");
    }

    #[test]
    fn command_policy_rejects_empty_principals() {
        let result =
            CommandIssuerPolicy::new("https://issuer.example.com".into(), 30, vec![], 1000);
        assert_eq!(
            result,
            Err(CommandPolicyError::EmptyAuthorizedPrincipals),
            "empty authorized_principals must be rejected"
        );
    }

    #[test]
    fn command_policy_rejects_zero_capacity() {
        let result = CommandIssuerPolicy::new(
            "https://issuer.example.com".into(),
            30,
            vec!["admin@example.com".into()],
            0,
        );
        assert_eq!(
            result,
            Err(CommandPolicyError::ZeroCapacity),
            "deny_set_capacity=0 must be rejected"
        );
    }

    #[test]
    fn command_policy_rejects_empty_issuer() {
        let result =
            CommandIssuerPolicy::new(String::new(), 30, vec!["admin@example.com".into()], 1000);
        assert_eq!(
            result,
            Err(CommandPolicyError::EmptyIssuer),
            "empty issuer must be rejected"
        );
    }

    // ── CommandError HTTP contract ────────────────────────────────────────────
    // Spec-exact status codes and bodies from the disconnect API response table.

    #[test]
    fn command_error_http_contract_is_spec_exact() {
        // Evidence failures → 403 "evidence rejected\n"
        assert_eq!(CommandError::EvidenceRejected.http_status(), 403);
        assert_eq!(
            CommandError::EvidenceRejected.response_body(),
            "evidence rejected\n"
        );

        // Authorization failures → 403 "authorization denied\n"
        assert_eq!(CommandError::AuthorizationDenied.http_status(), 403);
        assert_eq!(
            CommandError::AuthorizationDenied.response_body(),
            "authorization denied\n"
        );

        // Capacity → 503 "deny set full\n"
        assert_eq!(CommandError::DenySetFull.http_status(), 503);
        assert_eq!(CommandError::DenySetFull.response_body(), "deny set full\n");

        // JWKS unavailable → 503 "authorization unavailable\n"
        assert_eq!(CommandError::AuthorizationUnavailable.http_status(), 503);
        assert_eq!(
            CommandError::AuthorizationUnavailable.response_body(),
            "authorization unavailable\n"
        );

        // until ceiling / malformed → 400 "bad request\n"
        assert_eq!(CommandError::UntilExceedsCeiling.http_status(), 400);
        assert_eq!(
            CommandError::UntilExceedsCeiling.response_body(),
            "bad request\n"
        );
        assert_eq!(CommandError::MalformedRequest.http_status(), 400);
        assert_eq!(
            CommandError::MalformedRequest.response_body(),
            "bad request\n"
        );
    }

    // ── Mutation evidence: time-bound oracles ─────────────────────────────────
    //
    // The tests below exercise `verify_at` via real ES256-signed command JWTs.
    // The same test key and JWKS helpers as `verifier/tests.rs` are used so
    // key-selection and claim-validation semantics are identical.

    // FI-TRACE-DENY-SET oracle: past-until inserts with expired value.
    #[test]
    fn past_until_command_creates_no_future_denial_when_no_active_entry() {
        use nostr::Keys;

        let deny_map = NipFiDenyMap::new(
            100,
            vec![IssuerCapacity {
                issuer: "https://issuer.example.com".to_owned(),
                capacity: 100,
            }],
        );
        let k = Keys::generate().public_key();
        let now = Utc::now();
        let past = now - Duration::seconds(60);

        // Directly invoke the deny map: past-until on absent entry.
        deny_map
            .atomic_reserve_and_insert(
                "https://issuer.example.com",
                "jti-past",
                past,
                &k,
                past,
                now,
            )
            .expect("past-until on absent entry must succeed (not a capacity error)");

        // is_denied now must be false (entry is immediately expired).
        assert!(
            !deny_map.is_denied("https://issuer.example.com", &k, now),
            "past-until command on absent entry creates no future denial [FI-TRACE-DENY-SET]"
        );
    }

    // FI-TRACE-DENY-SET oracle: both delivery orders → max(until_A, until_B).
    #[test]
    fn deny_set_both_delivery_orders_give_max_until() {
        use nostr::Keys;

        let iss = "https://issuer.example.com";
        let now = Utc::now();
        let k = Keys::generate().public_key();

        // Order 1: longer first, shorter second.
        {
            let m = NipFiDenyMap::new(
                100,
                vec![IssuerCapacity {
                    issuer: iss.to_owned(),
                    capacity: 100,
                }],
            );
            let longer = now + Duration::seconds(600);
            let shorter = now + Duration::seconds(300);
            m.atomic_reserve_and_insert(iss, "jti-a1", longer, &k, longer, now)
                .unwrap();
            m.atomic_reserve_and_insert(iss, "jti-b1", shorter, &k, shorter, now)
                .unwrap();
            // At t = 400s: still denied (longer survives shorter).
            assert!(
                m.is_denied(iss, &k, now + Duration::seconds(400)),
                "Order 1 (longer first): deny at 400s must hold under merge rule"
            );
        }

        // Order 2: shorter first, longer second.
        {
            let m = NipFiDenyMap::new(
                100,
                vec![IssuerCapacity {
                    issuer: iss.to_owned(),
                    capacity: 100,
                }],
            );
            let shorter = now + Duration::seconds(300);
            let longer = now + Duration::seconds(600);
            m.atomic_reserve_and_insert(iss, "jti-a2", shorter, &k, shorter, now)
                .unwrap();
            m.atomic_reserve_and_insert(iss, "jti-b2", longer, &k, longer, now)
                .unwrap();
            // At t = 400s: still denied (shorter did not shorten the longer).
            assert!(
                m.is_denied(iss, &k, now + Duration::seconds(400)),
                "Order 2 (shorter first): deny at 400s must hold — delivery order must not shorten longer deny"
            );
        }
    }

    // ── Full-path CommandVerifier::verify_at tests (real ES256 key) ───────────
    //
    // Uses the same test key material as `verifier/tests.rs` so the key-
    // selection and signature-validation paths are exercised identically.
    // Each test carries a named mutation anchor: the assertion that goes wrong
    // when the guarded code path is removed.

    const TEST_EC_PKCS8_PEM: &str = "-----BEGIN PRIVATE KEY-----\nMIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgcnxDM4EiirH9dHUE\nWZc759TX4s5PAn8kO5ovXSnGxCWhRANCAARFb6ZnsfkqOOXyEhj3KBQphGKF4vTa\nzhebbavbZ1ZoklqkF1cGg+jTO7rONAVEzXvXUWtV6CdDV+rybiVmFP2w\n-----END PRIVATE KEY-----\n";
    const TEST_JWK_X: &str = "RW-mZ7H5Kjjl8hIY9ygUKYRiheL02s4Xm22r22dWaJI";
    const TEST_JWK_Y: &str = "WqQXVwaD6NM7us40BUTNe9dRa1XoJ0NX6vJuJWYU_bA";
    const TEST_KID: &str = "cmd-test-key-1";
    const ISS: &str = "https://idp.example.com";
    const AUD: &str = "https://relay.example.com";
    const PRINCIPAL: &str = "admin@idp.example.com";
    const METHOD: &str = "POST";
    const PATH: &str = "/api/nip-fi/disconnect";

    fn test_jwks_cmd() -> jsonwebtoken::jwk::JwkSet {
        serde_json::from_value(serde_json::json!({
            "keys": [{
                "kty": "EC", "crv": "P-256",
                "use": "sig", "alg": "ES256",
                "kid": TEST_KID,
                "x": TEST_JWK_X,
                "y": TEST_JWK_Y,
            }]
        }))
        .expect("valid test JWKS")
    }

    fn test_issuer_policy() -> IssuerPolicy {
        let contract = crate::nip_fi::jwks::JwksSourceContract::new(
            format!("{ISS}/.well-known/jwks.json"),
            300,
            3600,
        )
        .expect("valid contract");
        IssuerPolicy::new(
            ISS.to_owned(),
            vec![AUD.to_owned()],
            crate::nip_fi::config::TokenClass::DedicatedNipFi,
            crate::nip_fi::config::FreshnessClass::OfflineJwt,
            vec![jsonwebtoken::Algorithm::ES256],
            30,
            3600,
            None,
            contract,
        )
        .expect("valid policy")
    }

    fn test_command_policy() -> CommandIssuerPolicy {
        CommandIssuerPolicy::new(ISS.to_owned(), 30, vec![PRINCIPAL.to_owned()], 1000)
            .expect("valid cmd policy")
    }

    fn test_command_verifier() -> CommandVerifier<crate::nip_fi::verifier::StaticIssuerKeySource> {
        use crate::nip_fi::verifier::{AssertionKeySet, StaticIssuerKeySource};
        let future = Utc::now() + Duration::seconds(3600);
        let key_set = AssertionKeySet::new(ISS.to_owned(), 1, test_jwks_cmd(), future)
            .expect("valid key set");
        let mut registry = IssuerRegistry::new();
        registry.insert(test_issuer_policy());
        let deny_map = NipFiDenyMap::new(
            1000,
            vec![IssuerCapacity {
                issuer: ISS.to_owned(),
                capacity: 1000,
            }],
        );
        CommandVerifier::new(
            registry,
            StaticIssuerKeySource::new([key_set]),
            vec![test_command_policy()],
            deny_map,
        )
    }

    fn now_ts() -> i64 {
        Utc::now().timestamp()
    }

    fn target_key() -> nostr::PublicKey {
        nostr::Keys::generate().public_key()
    }

    /// Mint a real ES256 command JWT with optional claim overrides.
    fn mint_cmd_jwt(
        target: &nostr::PublicKey,
        until_offset_secs: i64,
        overrides: serde_json::Value,
    ) -> String {
        let now = now_ts();
        let mut claims = serde_json::json!({
            "iss": ISS,
            "aud": AUD,
            "sub": PRINCIPAL,
            "iat": now,
            "exp": now + 55,
            "jti": uuid::Uuid::new_v4().to_string(),
            "method": METHOD,
            "path": PATH,
            "cmd": "disconnect",
            "target_pubkey": target.to_hex(),
            "until": now + until_offset_secs,
        });
        if let serde_json::Value::Object(ref ov) = overrides {
            for (k, v) in ov {
                claims[k] = v.clone();
            }
        }
        let mut header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::ES256);
        header.kid = Some(TEST_KID.to_owned());
        header.typ = Some(COMMAND_JWT_TYP.to_owned());
        let key = jsonwebtoken::EncodingKey::from_ec_pem(TEST_EC_PKCS8_PEM.as_bytes())
            .expect("valid EC PEM");
        jsonwebtoken::encode(&header, &claims, &key).expect("sign")
    }

    // ── Happy path ────────────────────────────────────────────────────────────

    #[test]
    fn full_path_happy_path_returns_ok_and_inserts_deny() {
        // Mutation anchor: removing the deny-entry insertion (step 6) makes
        // is_denied return false even after Ok — caught here.
        let cv = test_command_verifier();
        let target = target_key();
        let now = Utc::now();
        let token = mint_cmd_jwt(&target, 300, serde_json::json!({}));
        let result = cv.verify_at(&token, METHOD, PATH, &target, now);
        assert!(result.is_ok(), "happy path must succeed: {result:?}");
        assert!(
            cv.deny_map()
                .is_denied(ISS, &target, now + Duration::seconds(1)),
            "deny entry must be inserted on success [FI-TRACE-DENY-SET]"
        );
    }

    // ── typ / signature ───────────────────────────────────────────────────────

    #[test]
    fn wrong_typ_rejects_as_evidence_rejected() {
        // Mutation anchor: removing the typ check admits at+jwt tokens as commands.
        let cv = test_command_verifier();
        let target = target_key();
        let now = Utc::now();
        let mut header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::ES256);
        header.kid = Some(TEST_KID.to_owned());
        header.typ = Some("at+jwt".to_owned());
        let claims = serde_json::json!({
            "iss": ISS, "aud": AUD, "sub": PRINCIPAL,
            "iat": now_ts(), "exp": now_ts() + 55,
            "jti": uuid::Uuid::new_v4().to_string(),
            "method": METHOD, "path": PATH, "cmd": "disconnect",
            "target_pubkey": target.to_hex(), "until": now_ts() + 300,
        });
        let key = jsonwebtoken::EncodingKey::from_ec_pem(TEST_EC_PKCS8_PEM.as_bytes()).unwrap();
        let token = jsonwebtoken::encode(&header, &claims, &key).unwrap();
        assert_eq!(
            cv.verify_at(&token, METHOD, PATH, &target, now),
            Err(CommandError::EvidenceRejected),
            "wrong typ must be EvidenceRejected"
        );
    }

    #[test]
    fn corrupted_signature_rejects_as_evidence_rejected() {
        // Mutation anchor: removing sig verification admits forged tokens.
        let cv = test_command_verifier();
        let target = target_key();
        let now = Utc::now();
        let mut token = mint_cmd_jwt(&target, 300, serde_json::json!({}));
        let last = token.pop().unwrap();
        token.push(if last == 'A' { 'B' } else { 'A' });
        assert_eq!(
            cv.verify_at(&token, METHOD, PATH, &target, now),
            Err(CommandError::EvidenceRejected),
            "corrupted signature must be EvidenceRejected"
        );
    }

    // ── aud ───────────────────────────────────────────────────────────────────

    #[test]
    fn wrong_aud_rejects_as_evidence_rejected() {
        // Mutation anchor: removing aud check admits tokens for other relays.
        let cv = test_command_verifier();
        let target = target_key();
        let now = Utc::now();
        let token = mint_cmd_jwt(
            &target,
            300,
            serde_json::json!({"aud": "https://other.relay.example.com"}),
        );
        assert_eq!(
            cv.verify_at(&token, METHOD, PATH, &target, now),
            Err(CommandError::EvidenceRejected),
            "wrong aud must be EvidenceRejected"
        );
    }

    // ── Time bounds ───────────────────────────────────────────────────────────

    #[test]
    fn expired_token_rejects_as_evidence_rejected() {
        // Mutation anchor: removing exp check admits expired tokens indefinitely.
        let cv = test_command_verifier();
        let target = target_key();
        let now = Utc::now();
        let past = now_ts() - 600;
        let token = mint_cmd_jwt(
            &target,
            300,
            serde_json::json!({"iat": past, "exp": past + 55}),
        );
        assert_eq!(
            cv.verify_at(&token, METHOD, PATH, &target, now),
            Err(CommandError::EvidenceRejected),
            "expired token must be EvidenceRejected"
        );
    }

    #[test]
    fn future_iat_beyond_skew_rejects() {
        // Mutation anchor: removing iat>now+skew check admits pre-issued tokens.
        let cv = test_command_verifier();
        let target = target_key();
        let now = Utc::now();
        let far_future_iat = now_ts() + 300; // 5 min future, skew=30s
        let token = mint_cmd_jwt(
            &target,
            300,
            serde_json::json!({"iat": far_future_iat, "exp": far_future_iat + 55}),
        );
        assert_eq!(
            cv.verify_at(&token, METHOD, PATH, &target, now),
            Err(CommandError::EvidenceRejected),
            "future iat > now+skew must be EvidenceRejected"
        );
    }

    #[test]
    fn token_older_than_command_age_rejects() {
        // Mutation anchor: removing iat+cmd_age check admits stale commands.
        let cv = test_command_verifier();
        let target = target_key();
        // iat at exactly max_command_age ago (30s) = now >= iat+30 → expired.
        let stale_iat = now_ts() - 30;
        let token = mint_cmd_jwt(
            &target,
            300,
            serde_json::json!({"iat": stale_iat, "exp": stale_iat + 55}),
        );
        let now = Utc::now();
        assert_eq!(
            cv.verify_at(&token, METHOD, PATH, &target, now),
            Err(CommandError::EvidenceRejected),
            "token at iat+max_cmd_age must be EvidenceRejected (equality is expired)"
        );
    }

    // ── method / path / cmd ───────────────────────────────────────────────────

    #[test]
    fn wrong_method_rejects() {
        // Mutation anchor: removing method check admits GET command tokens.
        let cv = test_command_verifier();
        let target = target_key();
        let now = Utc::now();
        let token = mint_cmd_jwt(&target, 300, serde_json::json!({"method": "GET"}));
        assert_eq!(
            cv.verify_at(&token, METHOD, PATH, &target, now),
            Err(CommandError::EvidenceRejected)
        );
    }

    #[test]
    fn wrong_path_rejects() {
        // Mutation anchor: removing path check admits tokens for other endpoints.
        let cv = test_command_verifier();
        let target = target_key();
        let now = Utc::now();
        let token = mint_cmd_jwt(&target, 300, serde_json::json!({"path": "/api/other"}));
        assert_eq!(
            cv.verify_at(&token, METHOD, PATH, &target, now),
            Err(CommandError::EvidenceRejected)
        );
    }

    #[test]
    fn wrong_cmd_value_rejects() {
        // Mutation anchor: removing cmd check admits other command type tokens.
        let cv = test_command_verifier();
        let target = target_key();
        let now = Utc::now();
        let token = mint_cmd_jwt(&target, 300, serde_json::json!({"cmd": "reconnect"}));
        assert_eq!(
            cv.verify_at(&token, METHOD, PATH, &target, now),
            Err(CommandError::EvidenceRejected)
        );
    }

    // ── target_pubkey / body agreement ───────────────────────────────────────

    #[test]
    fn body_target_mismatch_rejects_as_authorization_denied() {
        // Mutation anchor: removing the target==body check lets attackers
        // disconnect a different pubkey than they signed.
        let cv = test_command_verifier();
        let signed_target = target_key();
        let body_target = target_key(); // different
        let now = Utc::now();
        let token = mint_cmd_jwt(&signed_target, 300, serde_json::json!({}));
        assert_eq!(
            cv.verify_at(&token, METHOD, PATH, &body_target, now),
            Err(CommandError::AuthorizationDenied),
            "target/body mismatch must be AuthorizationDenied"
        );
    }

    // ── Authorization ─────────────────────────────────────────────────────────

    #[test]
    fn unauthorized_sub_rejects_as_authorization_denied() {
        // Mutation anchor: removing sub check lets any bearer of a valid JWT
        // issue disconnect commands.
        let cv = test_command_verifier();
        let target = target_key();
        let now = Utc::now();
        let token = mint_cmd_jwt(
            &target,
            300,
            serde_json::json!({"sub": "not-admin@idp.example.com"}),
        );
        assert_eq!(
            cv.verify_at(&token, METHOD, PATH, &target, now),
            Err(CommandError::AuthorizationDenied),
            "unauthorized sub must be AuthorizationDenied"
        );
    }

    // ── until ceiling ─────────────────────────────────────────────────────────

    #[test]
    fn until_exceeds_ceiling_returns_correct_error() {
        // Mutation anchor: removing ceiling check allows unbounded deny duration.
        let cv = test_command_verifier();
        let target = target_key();
        let now = Utc::now();
        let far_future = now_ts() + 10 * 365 * 24 * 3600;
        let token = mint_cmd_jwt(&target, 0, serde_json::json!({"until": far_future}));
        assert_eq!(
            cv.verify_at(&token, METHOD, PATH, &target, now),
            Err(CommandError::UntilExceedsCeiling),
            "until beyond ceiling must be UntilExceedsCeiling"
        );
    }

    // ── jti is the LAST step ─────────────────────────────────────────────────

    #[test]
    fn jti_replay_rejected_after_first_success() {
        // Mutation anchor: moving jti before auth checks would burn the jti
        // on a bad-auth token; replay would be indistinguishable from a new call.
        let cv = test_command_verifier();
        let target = target_key();
        let now = Utc::now();
        let jti = uuid::Uuid::new_v4().to_string();
        let token = mint_cmd_jwt(&target, 300, serde_json::json!({"jti": jti}));
        assert!(cv.verify_at(&token, METHOD, PATH, &target, now).is_ok());
        assert_eq!(
            cv.verify_at(&token, METHOD, PATH, &target, now),
            Err(CommandError::AuthorizationDenied),
            "replayed jti must be AuthorizationDenied"
        );
    }

    // ── 503 does NOT burn the jti ─────────────────────────────────────────────

    #[test]
    fn capacity_503_does_not_burn_jti_retry_succeeds_after_slot_freed() {
        // Mutation anchor: if jti were recorded before the capacity check, a
        // retry after freeing capacity would be spuriously rejected as replay.
        // The spec guarantees: 503 leaves the command replayable.
        //
        // Setup: capacity=1, first command fills the slot.
        // Step 1: second command with jti_b → 503 (capacity full).
        // Step 2: "expire" the first entry by using a future `now` that is
        //   past target_a's until; lazy eviction fires on the next mutation.
        // Step 3: retry with the same jti_b → must succeed (jti was NOT burned).
        use crate::nip_fi::verifier::{AssertionKeySet, StaticIssuerKeySource};

        let future = Utc::now() + Duration::seconds(3600);
        let key_set = AssertionKeySet::new(ISS.to_owned(), 1, test_jwks_cmd(), future)
            .expect("valid key set");
        let mut registry = IssuerRegistry::new();
        registry.insert(test_issuer_policy());
        // capacity = 1: one slot
        let deny_map = NipFiDenyMap::new(
            1,
            vec![IssuerCapacity {
                issuer: ISS.to_owned(),
                capacity: 1,
            }],
        );
        let cv = CommandVerifier::new(
            registry,
            StaticIssuerKeySource::new([key_set]),
            vec![test_command_policy()],
            deny_map,
        );

        let target_a = target_key();
        let target_b = target_key();
        let t0 = Utc::now();

        // Fill the single slot with target_a.  Use until_offset_secs=3 so the
        // deny entry expires at t0+3s, well before t1.
        let token_a = mint_cmd_jwt(&target_a, 3, serde_json::json!({}));
        assert!(cv.verify_at(&token_a, METHOD, PATH, &target_a, t0).is_ok());

        // target_b → 503 (capacity full); jti_b is NOT burned.
        let jti_b = uuid::Uuid::new_v4().to_string();
        let token_b = mint_cmd_jwt(&target_b, 300, serde_json::json!({"jti": jti_b}));
        assert_eq!(
            cv.verify_at(&token_b, METHOD, PATH, &target_b, t0),
            Err(CommandError::DenySetFull),
            "must be DenySetFull (503)"
        );

        // Advance `now` to t0+5s: target_a's entry (until=t0+3) is expired.
        // Lazy eviction fires on the next mutation inside verify_at, freeing
        // the slot.  token_b is still within command age (iat=t0, max=30s).
        let t1 = t0 + Duration::seconds(5);

        // Retry with the SAME jti_b at t1.  Lazy eviction removes target_a's
        // entry, freeing the slot.
        // Must succeed: 503 must NOT have burned jti_b.
        assert!(
            cv.verify_at(&token_b, METHOD, PATH, &target_b, t1).is_ok(),
            "retry with same jti after slot freed must succeed — 503 must NOT burn the jti"
        );
    }

    // ── Signed fractional verify_at witness ──────────────────────────────────
    //
    // Proves that a real ES256-signed command JWT whose `until` NumericDate is
    // fractional (whole seconds + 0.5) passes through CommandVerifier::verify_at
    // with the fractional deadline intact in CommandResult.until, and that the
    // inserted deny entry correctly honours the sub-second boundary.
    //
    // Mandatory reds:
    //  - truncating/zeroing fractional `until` in parse_numeric_date → returned
    //    deadline loses nanos; boundary assertions fail
    //  - bypassing verify_at with a synthetic CommandResult → this test goes
    //    through the real verifier path and the boundary assertions still fail
    //    when the map is queried with the wrong deadline

    #[test]
    fn fractional_until_survives_verify_at_and_preserves_boundary() {
        use chrono::TimeZone;

        let cv = test_command_verifier();
        let target = target_key();

        // Construct a command JWT with until = T + 0.5s expressed as a fractional
        // NumericDate float.  The test key's maximum_assertion_age is 3600s so
        // any `until` within now+3600+skew passes the ceiling check.
        let now = Utc::now();

        // Pick a whole-second base that is within the assertion-age ceiling.
        let t_whole_secs = now.timestamp() + 300; // 5 min from now
        let t_frac_secs: f64 = t_whole_secs as f64 + 0.5; // T + 500ms

        let claims = serde_json::json!({
            "iss": ISS,
            "aud": AUD,
            "sub": PRINCIPAL,
            "iat": now.timestamp(),
            "exp": now.timestamp() + 55,
            "jti": uuid::Uuid::new_v4().to_string(),
            "method": METHOD,
            "path": PATH,
            "cmd": "disconnect",
            "target_pubkey": target.to_hex(),
            "until": t_frac_secs,
        });
        // Ensure `until` is encoded as a JSON number (float), not a string.
        assert!(
            claims["until"].is_f64(),
            "until must be a JSON number for this test to exercise the fractional path"
        );

        let mut header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::ES256);
        header.kid = Some(TEST_KID.to_owned());
        header.typ = Some(COMMAND_JWT_TYP.to_owned());
        let key = jsonwebtoken::EncodingKey::from_ec_pem(TEST_EC_PKCS8_PEM.as_bytes())
            .expect("valid EC PEM");
        let token = jsonwebtoken::encode(&header, &claims, &key).expect("sign");

        // verify_at with `now` as the controlled clock.
        let result = cv.verify_at(&token, METHOD, PATH, &target, now);
        assert!(
            result.is_ok(),
            "fractional verify_at must succeed: {result:?}"
        );
        let cmd = result.unwrap();

        // The returned CommandResult.until must preserve the fractional nanos.
        let expected_until = chrono::Utc
            .timestamp_opt(t_whole_secs, 500_000_000)
            .single()
            .expect("representable timestamp");
        assert_eq!(
            cmd.until, expected_until,
            "CommandResult.until must retain the 500ms fractional nanos from the signed JWT"
        );

        // The deny entry inserted by verify_at must respect the sub-second boundary.
        let deny_map = cv.deny_map();

        // Denied immediately after T (T+1ns, well inside T+500ms).
        let now_after_t = chrono::Utc.timestamp_opt(t_whole_secs, 1).single().unwrap();
        assert!(
            deny_map.is_denied(ISS, &target, now_after_t),
            "must be denied at T+1ns (deadline is T+500ms)"
        );

        // Denied at T+499_999_999ns (just before the boundary).
        let now_before_boundary = chrono::Utc
            .timestamp_opt(t_whole_secs, 499_999_999)
            .single()
            .unwrap();
        assert!(
            deny_map.is_denied(ISS, &target, now_before_boundary),
            "must be denied at deadline - 1ns"
        );

        // Admitted at exact equality (now == until): `now < until` is false at equality.
        assert!(
            !deny_map.is_denied(ISS, &target, expected_until),
            "must be admitted at exact equality with the fractional deadline"
        );

        // Also admitted past the deadline.
        let now_past = chrono::Utc
            .timestamp_opt(t_whole_secs + 1, 0)
            .single()
            .unwrap();
        assert!(
            !deny_map.is_denied(ISS, &target, now_past),
            "must be admitted past the fractional deadline"
        );
    }
}
