//! Approver specifications for `request_approval` gates (WF-08).
//!
//! This module is the single definition of who may decide an approval gate.
//! Definition-time validation, gate creation, and every decision path (the
//! HTTP decide endpoint and the Nostr grant/deny command kinds) go through it,
//! so they cannot drift apart.
//!
//! # Security invariants
//!
//! - **Fail closed.** A spec that is empty, names a role, or cannot be parsed
//!   never authorizes anyone. There is no role resolver yet, so `@manager` and
//!   friends are rejected rather than treated as "anyone".
//! - **Exact identity.** A pubkey spec authorizes only the identity the caller
//!   has already proven (NIP-98 / NIP-42); the comparison is on decoded keys,
//!   never on caller-supplied names.
//! - **Templates pin to a pubkey.** `from` is template-resolved at run time
//!   from data that may be attacker-influenced (`{{trigger.text}}`, webhook
//!   fields). A templated spec must resolve to a pubkey; it can never widen to
//!   `any` or to an unresolved string.
//! - **No implicit privilege.** Nothing here special-cases the workflow
//!   initiator, owner, or channel admins. They decide only if the spec names
//!   them (or the author explicitly chose `any`).

use nostr::PublicKey;
use thiserror::Error;

/// The explicit opt-in spec: any identity the decision path has admitted.
pub const ANY_APPROVER: &str = "any";

/// Hint appended to every spec error so authors see the supported forms.
const SUPPORTED_FORMS: &str =
    "use a 64-character hex pubkey, 'any', or a template that resolves to a pubkey";

/// A parsed, supported approver specification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApproverSpec {
    /// Exactly this pubkey may grant or deny the gate.
    Pubkey(PublicKey),
    /// The author explicitly opted in to any identity that passes the decision
    /// path's own checks (authenticated, relay member, can read the workflow's
    /// channel). Written as the literal `any`.
    Anyone,
}

/// Why an approver spec is not usable.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ApproverSpecError {
    /// The spec is empty or whitespace.
    #[error("approver is empty; {SUPPORTED_FORMS}")]
    Empty,
    /// A named role such as `@manager`. No trusted role resolver exists.
    #[error("named approver roles such as '{0}' are not supported; {SUPPORTED_FORMS}")]
    UnsupportedRole(String),
    /// 64 hex characters that do not decode to a valid pubkey, or a hex string
    /// of the wrong length.
    #[error("approver '{0}' is not a valid pubkey; {SUPPORTED_FORMS}")]
    MalformedPubkey(String),
    /// Anything else (npub, display name, free text).
    #[error("unrecognized approver '{0}'; {SUPPORTED_FORMS}")]
    Unrecognized(String),
    /// A templated spec resolved to something other than a pubkey. The
    /// resolved value is deliberately not echoed: it may be untrusted input.
    #[error("approver template did not resolve to a 64-character hex pubkey")]
    TemplateNotPubkey,
}

/// Why a decision by a given identity is refused.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum DecisionDenied {
    /// The spec is valid but names a different identity.
    #[error("not the designated approver for this gate")]
    NotDesignatedApprover,
    /// The stored spec is not supported (legacy row, role name, empty, ...).
    /// Fails closed: nobody may decide.
    #[error("approver spec for this gate is not supported: {0}")]
    UnsupportedSpec(ApproverSpecError),
}

impl ApproverSpec {
    /// Parse a concrete (already template-resolved) spec.
    pub fn parse(raw: &str) -> Result<Self, ApproverSpecError> {
        let spec = raw.trim();
        if spec.is_empty() {
            return Err(ApproverSpecError::Empty);
        }
        if spec == ANY_APPROVER {
            return Ok(Self::Anyone);
        }
        if spec.starts_with('@') {
            return Err(ApproverSpecError::UnsupportedRole(spec.to_owned()));
        }
        if spec.chars().all(|c| c.is_ascii_hexdigit()) {
            if spec.len() != 64 {
                return Err(ApproverSpecError::MalformedPubkey(spec.to_owned()));
            }
            // `from_hex` only decodes bytes; `xonly()` proves the bytes are a
            // point on the curve, so a spec that no key could ever sign for is
            // refused up front instead of silently becoming undecidable.
            return PublicKey::from_hex(spec)
                .ok()
                .filter(|key| key.xonly().is_ok())
                .map(Self::Pubkey)
                .ok_or_else(|| ApproverSpecError::MalformedPubkey(spec.to_owned()));
        }
        Err(ApproverSpecError::Unrecognized(spec.to_owned()))
    }

