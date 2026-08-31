//! Capability-authenticated metadata for the trusted Context Engine ACP seam.

use anyhow::{bail, Context, Result};
use hmac::{Hmac, KeyInit, Mac};
use rand::Rng;
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

const REQUEST_DOMAIN: &[u8] = b"buzz-ce-request-v1\0";
const RESPONSE_DOMAIN: &[u8] = b"buzz-ce-response-v1\0";
const MAX_REPLY_BYTES: usize = 64 * 1024;

type HmacSha256 = Hmac<Sha256>;

/// Host-held state binding one ACP request to its permitted response.
#[derive(Clone)]
pub(crate) struct TrustedTurn {
    key: [u8; 32],
    pub delivery_id: String,
    pub channel: String,
    pub reply_to: String,
    pub thread_root: String,
    pub owner_pubkey: String,
    nonce: String,
}

/// Verified response text returned by the trusted adapter.
pub(crate) struct VerifiedReply {
    pub text: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BuzzResponse {
    version: u8,
    delivery_id: String,
    nonce: String,
    status: String,
    text: String,
    text_hash: String,
    response_mac: String,
}

pub(crate) fn attach_request(params: &mut Value, key: [u8; 32], now: i64) -> Result<TrustedTurn> {
    let envelope = params
        .pointer("/_meta/buzz")
        .cloned()
        .context("trusted Buzz envelope is missing")?;
    let delivery_id = required_string(&envelope, "deliveryId")?;
    let channel = required_string_at(&envelope, "/channel/id")?;
    let reply_to = required_string_at(&envelope, "/reply/replyToEventId")?;
    let thread_root = required_string_at(&envelope, "/reply/rootEventId")?;
    let owner_pubkey = required_string_at(&envelope, "/channel/ownerPubkey")?;
    let envelope_hash = hex::encode(Sha256::digest(
        crate::buzz_envelope::canonical_json(&envelope)?.as_bytes(),
    ));
    let mut nonce_bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut nonce_bytes);
    let nonce = hex::encode(nonce_bytes);
    let lease_token = uuid::Uuid::new_v4().to_string();
    let unsigned = json!({
        "version": 1,
        "deliveryId": delivery_id,
        "envelopeHash": envelope_hash,
        "nonce": nonce,
        "issuedAt": now,
        "leaseToken": lease_token,
    });
    let request_mac = mac_hex(&key, REQUEST_DOMAIN, &unsigned)?;
    params["_meta"]["buzzTransport"] = json!({
        "version": 1,
        "deliveryId": delivery_id,
        "envelopeHash": envelope_hash,
        "nonce": nonce,
        "issuedAt": now,
        "leaseToken": lease_token,
        "requestMac": request_mac,
    });
    Ok(TrustedTurn {
        key,
        delivery_id,
        channel,
        reply_to,
        thread_root,
        owner_pubkey,
        nonce,
    })
}

pub(crate) fn verify_response(turn: &TrustedTurn, result: &Value) -> Result<VerifiedReply> {
    let outer = result
        .as_object()
        .context("trusted ACP response result is not an object")?;
    if outer.len() != 2
        || outer.get("stopReason").and_then(Value::as_str) != Some("end_turn")
        || !outer.contains_key("buzzResponse")
    {
        bail!("trusted ACP response result shape is invalid");
    }
    let response: BuzzResponse = serde_json::from_value(
        result
            .get("buzzResponse")
            .cloned()
            .context("trusted ACP response is missing buzzResponse")?,
    )
    .context("trusted ACP buzzResponse shape is invalid")?;
    if response.version != 1
        || response.status != "done"
        || response.delivery_id != turn.delivery_id
        || response.nonce != turn.nonce
    {
        bail!("trusted ACP response binding is invalid");
    }
    if response.text.is_empty() {
        bail!("trusted ACP response text is empty");
    }
    if response.text.len() > MAX_REPLY_BYTES {
        bail!("trusted ACP response text exceeds 64 KiB");
    }
    let text_hash = hex::encode(Sha256::digest(response.text.as_bytes()));
    if response.text_hash != text_hash {
        bail!("trusted ACP response text hash is invalid");
    }
    let unsigned = json!({
        "version": 1,
        "deliveryId": turn.delivery_id,
        "nonce": turn.nonce,
        "status": "done",
        "textHash": text_hash,
    });
    verify_mac(
        &turn.key,
        RESPONSE_DOMAIN,
        &unsigned,
        &response.response_mac,
    )?;
    Ok(VerifiedReply {
        text: response.text,
    })
}

fn mac_hex(key: &[u8; 32], domain: &[u8], value: &Value) -> Result<String> {
    let mut mac = HmacSha256::new_from_slice(key).context("invalid capability key")?;
    mac.update(domain);
    mac.update(crate::buzz_envelope::canonical_json(value)?.as_bytes());
    Ok(hex::encode(mac.finalize().into_bytes()))
}

fn verify_mac(key: &[u8; 32], domain: &[u8], value: &Value, encoded: &str) -> Result<()> {
    if encoded.len() != 64
        || !encoded
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        bail!("trusted ACP response MAC is malformed");
    }
    let bytes = hex::decode(encoded).context("trusted ACP response MAC is not hex")?;
    let mut mac = HmacSha256::new_from_slice(key).context("invalid capability key")?;
    mac.update(domain);
    mac.update(crate::buzz_envelope::canonical_json(value)?.as_bytes());
    mac.verify_slice(&bytes)
        .map_err(|_| anyhow::anyhow!("trusted ACP response MAC is invalid"))
}

