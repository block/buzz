//! Synthetic signed fixtures. Adapted from appattest 0.1.1; see LICENSE.
use aws_lc_rs::digest::{digest, Context as DigestContext, SHA256};
use base64::{engine::general_purpose, Engine};
use ciborium::{cbor, Value};
use openssl::{
    asn1::{Asn1Integer, Asn1Object, Asn1OctetString, Asn1Time},
    bn::BigNum,
    ec::{EcGroup, EcKey, PointConversionForm},
    hash::{hash, MessageDigest},
    nid::Nid,
    pkey::PKey,
    rand::rand_bytes,
    x509::{
        extension::{BasicConstraints, ExtendedKeyUsage},
        X509Builder, X509NameBuilder, X509,
    },
};
use p256::ecdsa::{signature::Signer, DerSignature, SigningKey};

/// PEM bytes of the test root CA certificate (P-384).
/// Pass this to `verify_with_cert` / `app_id_verifies_with_cert` in tests.
pub const TEST_ROOT_CA_CERT_PEM: &[u8] = b"-----BEGIN CERTIFICATE-----\n\
MIICNzCCAb6gAwIBAgIUFeZt+1JiuWUm7RJtzNA3etiyaTgwCgYIKoZIzj0EAwIw\n\
SzElMCMGA1UEAwwcVGVzdCBBcHAgQXR0ZXN0YXRpb24gUm9vdCBDQTENMAsGA1UE\n\
CgwEVGVzdDETMBEGA1UECAwKQ2FsaWZvcm5pYTAeFw0yNjAyMjcxODU2MTBaFw00\n\
NjAyMjIxODU2MTBaMEsxJTAjBgNVBAMMHFRlc3QgQXBwIEF0dGVzdGF0aW9uIFJv\n\
b3QgQ0ExDTALBgNVBAoMBFRlc3QxEzARBgNVBAgMCkNhbGlmb3JuaWEwdjAQBgcq\n\
hkjOPQIBBgUrgQQAIgNiAAT2cBxT0pWtGUECFzUn+Hdx6MaDyVoOHFcjsd+wqlKZ\n\
IKNg+bopRItVz79g5Rn3F7126h7Q6ZEosNMCMiV2u1t47NBVGAvs0FtM/gqN2hZv\n\
4vy3F2l3Vwd2TatWzpRC0iKjYzBhMB0GA1UdDgQWBBR9vyJAdCH/3azKhgGa9JSh\n\
5LVM+DAfBgNVHSMEGDAWgBR9vyJAdCH/3azKhgGa9JSh5LVM+DAPBgNVHRMBAf8E\n\
BTADAQH/MA4GA1UdDwEB/wQEAwIBBjAKBggqhkjOPQQDAgNnADBkAjBi8yL62XLW\n\
g604KgEaEn9IFxR2c4vx+K0ZhYjPq6OU8X4pq9757JityNdJW1THgxgCMAo9bL44\n\
UUoFJPTgvmKobdPOIUQ1oRNRYnGmtfovWa31WM2IK0mOs+lpWYncFZOr3Q==\n\
-----END CERTIFICATE-----\n";

