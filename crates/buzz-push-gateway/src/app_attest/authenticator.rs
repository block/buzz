//! Operation-specific framing. No optional credential state exists on assertions.
use super::{
    cbor::{self, Value},
    error::AppAttestError as Error,
    properties::ClientProperties,
};
use aws_lc_rs::digest::{digest, SHA256};
use p256::elliptic_curve::sec1::ToEncodedPoint;

pub(super) struct Checked {
    pub counter: u32,
    pub flags: u8,
    pub properties: ClientProperties,
}

fn header(bytes: &[u8], app_id: &str) -> Result<Checked, Error> {
    let prefix = bytes.get(..37).ok_or(Error::AuthenticatorDataTooShort)?;
    if prefix[..32] != *digest(&SHA256, app_id.as_bytes()).as_ref() {
        return Err(Error::InvalidAppID);
    }
    Ok(Checked {
        counter: u32::from_be_bytes(
            prefix[33..37]
                .try_into()
                .map_err(|_| Error::AuthenticatorDataTooShort)?,
        ),
        flags: prefix[32],
        properties: ClientProperties::default(),
    })
}

fn properties(tail: &[u8]) -> Result<ClientProperties, Error> {
    // Frame by operation and parsed CBOR, not AT/ED flags: Apple's published
    // enrollment sample includes extensions with ED clear.
    if tail.is_empty() {
        return Ok(ClientProperties::default());
    }
    ClientProperties::from_extensions(&cbor::complete(tail)?)
}

pub(super) fn assertion(bytes: &[u8], app_id: &str, previous: u32) -> Result<Checked, Error> {
    let mut checked = header(bytes, app_id)?;
    if checked.counter <= previous {
        return Err(Error::InvalidCounter);
    }
    checked.properties = properties(&bytes[37..])?;
    Ok(checked)
}

pub(super) fn enrollment(
    bytes: &[u8],
    app_id: &str,
    key_id: &[u8],
    public_key: &[u8; 65],
    development: bool,
) -> Result<Checked, Error> {
    let mut checked = header(bytes, app_id)?;
    if checked.counter != 0 {
        return Err(Error::InvalidCounter);
    }
    let credential_prefix = bytes.get(..55).ok_or(Error::AuthenticatorDataTooShort)?;
    let expected = if development {
        b"appattestdevelop"
    } else {
        b"appattest\0\0\0\0\0\0\0"
    };
    if &credential_prefix[37..53] != expected {
        return Err(Error::InvalidAAGUID);
    }
    // App Attest credential IDs are SHA-256 hashes, always 32 bytes. Preserve
    // appattest 0.1.1's workaround for reported iOS 26.x nonstandard length
    // fields. The original length bytes remain authenticated by the nonce;
    // the credential, supplied key ID, COSE key and certificate must agree.
    const END: usize = 55 + 32;
    let end = END;
    if bytes.get(55..end).ok_or(Error::AuthenticatorDataTooShort)? != key_id {
        return Err(Error::InvalidCredentialID);
    }
    let tail = bytes.get(end..).ok_or(Error::AuthenticatorDataTooShort)?;
    let (key, consumed) = credential_key(tail)?;
    if &key != public_key {
        return Err(Error::InvalidPublicKey);
    }
    checked.properties = properties(&tail[consumed..])?;
    Ok(checked)
}

fn credential_key(tail: &[u8]) -> Result<([u8; 65], usize), Error> {
    let (cose, consumed) = cbor::prefix(tail)?;
    let map = cose.map()?;
    if !map
        .keys()
        .all(|k| matches!(k, Value::Integer(_) | Value::Text(_)))
    {
        return Err(Error::InvalidPublicKey);
    }
    for (label, expected) in [(1, 2), (3, -7), (-1, 1)] {
        if map.get(&Value::Integer(label)) != Some(&Value::Integer(expected)) {
            return Err(Error::InvalidPublicKey);
        }
    }
    if let Some(operations) = map.get(&Value::Integer(4)) {
        match operations {
            Value::Array(v)
                if v.iter().all(|op| matches!(op, Value::Integer(_)))
                    && v.contains(&Value::Integer(2)) => {}
            _ => return Err(Error::InvalidPublicKey),
        }
    }
    let x = map
        .get(&Value::Integer(-2))
        .ok_or(Error::InvalidPublicKey)?
        .bytes()?;
    if x.len() != 32 {
        return Err(Error::InvalidPublicKey);
    }
    let mut encoded = Vec::with_capacity(65);
    match map.get(&Value::Integer(-3)) {
        Some(Value::Bytes(y)) if y.len() == 32 => {
            encoded.push(4);
            encoded.extend_from_slice(x);
            encoded.extend_from_slice(y);
        }
        Some(Value::Simple(parity @ (20 | 21))) => {
            encoded.push(if *parity == 21 { 3 } else { 2 });
            encoded.extend_from_slice(x);
        }
        _ => return Err(Error::InvalidPublicKey),
    }
    let key = p256::PublicKey::from_sec1_bytes(&encoded).map_err(|_| Error::InvalidPublicKey)?;

    Ok((
        key.to_encoded_point(false)
            .as_bytes()
            .try_into()
            .map_err(|_| Error::InvalidPublicKey)?,
        consumed,
    ))
}