    /// Canonical stored form: lowercase hex for a pubkey, `any` otherwise.
    pub fn canonical(&self) -> String {
        match self {
            Self::Pubkey(key) => key.to_hex(),
            Self::Anyone => ANY_APPROVER.to_owned(),
        }
    }

    /// Whether `requester` (an already-authenticated identity) satisfies
    /// this spec.
    pub fn authorizes(&self, requester: &PublicKey) -> bool {
        match self {
            Self::Pubkey(key) => key == requester,
            Self::Anyone => true,
        }
    }
}

/// Whether the definition-time `from` value contains a template placeholder.
fn is_templated(raw: &str) -> bool {
    raw.contains("{{")
}

/// Definition-time check for a `request_approval` step's `from`.
///
/// Concrete specs must parse. Templated specs are accepted here and
/// re-checked by [`resolve_gate_approver`] once resolved, where they must
/// yield a pubkey.
pub fn validate_definition_spec(raw: &str) -> Result<(), ApproverSpecError> {
    if is_templated(raw) {
        return Ok(());
    }
    ApproverSpec::parse(raw).map(|_| ())
}

/// Gate-creation check: turn the definition's `from` and its template-resolved
/// value into the canonical spec that is persisted on the gate.
///
/// A templated `from` must resolve to a pubkey: untrusted template input can
/// never produce `any`, an empty spec, or a role name.
pub fn resolve_gate_approver(raw: &str, resolved: &str) -> Result<String, ApproverSpecError> {
    if is_templated(raw) {
        return match ApproverSpec::parse(resolved) {
            Ok(spec @ ApproverSpec::Pubkey(_)) => Ok(spec.canonical()),
            _ => Err(ApproverSpecError::TemplateNotPubkey),
        };
    }
    ApproverSpec::parse(resolved).map(|spec| spec.canonical())
}