fn required_string(value: &Value, field: &str) -> Result<String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .with_context(|| format!("trusted Buzz field {field} is missing"))
}

fn required_string_at(value: &Value, pointer: &str) -> Result<String> {
    value
        .pointer(pointer)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .with_context(|| format!("trusted Buzz field {pointer} is missing"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params() -> Value {
        json!({
            "_meta": {"buzz": {
                "version": 1,
                "deliveryId": "11".repeat(32),
                "channel": {"id": "631bce80-fe26-436c-98ee-2c28e9360ec4", "ownerPubkey": "22".repeat(32)},
                "reply": {"replyToEventId": "33".repeat(32), "rootEventId": "44".repeat(32)},
                "events": []
            }}
        })
    }

    fn signed_response(turn: &TrustedTurn, text: &str) -> Value {
        let text_hash = hex::encode(Sha256::digest(text.as_bytes()));
        let unsigned = json!({
            "version": 1,
            "deliveryId": turn.delivery_id,
            "nonce": turn.nonce,
            "status": "done",
            "textHash": text_hash,
        });
        let response_mac = mac_hex(&turn.key, RESPONSE_DOMAIN, &unsigned).unwrap();
        json!({"stopReason": "end_turn", "buzzResponse": {
            "version": 1,
            "deliveryId": turn.delivery_id,
            "nonce": turn.nonce,
            "status": "done",
            "text": text,
            "textHash": text_hash,
            "responseMac": response_mac,
        }})
    }

    #[test]
    fn request_and_response_are_bound_by_capability_mac() {
        let mut params = params();
        let turn = attach_request(&mut params, [7; 32], 1_700_000_000).unwrap();
        let response = signed_response(&turn, "hello");
        assert_eq!(verify_response(&turn, &response).unwrap().text, "hello");
        assert_eq!(
            params["_meta"]["buzzTransport"]["nonce"]
                .as_str()
                .unwrap()
                .len(),
            64
        );
    }

    #[test]
    fn hmac_contract_matches_cross_language_vectors() {
        let key: [u8; 32] = std::array::from_fn(|index| index as u8);
        let request = json!({
            "version": 1,
            "deliveryId": "11".repeat(32),
            "envelopeHash": "22".repeat(32),
            "nonce": "33".repeat(32),
            "issuedAt": 1_700_000_000,
            "leaseToken": "631bce80-fe26-436c-98ee-2c28e9360ec4",
        });
        assert_eq!(
            mac_hex(&key, REQUEST_DOMAIN, &request).unwrap(),
            "c7ff624511b8495ba389bfc60272d5b9a6c53cdec382ee3761cd94450eda5688"
        );
        let response = json!({
            "version": 1,
            "deliveryId": "11".repeat(32),
            "nonce": "33".repeat(32),
            "status": "done",
            "textHash": "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824",
        });
        assert_eq!(
            mac_hex(&key, RESPONSE_DOMAIN, &response).unwrap(),
            "813a4c3f9b7c7774c2d4f3ff666d40bb854e91f6f42d5bcca307159d82a4dea2"
        );
    }

    #[test]
    fn stacy_hmac_contract_matches_fixed_golden_vector() {
        let key: [u8; 32] = std::array::from_fn(|index| index as u8);
        let request = json!({
            "version": 1,
            "deliveryId": "11".repeat(32),
            "envelopeHash": "22".repeat(32),
            "nonce": "33".repeat(32),
            "issuedAt": 1_788_000_000,
            "leaseToken": "b379b2cc-7e42-4d0b-bc10-dc103e61c5c8",
        });
        assert_eq!(
            mac_hex(&key, REQUEST_DOMAIN, &request).unwrap(),
            "2b0e09cd130a19bf4dd42885a95f1fa90515b56fb769b49c8dd2eee8f2553ffc"
        );

        let text_hash = hex::encode(Sha256::digest("Stacy ✅".as_bytes()));
        assert_eq!(
            text_hash,
            "f955ce6ec1c1ee1c4dfc7c17bd7a80e8472409d612a24b187026464094e3cae5"
        );
        let response = json!({
            "version": 1,
            "deliveryId": "11".repeat(32),
            "nonce": "33".repeat(32),
            "status": "done",
            "textHash": text_hash,
        });
        assert_eq!(
            mac_hex(&key, RESPONSE_DOMAIN, &response).unwrap(),
            "54fbd527e212e83640065cb42705e8e3b963eaf51d451039d546d481c495f1ee"
        );
    }

    #[test]
    fn tampering_and_unknown_response_fields_fail_closed() {
        let mut params = params();
        let turn = attach_request(&mut params, [9; 32], 1_700_000_000).unwrap();
        let mut response = signed_response(&turn, "hello");
        response["buzzResponse"]["text"] = json!("tampered");
        assert!(verify_response(&turn, &response).is_err());
        let mut response = signed_response(&turn, "hello");
        response["buzzResponse"]["unexpected"] = json!(true);
        assert!(verify_response(&turn, &response).is_err());
        let mut response = signed_response(&turn, "hello");
        response["unexpected"] = json!(true);
        assert!(verify_response(&turn, &response).is_err());
    }

    #[test]
    fn empty_response_fails_closed() {
        let mut params = params();
        let turn = attach_request(&mut params, [9; 32], 1_700_000_000).unwrap();
        assert!(verify_response(&turn, &signed_response(&turn, "")).is_err());
    }
}