const TEST_INT_CA_CERT_PEM: &[u8] = b"-----BEGIN CERTIFICATE-----\n\
MIICHDCCAaGgAwIBAgIUYM1hx4yf7fLmLiIQ57Ov3d5h27cwCgYIKoZIzj0EAwIw\n\
SzElMCMGA1UEAwwcVGVzdCBBcHAgQXR0ZXN0YXRpb24gUm9vdCBDQTENMAsGA1UE\n\
CgwEVGVzdDETMBEGA1UECAwKQ2FsaWZvcm5pYTAeFw0yNjAyMjcxODU2MTBaFw00\n\
NjAyMjIxODU2MTBaMEgxIjAgBgNVBAMMGVRlc3QgQXBwIEF0dGVzdGF0aW9uIENB\n\
IDExDTALBgNVBAoMBFRlc3QxEzARBgNVBAgMCkNhbGlmb3JuaWEwWTATBgcqhkjO\n\
PQIBBggqhkjOPQMBBwNCAARXQJU3qZo9zCz2xZpq0ZyFfYN+WpG6uEdt++GKRcaj\n\
jmJiMYBpxOBtrecCgvPlrZdS7UlHhmUg2Zdfnvu7+DEIo2YwZDASBgNVHRMBAf8E\n\
CDAGAQH/AgEAMB0GA1UdDgQWBBQzJ5DNIxBnjG4jq4wZ0ZgB+4SYsDAfBgNVHSME\n\
GDAWgBR9vyJAdCH/3azKhgGa9JSh5LVM+DAOBgNVHQ8BAf8EBAMCAQYwCgYIKoZI\n\
zj0EAwIDaQAwZgIxAJ2KT9ZsXtEtvemPNnp5FsyM1WEli1qztnjhjDV5qHcCFydq\n\
g8rl2BjX1qwXyl9odQIxAMXiN2QWgtbC7VqNUjZFevnwVPwXgkRr+g6mqKjWVTTf\n\
rGDANtkIDXqrnt8GyCZQWw==\n\
-----END CERTIFICATE-----\n";

// Published upstream synthetic CA key; the gateway pins Apple's root and never trusts this CA.
const TEST_INT_CA_KEY_PEM: &[u8] = include_bytes!("../../tests/fixtures/appattest-test-ca-key.pem");

/// Output of [`build_test_attestation`].
pub struct TestAttestation {
    /// CBOR-encoded attestation bytes. Base64-encode and pass to
    /// `Attestation::from_base64`, or decode directly via
    /// `Attestation::from_cbor` if exposed.
    pub cbor: Vec<u8>,
    /// Base64-encoded key ID (standard encoding). Pass to `verify` /
    /// `app_id_verifies`.
    pub key_id: String,
    /// The device's P-256 private key. Keep this to sign assertions in
    /// subsequent calls.
    pub device_key: SigningKey,
}

/// Build a fully valid synthetic attestation signed by the embedded test CA.
///
/// The returned [`TestAttestation`] will pass
/// `Attestation::app_id_verifies(challenge, &[app_id], &key_id, TEST_ROOT_CA_CERT_PEM)`.
pub fn build_test_attestation(challenge: &str, app_id: &str) -> TestAttestation {
    build_test_attestation_with_auth_data(challenge, app_id, |_| {})
}

