//! Web Push (RFC 8030/8291/8292) last hop for the `buzz-web` profile.
//!
//! Same posture as APNs: the gateway never sees message content. The relay may
//! attach a closed, content-free [`WebPushHint`] (wake class, event id, channel
//! id) so the browser can deep-link; the payload is built here from that hint
//! and a compiled-in copy table, then encrypted end-to-end to the browser.

use std::time::Duration;

use aes_gcm::{aead::Aead, Aes128Gcm, KeyInit, Nonce};
use async_trait::async_trait;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD as B64URL, Engine as _};
use hkdf::Hkdf;
use p256::{
    ecdh::EphemeralSecret,
    ecdsa::{signature::Signer, Signature, SigningKey},
    elliptic_curve::sec1::ToEncodedPoint,
    PublicKey,
};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

use crate::apns::DeliveryOutcome;
use crate::model::WebPushHint;

/// Largest serialized subscription the gateway will custody.
pub const MAX_SUBSCRIPTION_JSON_BYTES: usize = 1024;
const MAX_ENDPOINT_URL_BYTES: usize = 800;
const RECORD_SIZE: u32 = 4096;

/// Browser push services the gateway will POST to. Anything else is rejected
/// at registration, so a subscription can never aim the gateway at an
/// internal or attacker-chosen host (SSRF).
const ALLOWED_PUSH_HOST_SUFFIXES: &[&str] = &[
    ".push.apple.com",
    ".googleapis.com",
    ".push.services.mozilla.com",
    ".notify.windows.com",
];
const ALLOWED_PUSH_HOSTS: &[&str] = &["fcm.googleapis.com"];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WebPushKeys {
    pub p256dh: String,
    pub auth: String,
}

/// Browser `PushSubscription.toJSON()` minus `expirationTime`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WebPushSubscription {
    pub endpoint: String,
    pub keys: WebPushKeys,
}

/// Validate endpoint host allowlist and key encodings.
pub fn validate_subscription(sub: &WebPushSubscription) -> Result<(), &'static str> {
    if sub.endpoint.len() > MAX_ENDPOINT_URL_BYTES {
        return Err("endpoint too long");
    }
    let url = url::Url::parse(&sub.endpoint).map_err(|_| "bad endpoint")?;
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some_and(|p| p != 443)
    {
        return Err("bad endpoint");
    }
    let host = url.host_str().ok_or("bad endpoint")?.to_ascii_lowercase();
    if !ALLOWED_PUSH_HOSTS.contains(&host.as_str())
        && !ALLOWED_PUSH_HOST_SUFFIXES.iter().any(|s| host.ends_with(s))
    {
        return Err("unsupported push service");
    }
    let p = B64URL
        .decode(sub.keys.p256dh.trim_end_matches('='))
        .map_err(|_| "bad p256dh")?;
    PublicKey::from_sec1_bytes(&p).map_err(|_| "bad p256dh")?;
    if p.len() != 65 {
        return Err("bad p256dh");
    }
    let a = B64URL
        .decode(sub.keys.auth.trim_end_matches('='))
        .map_err(|_| "bad auth")?;
    if a.len() != 16 {
        return Err("bad auth");
    }
    Ok(())
}

/// Canonical bytes stored (encrypted) as the installation "token".
pub fn subscription_bytes(sub: &WebPushSubscription) -> Vec<u8> {
    serde_json::to_vec(sub).expect("subscription serializes")
}

/// Content-free, deep-linkable notification payload.
pub fn build_payload(hint: Option<&WebPushHint>) -> Vec<u8> {
    let class = hint.map(|h| h.class.as_str()).unwrap_or("default");
    let body = match class {
        "dm" => "New direct message",
        "mention" => "You were mentioned",
        "thread_reply" => "New reply in a thread",
        "needs_action" => "Something needs your approval",
        _ => "New activity",
    };
    let target = hint.map(|h| {
        serde_json::json!({
            "channelId": h.channel_id,
            "threadRootId": h.thread_root_id,
            "eventId": h.event_id,
            "kind": null,
        })
    });
    serde_json::to_vec(&serde_json::json!({
        "title": "Hula Buzz",
        "body": body,
        "tag": hint.map(|h| format!("buzz-{}", h.event_id)),
        "target": target,
        "prefetch": hint.and_then(|h| h.prefetch.as_ref()),
    }))
    .expect("payload serializes")
}

