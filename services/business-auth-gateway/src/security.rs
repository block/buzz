use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use rand::Rng;
use sha2::{Digest, Sha256};
use uuid::Uuid;

pub fn random_token() -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

pub fn hash(value: &str) -> Vec<u8> {
    Sha256::digest(value.as_bytes()).to_vec()
}

pub fn hash_optional(value: Option<&str>) -> Option<Vec<u8>> {
    value.filter(|v| !v.is_empty()).map(hash)
}

pub fn short_pubkey(value: &str) -> String {
    if value.len() < 16 {
        return "invalid".into();
    }
    format!("{}…{}", &value[..8], &value[value.len() - 8..])
}

pub fn trace_id(value: Option<&str>) -> Uuid {
    value
        .and_then(|v| Uuid::parse_str(v).ok())
        .unwrap_or_else(Uuid::new_v4)
}

pub fn valid_pubkey(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

#[allow(clippy::too_many_arguments)]
pub fn canonical_binding_payload(
    id: Uuid,
    nonce: &str,
    issuer: &str,
    subject: &str,
    pubkey: &str,
    device_id: &str,
    issued_at: i64,
    expires_at: i64,
) -> String {
    // Fixed field order and LF separators are the signed protocol. Values are
    // length-bounded before this function and may not contain CR/LF.
    format!("bizfin-device-binding-v1\nchallenge_id={id}\nnonce={nonce}\naudience=bizfin-workbench-device-binding\noidc_issuer={issuer}\noidc_subject={subject}\nbuzz_pubkey={pubkey}\ndevice_id={device_id}\nissued_at={issued_at}\nexpires_at={expires_at}")
}

pub fn safe_target(path: &str) -> bool {
    if !path.starts_with("/embed/")
        || path.starts_with("//")
        || path.contains('\\')
        || path.contains('\r')
        || path.contains('\n')
    {
        return false;
    }
    let lowered = path.to_ascii_lowercase();
    !lowered.contains("%2f%2f")
        && !lowered.contains("%5c")
        && url::Url::parse(&format!("https://business.invalid{path}"))
            .map(|u| u.origin().ascii_serialization() == "https://business.invalid")
            .unwrap_or(false)
}

pub fn safe_text(value: &str, min: usize, max: usize) -> bool {
    let len = value.chars().count();
    (min..=max).contains(&len) && !value.contains(['\r', '\n', '\0'])
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn token_is_256_bit_base64url() {
        let v = random_token();
        assert_eq!(v.len(), 43);
        assert!(v
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b)));
    }
    #[test]
    fn target_policy_rejects_escape() {
        assert!(safe_target("/embed/sales/orders/SO-1"));
        assert!(!safe_target("https://evil.test/embed/x"));
        assert!(!safe_target("//evil.test/embed/x"));
        assert!(!safe_target("/embed/%5c%5cevil"));
    }
    #[test]
    fn canonical_payload_is_stable() {
        let id = Uuid::nil();
        assert_eq!(canonical_binding_payload(id,"n","i","s",&"a".repeat(64),"device-01",1,2), format!("bizfin-device-binding-v1\nchallenge_id={id}\nnonce=n\naudience=bizfin-workbench-device-binding\noidc_issuer=i\noidc_subject=s\nbuzz_pubkey={}\ndevice_id=device-01\nissued_at=1\nexpires_at=2", "a".repeat(64)));
    }
}
