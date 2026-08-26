//! The strict wire form of a [`BrokerResponse`].
//!
//! Split from [`super`] to keep that file within the repo's 1,000-line ceiling.
//! This is the response side's only reader, so it is the one place the envelope's
//! strictness is defined.

use serde::Deserialize;

use super::{absent_or_valued, ActionOutcome, BrokerError, BrokerResponse, BrokerResult};

/// The strict wire form of a [`BrokerResponse`]: every key spelled out, no
/// `flatten`, so `deny_unknown_fields` is actually in force.
///
/// The status-specific members are `Option` only because one struct describes
/// three shapes; the status match below requires the exact set per status.
/// They deserialize through [`absent_or_valued`] because the match reads
/// `None` as *absent*, and plain `#[serde(default)]` would map an explicit
/// `null` to the same `None` — letting a contradictory response like
/// `{"status":"failed","outcome":null}` skip the check.
///
/// `outcome` is held as a `RawValue` and re-parsed, so this reader is
/// JSON-specific — which is fine, JSON is the only encoding this contract has
/// ever specified.
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct WireResponse {
    r#type: String,
    protocol_version: u16,
    request_id: String,
    status: String,
    #[serde(default, deserialize_with = "absent_or_valued")]
    action: Option<String>,
    #[serde(default, deserialize_with = "absent_or_valued")]
    outcome: Option<Box<serde_json::value::RawValue>>,
    #[serde(default, deserialize_with = "absent_or_valued")]
    error: Option<BrokerError>,
    #[serde(default)]
    replayed: bool,
}

impl<'de> Deserialize<'de> for BrokerResponse {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use serde::de::Error as _;

        let wire = WireResponse::deserialize(deserializer)?;

        // One arm per status, each naming the members that status may carry.
        // Anything present that this status does not admit is rejected here, so
        // "succeeded with an error" cannot be parsed and then ignored.
        let result = match wire.status.as_str() {
            "succeeded" => {
                if wire.error.is_some() {
                    return Err(D::Error::custom(
                        "a succeeded response must not carry an error",
                    ));
                }
                let action = wire
                    .action
                    .ok_or_else(|| D::Error::missing_field("action"))?;
                let outcome = wire
                    .outcome
                    .ok_or_else(|| D::Error::missing_field("outcome"))?;
                // Re-parse the outcome under its action tag from the original
                // bytes — not via `serde_json::Value`, which collapses
                // duplicate keys last-wins — so each outcome type's
                // `deny_unknown_fields` applies to the bytes as sent.
                let tagged = format!(
                    "{{\"action\":{},\"outcome\":{}}}",
                    serde_json::to_string(&action).map_err(D::Error::custom)?,
                    outcome.get()
                );
                let outcome: ActionOutcome =
                    serde_json::from_str(&tagged).map_err(D::Error::custom)?;
                BrokerResult::Succeeded { outcome }
            }
            status @ ("failed" | "indeterminate") => {
                if wire.action.is_some() || wire.outcome.is_some() {
                    return Err(D::Error::custom(format!(
                        "a {status} response must not carry an action or outcome"
                    )));
                }
                let error = wire.error.ok_or_else(|| D::Error::missing_field("error"))?;
                if status == "failed" {
                    BrokerResult::Failed { error }
                } else {
                    BrokerResult::Indeterminate { error }
                }
            }
            other => {
                return Err(D::Error::custom(format!(
                    "unknown broker result status \"{other}\""
                )))
            }
        };

        Ok(Self {
            r#type: wire.r#type,
            protocol_version: wire.protocol_version,
            request_id: wire.request_id,
            result,
            replayed: wire.replayed,
        })
    }
}