/// RFC 8291 aes128gcm encryption of `plaintext` to the browser keys.
pub fn encrypt(sub: &WebPushSubscription, plaintext: &[u8]) -> Result<Vec<u8>, &'static str> {
    let ua_bytes = B64URL
        .decode(sub.keys.p256dh.trim_end_matches('='))
        .map_err(|_| "bad p256dh")?;
    let ua_public = PublicKey::from_sec1_bytes(&ua_bytes).map_err(|_| "bad p256dh")?;
    let auth = B64URL
        .decode(sub.keys.auth.trim_end_matches('='))
        .map_err(|_| "bad auth")?;
    let as_secret = EphemeralSecret::random(&mut p256::elliptic_curve::rand_core::OsRng);
    let as_public = as_secret.public_key().to_encoded_point(false);
    let shared = as_secret.diffie_hellman(&ua_public);
    let mut salt = [0u8; 16];
    getrandom::fill(&mut salt).map_err(|_| "rng")?;
    encrypt_with(
        &ua_bytes,
        &auth,
        as_public.as_bytes(),
        shared.raw_secret_bytes(),
        &salt,
        plaintext,
    )
}

fn encrypt_with(
    ua_public: &[u8],
    auth: &[u8],
    as_public: &[u8],
    ecdh_secret: &[u8],
    salt: &[u8; 16],
    plaintext: &[u8],
) -> Result<Vec<u8>, &'static str> {
    let mut key_info = b"WebPush: info\0".to_vec();
    key_info.extend_from_slice(ua_public);
    key_info.extend_from_slice(as_public);
    let mut ikm = [0u8; 32];
    Hkdf::<Sha256>::new(Some(auth), ecdh_secret)
        .expand(&key_info, &mut ikm)
        .map_err(|_| "hkdf")?;
    let hk = Hkdf::<Sha256>::new(Some(salt), &ikm);
    let mut cek = [0u8; 16];
    let mut nonce = [0u8; 12];
    hk.expand(b"Content-Encoding: aes128gcm\0", &mut cek)
        .map_err(|_| "hkdf")?;
    hk.expand(b"Content-Encoding: nonce\0", &mut nonce)
        .map_err(|_| "hkdf")?;
    let mut record = plaintext.to_vec();
    record.push(0x02); // last-record delimiter
    if record.len() + 16 > RECORD_SIZE as usize {
        return Err("payload too large");
    }
    let ct = Aes128Gcm::new_from_slice(&cek)
        .map_err(|_| "aes")?
        .encrypt(Nonce::from_slice(&nonce), record.as_slice())
        .map_err(|_| "aes")?;
    let mut out = Vec::with_capacity(86 + ct.len());
    out.extend_from_slice(salt);
    out.extend_from_slice(&RECORD_SIZE.to_be_bytes());
    out.push(as_public.len() as u8);
    out.extend_from_slice(as_public);
    out.extend_from_slice(&ct);
    Ok(out)
}

/// Server VAPID identity (RFC 8292). The private key is loaded from a mounted
/// secret file and never leaves this struct.
pub struct Vapid {
    key: SigningKey,
    public_b64: String,
    subject: String,
}

impl Vapid {
    /// Load from a PKCS#8/SEC1 PEM or a raw base64url 32-byte scalar.
    pub fn from_secret(secret: &str, subject: String) -> Result<Self, &'static str> {
        if !(subject.starts_with("mailto:") || subject.starts_with("https://")) {
            return Err("VAPID subject must be mailto: or https:");
        }
        let trimmed = secret.trim();
        let key = if trimmed.starts_with("-----BEGIN") {
            use p256::pkcs8::DecodePrivateKey;
            SigningKey::from_pkcs8_pem(trimmed)
                .or_else(|_| p256::SecretKey::from_sec1_pem(trimmed).map(SigningKey::from))
                .map_err(|_| "bad VAPID key")?
        } else {
            let raw = B64URL
                .decode(trimmed.trim_end_matches('='))
                .map_err(|_| "bad VAPID key")?;
            SigningKey::from_slice(&raw).map_err(|_| "bad VAPID key")?
        };
        let public_b64 = B64URL.encode(key.verifying_key().to_encoded_point(false).as_bytes());
        Ok(Self {
            key,
            public_b64,
            subject,
        })
    }
    pub fn public_key_b64url(&self) -> &str {
        &self.public_b64
    }
    /// `Authorization: vapid t=<jwt>, k=<pub>` for one push-service origin.
    pub fn authorization(&self, endpoint: &str, now: i64) -> Result<String, &'static str> {
        let url = url::Url::parse(endpoint).map_err(|_| "bad endpoint")?;
        let aud = url.origin().ascii_serialization();
        let header = B64URL.encode(br#"{"typ":"JWT","alg":"ES256"}"#);
        let claims = B64URL.encode(
            serde_json::to_vec(
                &serde_json::json!({"aud": aud, "exp": now + 12 * 3600, "sub": self.subject}),
            )
            .map_err(|_| "jwt")?,
        );
        let input = format!("{header}.{claims}");
        let sig: Signature = self.key.sign(input.as_bytes());
        Ok(format!(
            "vapid t={input}.{}, k={}",
            B64URL.encode(sig.to_bytes()),
            self.public_b64
        ))
    }
}

