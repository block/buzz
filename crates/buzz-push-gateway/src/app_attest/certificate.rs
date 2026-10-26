//! Apple certificate adapters; path and signature validation remain in webpki.
use super::error::AppAttestError as Error;
use aws_lc_rs::digest::{digest, SHA256};
use p256::elliptic_curve::sec1::ToSec1Point;
use rustls_pki_types::{CertificateDer, UnixTime};
use x509_parser::{certificate::X509Certificate, parse_x509_certificate};

static SIGNATURE_ALGORITHMS: &[&dyn rustls_pki_types::SignatureVerificationAlgorithm] = &[
    webpki::aws_lc_rs::ECDSA_P256_SHA256,
    webpki::aws_lc_rs::ECDSA_P256_SHA384,
    webpki::aws_lc_rs::ECDSA_P384_SHA256,
    webpki::aws_lc_rs::ECDSA_P384_SHA384,
];

// App Attest is not TLS. Preserve the supported Apple purpose profile while
// consuming and validating every OID; do not skip malformed EKU input.
struct AppleUsage;
impl webpki::ExtendedKeyUsageValidator for AppleUsage {
    fn validate(&self, iter: webpki::KeyPurposeIdIter<'_, '_>) -> Result<(), webpki::Error> {
        for oid in iter {
            oid?;
        }
        Ok(())
    }
}

fn parsed(bytes: &[u8]) -> Result<X509Certificate<'_>, Error> {
    let (rest, cert) = parse_x509_certificate(bytes).map_err(|_| Error::InvalidCertificate)?;
    if !rest.is_empty() {
        return Err(Error::InvalidCertificate);
    }
    // RFC 5280 requires each extension at most once, including unknown OIDs.
    cert.extensions_map()
        .map_err(|_| Error::InvalidCertificate)?;
    Ok(cert)
}

pub(super) fn verify_chain(certificates: &[Vec<u8>], root: &[u8]) -> Result<(), Error> {
    let leaf = certificates.first().ok_or(Error::InvalidCertificate)?;
    let root = CertificateDer::from(root);
    let anchor = webpki::anchor_from_trusted_cert(&root).map_err(|_| Error::InvalidCertificate)?;
    let leaf_der = CertificateDer::from(leaf.as_slice());
    let end = webpki::EndEntityCert::try_from(&leaf_der).map_err(|_| Error::InvalidCertificate)?;
    let intermediates: Vec<_> = certificates[1..]
        .iter()
        .map(|c| CertificateDer::from(c.as_slice()))
        .collect();
    end.verify_for_usage(
        SIGNATURE_ALGORITHMS,
        &[anchor],
        &intermediates,
        UnixTime::now(),
        AppleUsage,
        None,
        None,
    )
    .map_err(|_| Error::InvalidCertificate)?;
    for (index, bytes) in certificates.iter().enumerate() {
        let cert = parsed(bytes)?;
        if let Some(usage) = cert.key_usage().map_err(|_| Error::InvalidCertificate)? {
            if (index == 0 && !usage.value.digital_signature())
                || (index > 0 && !usage.value.key_cert_sign())
            {
                return Err(Error::InvalidCertificate);
            }
        }
    }
    Ok(())
}

pub(super) fn public_key(bytes: &[u8]) -> Result<[u8; 65], Error> {
    let cert = parsed(bytes)?;
    let spki = cert.public_key();
    if spki.algorithm.algorithm.to_id_string() != "1.2.840.10045.2.1"
        || spki
            .algorithm
            .parameters
            .as_ref()
            .and_then(|p| p.as_oid().ok())
            .map(|p| p.to_id_string())
            .as_deref()
            != Some("1.2.840.10045.3.1.7")
        || spki.subject_public_key.unused_bits != 0
    {
        return Err(Error::InvalidPublicKey);
    }
    let key = p256::PublicKey::from_sec1_bytes(spki.subject_public_key.data.as_ref())
        .map_err(|_| Error::InvalidPublicKey)?;
    key.to_sec1_point(false)
        .as_bytes()
        .try_into()
        .map_err(|_| Error::InvalidPublicKey)
}

pub(super) fn verify_nonce(bytes: &[u8], expected: &[u8]) -> Result<(), Error> {
    let cert = parsed(bytes)?;
    let extension = cert
        .extensions()
        .iter()
        .find(|e| e.oid.to_id_string() == "1.2.840.113635.100.8.2")
        .ok_or(Error::InvalidNonce)?;
    // DER has exactly one encoding for SEQUENCE { [1] EXPLICIT OCTET STRING
    // (SIZE(32)) }. Comparing that complete encoding avoids a custom DER walker
    // and rejects extra fields/trailing bytes/noncanonical lengths.
    let mut encoded = vec![0x30, 0x24, 0xa1, 0x22, 0x04, 0x20];
    if expected.len() != 32 {
        return Err(Error::InvalidNonce);
    }
    encoded.extend_from_slice(expected);
    if extension.value != encoded {
        return Err(Error::InvalidNonce);
    }
    Ok(())
}

pub(super) fn key_id(key: &[u8]) -> Vec<u8> {
    digest(&SHA256, key).as_ref().to_vec()
}