/// Decision-time check shared by every path that can grant or deny a gate.
///
/// `stored_spec` is the value persisted on the gate (never caller input) and
/// `requester` is the identity the transport already proved. Callers must run
/// this — and their channel-access check — before writing any state.
pub fn authorize_decision(stored_spec: &str, requester: &PublicKey) -> Result<(), DecisionDenied> {
    let spec = ApproverSpec::parse(stored_spec).map_err(DecisionDenied::UnsupportedSpec)?;
    if spec.authorizes(requester) {
        Ok(())
    } else {
        Err(DecisionDenied::NotDesignatedApprover)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::Keys;

    fn key() -> PublicKey {
        Keys::generate().public_key()
    }

    #[test]
    fn exact_pubkey_authorizes_only_that_identity() {
        let approver = key();
        let other = key();
        let spec = approver.to_hex();
        assert_eq!(authorize_decision(&spec, &approver), Ok(()));
        assert_eq!(
            authorize_decision(&spec, &other),
            Err(DecisionDenied::NotDesignatedApprover)
        );
        // Case and surrounding whitespace do not change the identity.
        assert_eq!(authorize_decision(&spec.to_uppercase(), &approver), Ok(()));
        assert_eq!(
            authorize_decision(&format!("  {spec}\n"), &approver),
            Ok(())
        );
    }

    /// The P1 from review: an unresolved role must never authorize anyone,
    /// including the identity whose role name it is.
    #[test]
    fn role_specs_fail_closed_for_everyone() {
        for role in ["@manager", "@release-manager", "@admin", "@owner", "@"] {
            for requester in [key(), key()] {
                assert!(
                    matches!(
                        authorize_decision(role, &requester),
                        Err(DecisionDenied::UnsupportedSpec(
                            ApproverSpecError::UnsupportedRole(_)
                        ))
                    ),
                    "{role} must fail closed"
                );
            }
        }
    }

    /// Full table over the unsupported input space: none of these may
    /// authorize anyone.
    #[test]
    fn unsupported_specs_never_authorize() {
        let requester = key();
        let hex = requester.to_hex();
        type Expect = fn(&ApproverSpecError) -> bool;
        let cases: Vec<(String, Expect)> = vec![
            (String::new(), |e| matches!(e, ApproverSpecError::Empty)),
            ("   ".into(), |e| matches!(e, ApproverSpecError::Empty)),
            (format!("@{hex}"), |e| {
                matches!(e, ApproverSpecError::UnsupportedRole(_))
            }),
            (hex[..63].to_owned(), |e| {
                matches!(e, ApproverSpecError::MalformedPubkey(_))
            }),
            (format!("{hex}0"), |e| {
                matches!(e, ApproverSpecError::MalformedPubkey(_))
            }),
            // 64 hex chars that are not a valid x-only key (p > field size).
            ("f".repeat(64), |e| {
                matches!(e, ApproverSpecError::MalformedPubkey(_))
            }),
            ("Any".into(), |e| {
                matches!(e, ApproverSpecError::Unrecognized(_))
            }),
            ("anyone".into(), |e| {
                matches!(e, ApproverSpecError::Unrecognized(_))
            }),
            ("manager".into(), |e| {
                matches!(e, ApproverSpecError::Unrecognized(_))
            }),
            (format!("npub1{}", "q".repeat(58)), |e| {
                matches!(e, ApproverSpecError::Unrecognized(_))
            }),
            ("{{trigger.author}}".into(), |e| {
                matches!(e, ApproverSpecError::Unrecognized(_))
            }),
            (format!("{hex},{hex}"), |e| {
                matches!(e, ApproverSpecError::Unrecognized(_))
            }),
        ];
        for (spec, expected) in cases {
            match authorize_decision(&spec, &requester) {
                Err(DecisionDenied::UnsupportedSpec(error)) => {
                    assert!(expected(&error), "{spec:?} gave {error:?}");
                    assert!(error.to_string().contains("64-character hex pubkey"));
                }
                other => panic!("{spec:?} must fail closed, got {other:?}"),
            }
        }
    }

    #[test]
    fn any_is_an_explicit_opt_in() {
        assert_eq!(ApproverSpec::parse("any"), Ok(ApproverSpec::Anyone));
        assert_eq!(ApproverSpec::parse(" any "), Ok(ApproverSpec::Anyone));
        assert_eq!(authorize_decision("any", &key()), Ok(()));
    }

    #[test]
    fn definition_validation_matches_parse_and_allows_templates() {
        let hex = key().to_hex();
        assert_eq!(validate_definition_spec(&hex), Ok(()));
        assert_eq!(validate_definition_spec("any"), Ok(()));
        assert_eq!(validate_definition_spec("{{trigger.author}}"), Ok(()));
        assert!(matches!(
            validate_definition_spec("@manager"),
            Err(ApproverSpecError::UnsupportedRole(_))
        ));
        assert_eq!(validate_definition_spec(""), Err(ApproverSpecError::Empty));
    }

    #[test]
    fn templated_approver_must_resolve_to_a_pubkey() {
        let approver = key();
        let hex = approver.to_hex();
        assert_eq!(
            resolve_gate_approver("{{trigger.author}}", &hex.to_uppercase()),
            Ok(hex.clone())
        );
        // Untrusted template input can never widen the gate.
        for resolved in ["any", "", "@manager", "{{trigger.author}}", "nonsense"] {
            assert_eq!(
                resolve_gate_approver("{{trigger.text}}", resolved),
                Err(ApproverSpecError::TemplateNotPubkey),
                "{resolved:?}"
            );
        }
        // Concrete specs are canonicalized.
        assert_eq!(
            resolve_gate_approver(" any ", " any "),
            Ok("any".to_owned())
        );
        assert_eq!(
            resolve_gate_approver(&hex.to_uppercase(), &hex.to_uppercase()),
            Ok(hex)
        );
        assert!(resolve_gate_approver("@manager", "@manager").is_err());
    }
}