/// Classify a push-service response without leaking its body.
pub fn classify(code: u16) -> DeliveryOutcome {
    match code {
        200..=202 => DeliveryOutcome::Accepted,
        404 | 410 => DeliveryOutcome::InvalidEndpoint {
            unregistered_at: None,
        },
        401 | 403 => DeliveryOutcome::ConfigurationFault,
        429 | 500..=599 => DeliveryOutcome::Retry {
            retry_after_seconds: None,
        },
        _ => DeliveryOutcome::PermanentRequestFault,
    }
}

#[async_trait]
pub trait WebPushTransport: Send + Sync {
    async fn send(
        &self,
        sub: &WebPushSubscription,
        payload: &[u8],
        ttl_seconds: i64,
        urgency: &'static str,
    ) -> DeliveryOutcome;
}

pub struct HttpWebPushTransport {
    pub vapid: Vapid,
    pub http: reqwest::Client,
    pub now: fn() -> i64,
}

impl HttpWebPushTransport {
    pub fn new(vapid: Vapid, now: fn() -> i64) -> Result<Self, reqwest::Error> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        Ok(Self { vapid, http, now })
    }
}

#[async_trait]
impl WebPushTransport for HttpWebPushTransport {
    async fn send(
        &self,
        sub: &WebPushSubscription,
        payload: &[u8],
        ttl_seconds: i64,
        urgency: &'static str,
    ) -> DeliveryOutcome {
        if validate_subscription(sub).is_err() {
            return DeliveryOutcome::InvalidEndpoint {
                unregistered_at: None,
            };
        }
        let Ok(body) = encrypt(sub, payload) else {
            return DeliveryOutcome::PermanentRequestFault;
        };
        let Ok(auth) = self.vapid.authorization(&sub.endpoint, (self.now)()) else {
            return DeliveryOutcome::ConfigurationFault;
        };
        match self
            .http
            .post(&sub.endpoint)
            .header("TTL", ttl_seconds.clamp(0, 86_400).to_string())
            .header("Content-Encoding", "aes128gcm")
            .header("Content-Type", "application/octet-stream")
            .header("Urgency", urgency)
            .header("Authorization", auth)
            .body(body)
            .send()
            .await
        {
            Ok(r) => classify(r.status().as_u16()),
            Err(_) => DeliveryOutcome::Retry {
                retry_after_seconds: None,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sub(endpoint: &str) -> WebPushSubscription {
        // RFC 8291 §5 example receiver keys.
        WebPushSubscription {
            endpoint: endpoint.into(),
            keys: WebPushKeys {
                p256dh: "BCVxsr7N_eNgVRqvHtD0zTZsEc6-VV-JvLexhqUzORcxaOzi6-AYWXvTBHm4bjyPjs7Vd8pZGH6SRpkNtoIAiw4".into(),
                auth: "BTBZMqHH6r4Tts7J_aSIgg".into(),
            },
        }
    }

    #[test]
    fn rfc8291_vector() {
        let ua = B64URL.decode("BCVxsr7N_eNgVRqvHtD0zTZsEc6-VV-JvLexhqUzORcxaOzi6-AYWXvTBHm4bjyPjs7Vd8pZGH6SRpkNtoIAiw4").unwrap();
        let auth = B64URL.decode("BTBZMqHH6r4Tts7J_aSIgg").unwrap();
        let as_priv = B64URL
            .decode("yfWPiYE-n46HLnH0KqZOF1fJJU3MYrct3AELtAQ-oRw")
            .unwrap();
        let as_secret = p256::SecretKey::from_slice(&as_priv).unwrap();
        let as_pub = as_secret.public_key().to_encoded_point(false);
        let shared = p256::ecdh::diffie_hellman(
            as_secret.to_nonzero_scalar(),
            PublicKey::from_sec1_bytes(&ua).unwrap().as_affine(),
        );
        let salt: [u8; 16] = B64URL
            .decode("DGv6ra1nlYgDCS1FRnbzlw")
            .unwrap()
            .try_into()
            .unwrap();
        let out = encrypt_with(
            &ua,
            &auth,
            as_pub.as_bytes(),
            shared.raw_secret_bytes(),
            &salt,
            b"When I grow up, I want to be a watermelon",
        )
        .unwrap();
        assert_eq!(
            B64URL.encode(out),
            "DGv6ra1nlYgDCS1FRnbzlwAAEABBBP4z9KsN6nGRTbVYI_c7VJSPQTBtkgcy27mlmlMoZIIgDll6e3vCYLocInmYWAmS6TlzAC8wEqKK6PBru3jl7A_yl95bQpu6cVPTpK4Mqgkf1CXztLVBSt2Ks3oZwbuwXPXLWyouBWLVWGNWQexSgSxsj_Qulcy4a-fN"
        );
    }

    #[test]
    fn allowlist_blocks_internal_hosts() {
        assert!(validate_subscription(&sub("https://web.push.apple.com/abc")).is_ok());
        assert!(validate_subscription(&sub("https://fcm.googleapis.com/fcm/send/x")).is_ok());
        assert!(validate_subscription(&sub(
            "https://updates.push.services.mozilla.com/wpush/v2/x"
        ))
        .is_ok());
        assert!(validate_subscription(&sub("https://169.254.169.254/latest")).is_err());
        assert!(validate_subscription(&sub("http://web.push.apple.com/abc")).is_err());
        assert!(validate_subscription(&sub("https://evilpush.apple.com.attacker.io/x")).is_err());
        assert!(validate_subscription(&sub("https://web.push.apple.com:8443/x")).is_err());
    }

    #[test]
    fn payload_is_content_free() {
        let hint = WebPushHint {
            class: "dm".into(),
            event_id: "ab".repeat(32),
            channel_id: Some("123e4567-e89b-42d3-a456-426614174000".into()),
            thread_root_id: Some("b".repeat(64)),
            prefetch: None,
        };
        let p: serde_json::Value = serde_json::from_slice(&build_payload(Some(&hint))).unwrap();
        assert_eq!(p["body"], "New direct message");
        assert_eq!(p["target"]["eventId"], "ab".repeat(32));
        assert_eq!(p["target"]["threadRootId"], "b".repeat(64));
        let mut bad = hint.clone();
        bad.thread_root_id = Some("XYZ".into());
        assert!(!bad.is_valid());
        assert!(hint.is_valid());
        assert_eq!(p.as_object().unwrap().len(), 5);
        assert!(p["prefetch"].is_null());
    }

    #[test]
    fn prefetch_hint_passthrough_and_validation() {
        let pf = crate::model::PrefetchHint {
            url: "https://relay.example.com/push/prefetch".into(),
            token: "eyJ2IjoxfQ.c2ln-_".into(),
        };
        let hint = WebPushHint {
            class: "mention".into(),
            event_id: "ab".repeat(32),
            channel_id: None,
            thread_root_id: None,
            prefetch: Some(pf.clone()),
        };
        assert!(hint.is_valid());
        let p: serde_json::Value = serde_json::from_slice(&build_payload(Some(&hint))).unwrap();
        assert_eq!(p["prefetch"]["url"], pf.url);
        assert_eq!(p["prefetch"]["token"], pf.token);
        for bad in [
            crate::model::PrefetchHint {
                url: "http://x/y".into(),
                ..pf.clone()
            },
            crate::model::PrefetchHint {
                token: "a b".into(),
                ..pf.clone()
            },
            crate::model::PrefetchHint {
                token: "a".repeat(513),
                ..pf.clone()
            },
            crate::model::PrefetchHint {
                url: format!("https://{}", "a".repeat(250)),
                ..pf.clone()
            },
        ] {
            let mut h = hint.clone();
            h.prefetch = Some(bad);
            assert!(!h.is_valid());
        }
        let json = r#"{"class":"dm","event_id":"x","channel_id":null,"prefetch":{"url":"https://a","token":"t","extra":1}}"#;
        assert!(serde_json::from_str::<WebPushHint>(json).is_err());
    }

    #[test]
    fn vapid_jwt_verifies() {
        let vapid = Vapid::from_secret(
            "yfWPiYE-n46HLnH0KqZOF1fJJU3MYrct3AELtAQ-oRw",
            "mailto:a@b.c".into(),
        )
        .unwrap();
        let h = vapid
            .authorization("https://web.push.apple.com/x", 1_000)
            .unwrap();
        let t = h
            .strip_prefix("vapid t=")
            .unwrap()
            .split(", k=")
            .next()
            .unwrap();
        let (input, sig) = t.rsplit_once('.').unwrap();
        use p256::ecdsa::signature::Verifier;
        let sig = Signature::from_slice(&B64URL.decode(sig).unwrap()).unwrap();
        assert!(vapid
            .key
            .verifying_key()
            .verify(input.as_bytes(), &sig)
            .is_ok());
        assert!(input.contains('.'));
    }

    #[test]
    fn classify_maps_dead_subscriptions() {
        assert_eq!(classify(201), DeliveryOutcome::Accepted);
        assert!(matches!(
            classify(410),
            DeliveryOutcome::InvalidEndpoint { .. }
        ));
        assert!(matches!(
            classify(404),
            DeliveryOutcome::InvalidEndpoint { .. }
        ));
        assert_eq!(classify(403), DeliveryOutcome::ConfigurationFault);
    }
}