/// Build a signed fixture after deliberately varying authenticator fields.
/// This permits independent positive and negative structural test cases.
pub fn build_test_attestation_with_auth_data(
    challenge: &str,
    app_id: &str,
    change: impl FnOnce(&mut Vec<u8>),
) -> TestAttestation {
    // Generate a P-256 device keypair via openssl, then import the scalar into
    // p256 for signing.
    let group = EcGroup::from_curve_name(Nid::X9_62_PRIME256V1).unwrap();
    let openssl_key = EcKey::generate(&group).unwrap();

    let scalar_bytes = openssl_key.private_key().to_vec();
    // p256 expects a 32-byte scalar; left-pad if shorter.
    let mut scalar32 = [0u8; 32];
    let off = 32 - scalar_bytes.len();
    scalar32[off..].copy_from_slice(&scalar_bytes);
    let signing_key = SigningKey::from_bytes(&scalar32.into()).unwrap();

    let device_pkey = PKey::from_ec_key(openssl_key.clone()).unwrap();

    // key_id = base64(SHA256(uncompressed public key bytes))
    let mut ctx = openssl::bn::BigNumContext::new().unwrap();
    let pub_key_bytes = openssl_key
        .public_key()
        .to_bytes(&group, PointConversionForm::UNCOMPRESSED, &mut ctx)
        .unwrap();
    let pub_key_hash = hash(MessageDigest::sha256(), &pub_key_bytes).unwrap();
    let key_id = general_purpose::STANDARD.encode(pub_key_hash.as_ref());
    let key_id_bytes = pub_key_hash.to_vec();

    // authenticator data layout:
    //   [0..32]  rp_id_hash  = SHA256(app_id)
    //   [32]     flags       = 0x41 (attested credential data included)
    //   [33..37] counter     = 0u32 big-endian
    //   [37..53] aaguid      = b"appattest\0\0\0\0\0\0\0" (16 bytes)
    //   [53..55] cred_id_len = 32u16 big-endian
    //   [55..87] cred_id     = key_id raw bytes
    let rp_id_hash: [u8; 32] = digest(&SHA256, app_id.as_bytes())
        .as_ref()
        .try_into()
        .unwrap();
    let mut aaguid = [0u8; 16];
    aaguid[..9].copy_from_slice(b"appattest");

    let mut auth_data = Vec::with_capacity(87);
    auth_data.extend_from_slice(&rp_id_hash);
    auth_data.push(0x41);
    auth_data.extend_from_slice(&[0u8; 4]);
    auth_data.extend_from_slice(&aaguid);
    let mut len_bytes = [0u8; 2];
    len_bytes.copy_from_slice(&(key_id_bytes.len() as u16).to_be_bytes());
    auth_data.extend_from_slice(&len_bytes);
    auth_data.extend_from_slice(&key_id_bytes);
    let cose = Value::Map(vec![
        (Value::Integer(1.into()), Value::Integer(2.into())),
        (Value::Integer(3.into()), Value::Integer((-7).into())),
        (Value::Integer((-1).into()), Value::Integer(1.into())),
        (
            Value::Integer((-2).into()),
            Value::Bytes(pub_key_bytes[1..33].to_vec()),
        ),
        (
            Value::Integer((-3).into()),
            Value::Bytes(pub_key_bytes[33..65].to_vec()),
        ),
    ]);
    ciborium::into_writer(&cose, &mut auth_data).unwrap();
    change(&mut auth_data);

    // nonce = SHA256(authData || SHA256(challenge))
    let client_data_hash: [u8; 32] = digest(&SHA256, challenge.as_bytes())
        .as_ref()
        .try_into()
        .unwrap();
    let nonce: [u8; 32] = {
        let mut ctx = DigestContext::new(&SHA256);
        ctx.update(&auth_data);
        ctx.update(&client_data_hash);
        ctx.finish().as_ref().try_into().unwrap()
    };

    let cred_cert_der = build_cred_cert(&device_pkey, nonce.as_slice(), &key_id);
    let int_cert_der = X509::from_pem(TEST_INT_CA_CERT_PEM)
        .unwrap()
        .to_der()
        .unwrap();

    let cbor = encode_attestation_cbor(&cred_cert_der, &int_cert_der, &auth_data);

    TestAttestation {
        cbor,
        key_id,
        device_key: signing_key,
    }
}

/// Build a synthetic assertion signed by the given device private key.
///
/// Returns raw CBOR bytes ready for `Assertion::from_assertion`.
///
/// `previous_counter` should be the counter value stored server-side;
/// the assertion will use `previous_counter + 1`.
pub fn build_test_assertion(
    app_id: &str,
    client_data_hash: impl AsRef<[u8]>,
    previous_counter: u32,
    device_key: &SigningKey,
) -> Vec<u8> {
    let rp_id_hash: [u8; 32] = digest(&SHA256, app_id.as_bytes())
        .as_ref()
        .try_into()
        .unwrap();
    let counter = previous_counter + 1;

    let mut auth_data = Vec::with_capacity(37);
    auth_data.extend_from_slice(&rp_id_hash);
    auth_data.push(0x01);
    let mut counter_bytes = [0u8; 4];
    counter_bytes.copy_from_slice(&counter.to_be_bytes());
    auth_data.extend_from_slice(&counter_bytes);

    let nonce: Vec<u8> = {
        let mut ctx = DigestContext::new(&SHA256);
        ctx.update(&auth_data);
        ctx.update(client_data_hash.as_ref());
        ctx.finish().as_ref().to_vec()
    };

    let sig: DerSignature = device_key.sign(&nonce);
    encode_assertion_cbor(&auth_data, sig.as_bytes())
}

