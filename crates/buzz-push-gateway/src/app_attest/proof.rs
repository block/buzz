//! Buzz's two App Attest operations, using established signature and path validators.
use super::{
    authenticator,
    cbor::{self, Value},
    certificate,
    error::AppAttestError as Error,
    properties::ClientProperties,
};
use aws_lc_rs::{
    digest::{digest, Context, SHA256},
    signature::{UnparsedPublicKey, ECDSA_P256_SHA256_ASN1},
};
use base64::{engine::general_purpose::STANDARD, Engine};

pub(super) struct Enrollment {
    pub key_id: Vec<u8>,
    pub public_key: [u8; 65],
    pub flags: u8,
    pub properties: ClientProperties,
}

pub(super) struct Assertion {
    pub counter: u32,
    pub flags: u8,
    pub properties: ClientProperties,
}

// Both operations authenticate precisely the received bytes with this nonce.
fn nonce(auth: &[u8], transcript: &[u8]) -> aws_lc_rs::digest::Digest {
    let mut hash = Context::new(&SHA256);
    hash.update(auth);
    hash.update(digest(&SHA256, transcript).as_ref());
    hash.finish()
}

pub(super) fn enroll(
    bytes: &[u8],
    transcript: &[u8],
    app_id: &str,
    key_id: &str,
    root_pem: &[u8],
    development: bool,
) -> Result<Enrollment, Error> {
    let envelope = cbor::complete(bytes)?;
    envelope.text_keys()?;
    if envelope.field("fmt")? != &Value::Text("apple-appattest".into()) {
        return Err(Error::InvalidFormat);
    }
    let statement = envelope.field("attStmt")?;
    statement.text_keys()?;
    let Value::Array(chain) = statement.field("x5c")? else {
        return Err(Error::InvalidCertificate);
    };
    let certificates = chain
        .iter()
        .map(|c| c.bytes().map(|b| b.to_vec()))
        .collect::<Result<Vec<_>, _>>()?;
    let leaf = certificates.first().ok_or(Error::InvalidCertificate)?;
    statement.field("receipt")?.bytes()?;
    let auth_bytes = envelope.field("authData")?.bytes()?;
    if root_pem.len() > cbor::MAX_BYTES {
        return Err(Error::ResourceLimit);
    }
    let (rest, root) =
        x509_parser::pem::parse_x509_pem(root_pem).map_err(|_| Error::InvalidCertificate)?;
    if root.label != "CERTIFICATE" || !rest.iter().all(u8::is_ascii_whitespace) {
        return Err(Error::InvalidCertificate);
    }
    certificate::verify_chain(&certificates, &root.contents)?;
    let public_key = certificate::public_key(leaf)?;
    if key_id.len() > 44 {
        return Err(Error::InvalidPublicKey);
    }
    let key_id = STANDARD
        .decode(key_id)
        .map_err(|_| Error::InvalidPublicKey)?;
    if key_id.len() != 32 || certificate::key_id(&public_key) != key_id {
        return Err(Error::InvalidPublicKey);
    }
    certificate::verify_nonce(leaf, nonce(auth_bytes, transcript).as_ref())?;
    let auth = authenticator::enrollment(auth_bytes, app_id, &key_id, &public_key, development)?;
    Ok(Enrollment {
        key_id,
        public_key,
        flags: auth.flags,
        properties: auth.properties,
    })
}

pub(super) fn assert(
    bytes: &[u8],
    transcript: &[u8],
    app_id: &str,
    public_key: &[u8],
    previous_counter: u32,
) -> Result<Assertion, Error> {
    let envelope = cbor::complete(bytes)?;
    envelope.text_keys()?;
    let auth_bytes = envelope.field("authenticatorData")?.bytes()?;
    let signature = envelope.field("signature")?.bytes()?;
    UnparsedPublicKey::new(&ECDSA_P256_SHA256_ASN1, public_key)
        .verify(nonce(auth_bytes, transcript).as_ref(), signature)
        .map_err(|_| Error::InvalidSignature)?;
    let auth = authenticator::assertion(auth_bytes, app_id, previous_counter)?;
    Ok(Assertion {
        counter: auth.counter,
        flags: auth.flags,
        properties: auth.properties,
    })
}