fn build_cred_cert(
    device_pkey: &PKey<openssl::pkey::Private>,
    nonce: &[u8],
    key_id: &str,
) -> Vec<u8> {
    build_cred_cert_with(device_pkey, nonce, key_id, |_| {})
}

fn build_cred_cert_with(
    device_pkey: &PKey<openssl::pkey::Private>,
    nonce: &[u8],
    key_id: &str,
    change: impl FnOnce(&mut X509Builder),
) -> Vec<u8> {
    assert_eq!(nonce.len(), 32);

    let int_cert = X509::from_pem(TEST_INT_CA_CERT_PEM).unwrap();
    let int_key = PKey::private_key_from_pem(TEST_INT_CA_KEY_PEM).unwrap();

    let mut name_builder = X509NameBuilder::new().unwrap();
    name_builder.append_entry_by_text("CN", key_id).unwrap();
    name_builder
        .append_entry_by_text("OU", "AAA Certification")
        .unwrap();
    name_builder.append_entry_by_text("O", "Test").unwrap();
    name_builder
        .append_entry_by_text("ST", "California")
        .unwrap();
    let subject_name = name_builder.build();

    let mut builder = X509Builder::new().unwrap();
    builder.set_version(2).unwrap();
    builder.set_subject_name(&subject_name).unwrap();
    builder.set_issuer_name(int_cert.subject_name()).unwrap();
    builder.set_pubkey(device_pkey).unwrap();
    builder
        .set_not_before(&Asn1Time::days_from_now(0).unwrap())
        .unwrap();
    builder
        .set_not_after(&Asn1Time::days_from_now(365).unwrap())
        .unwrap();

    let mut serial_bytes = [0u8; 8];
    rand_bytes(&mut serial_bytes).unwrap();
    let serial_asn1 = Asn1Integer::from_bn(&BigNum::from_slice(&serial_bytes).unwrap()).unwrap();
    builder.set_serial_number(&serial_asn1).unwrap();

    builder
        .append_extension(BasicConstraints::new().build().unwrap())
        .unwrap();

    // Apple's proprietary App Attest EKU (OID 1.2.840.113635.100.4.24).
    builder
        .append_extension(
            ExtendedKeyUsage::new()
                .other("1.2.840.113635.100.4.24")
                .build()
                .unwrap(),
        )
        .unwrap();

    // Nonce extension OID 1.2.840.113635.100.8.2, encoding:
    //   30 24        SEQUENCE, length 36
    //     a1 22      [1] EXPLICIT, length 34
    //       04 20    OCTET STRING, length 32
    //         <nonce bytes>
    //
    // `extract_nonce_from_cert` walks this with `der_unwrap_tag`, stripping the
    // 04 20 header to yield the raw 32-byte nonce.
    let mut ext_value = Vec::with_capacity(38);
    ext_value.extend_from_slice(&[0x30, 0x24, 0xa1, 0x22, 0x04, 0x20]);
    ext_value.extend_from_slice(nonce);

    let oid = Asn1Object::from_str("1.2.840.113635.100.8.2").unwrap();
    let octet_val = Asn1OctetString::new_from_bytes(&ext_value).unwrap();
    builder
        .append_extension(
            openssl::x509::X509Extension::new_from_der(&oid, false, &octet_val).unwrap(),
        )
        .unwrap();

    change(&mut builder);
    builder.sign(&int_key, MessageDigest::sha256()).unwrap();
    builder.build().to_der().unwrap()
}

fn encode_attestation_cbor(cred_cert_der: &[u8], int_cert_der: &[u8], auth_data: &[u8]) -> Vec<u8> {
    let val = cbor!({
        "fmt" => "apple-appattest",
        "attStmt" => {
            "x5c" => [
                Value::Bytes(cred_cert_der.to_vec()),
                Value::Bytes(int_cert_der.to_vec())
            ],
            "receipt" => Value::Bytes(vec![])
        },
        "authData" => Value::Bytes(auth_data.to_vec())
    })
    .unwrap();

    let mut buf = Vec::new();
    ciborium::into_writer(&val, &mut buf).unwrap();
    buf
}

fn encode_assertion_cbor(auth_data: &[u8], signature: &[u8]) -> Vec<u8> {
    let val = cbor!({
        "authenticatorData" => Value::Bytes(auth_data.to_vec()),
        "signature" => Value::Bytes(signature.to_vec())
    })
    .unwrap();

    let mut buf = Vec::new();
    ciborium::into_writer(&val, &mut buf).unwrap();
    buf
}

fn certificate_chain(change: impl FnOnce(&mut X509Builder)) -> Vec<Vec<u8>> {
    let group = EcGroup::from_curve_name(Nid::X9_62_PRIME256V1).unwrap();
    let key = PKey::from_ec_key(EcKey::generate(&group).unwrap()).unwrap();
    vec![
        build_cred_cert_with(&key, &[7; 32], "test", change),
        X509::from_pem(TEST_INT_CA_CERT_PEM)
            .unwrap()
            .to_der()
            .unwrap(),
    ]
}

/// Signed certificate chains that each break one path or usage constraint.
pub fn rejected_certificate_chains() -> Vec<(&'static str, Vec<Vec<u8>>)> {
    use openssl::x509::extension::KeyUsage;
    let wrong_usage = certificate_chain(|builder| {
        builder
            .append_extension(KeyUsage::new().key_encipherment().build().unwrap())
            .unwrap();
    });
    let expired = certificate_chain(|builder| {
        builder
            .set_not_before(&Asn1Time::from_unix(0).unwrap())
            .unwrap();
        builder
            .set_not_after(&Asn1Time::from_unix(1).unwrap())
            .unwrap();
    });
    let unknown_critical = certificate_chain(|builder| {
        let oid = Asn1Object::from_str("1.2.3.4.5").unwrap();
        let value = Asn1OctetString::new_from_bytes(b"\x05\x00").unwrap();
        builder
            .append_extension(
                openssl::x509::X509Extension::new_from_der(&oid, true, &value).unwrap(),
            )
            .unwrap();
    });
    let mut trailing = valid_certificate_chain();
    trailing[0].push(0);
    vec![
        ("wrong usage", wrong_usage),
        ("expired", expired),
        ("unknown critical", unknown_critical),
        ("duplicate nonce", duplicate_nonce_chain()),
        ("trailing", trailing),
    ]
}

/// A signed chain that satisfies every path and usage constraint.
pub fn valid_certificate_chain() -> Vec<Vec<u8>> {
    use openssl::x509::extension::KeyUsage;
    certificate_chain(|builder| {
        builder
            .append_extension(KeyUsage::new().digital_signature().build().unwrap())
            .unwrap();
    })
}

fn duplicate_nonce_chain() -> Vec<Vec<u8>> {
    certificate_chain(|builder| {
        let oid = Asn1Object::from_str("1.2.840.113635.100.8.2").unwrap();
        let value = Asn1OctetString::new_from_bytes(b"\x30\x00").unwrap();
        builder
            .append_extension(
                openssl::x509::X509Extension::new_from_der(&oid, false, &value).unwrap(),
            )
            .unwrap();
    })
}

/// DER of the test root that signs every chain above.
pub fn test_root_der() -> Vec<u8> {
    X509::from_pem(TEST_ROOT_CA_CERT_PEM)
        .unwrap()
        .to_der()
        .unwrap()
}

/// Run the same signed-certificate corpus against an alternative adapter.
pub fn compare_certificate_constraints<E: std::fmt::Debug>(
    verify_chain: impl Fn(&[Vec<u8>], &[u8]) -> Result<(), E>,
    verify_nonce: impl Fn(&[u8], &[u8]) -> Result<(), E>,
) {
    let root = test_root_der();
    verify_chain(&valid_certificate_chain(), &root).unwrap();
    for (name, chain) in rejected_certificate_chains() {
        assert!(verify_chain(&chain, &root).is_err(), "{name}");
    }
    assert!(verify_nonce(&duplicate_nonce_chain()[0], &[7; 32]).is_err());
}
