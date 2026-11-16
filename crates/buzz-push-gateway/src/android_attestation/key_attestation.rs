//! The production Android Key Attestation verifier.
//!
//! Follows Google's verification guidance
//! (<https://developer.android.com/privacy-and-security/security-key-attestation>):
//! the chain must end at a pinned Google root, every signature must verify,
//! no certificate may be on the revocation list, and the attestation
//! extension is read only from the hardware-enforced list, except for the
//! application id, which the operating system writes into the
//! software-enforced list.

use std::sync::Arc;

use base64::{engine::general_purpose::STANDARD, Engine as _};
use p256::elliptic_curve::sec1::ToSec1Point;
use sha2::{Digest, Sha256};
use x509_parser::{certificate::X509Certificate, parse_x509_certificate, time::ASN1Time};

use super::{
    key_description::{
        self, KeyDescription, VerifiedBootState, KEY_DESCRIPTION_OID, STRONG_BOX,
        TRUSTED_ENVIRONMENT,
    },
    revocation::{serial_key, RevocationList},
    AndroidAttestationError, AndroidAttestationVerifier, VerifiedAndroidKey,
};

/// Google's published hardware attestation roots, pinned by public key.
pub const GOOGLE_ROOTS_PEM: &str = include_str!("google_roots.pem");

/// Present in certificates issued through Remote Key Provisioning. Their short
/// validity is part of the threat model, so it is enforced; the long-expired
/// factory chains Google tells verifiers to keep trusting have no such
/// extension and are covered by the revocation list instead.
const PROVISIONING_INFO_OID: &str = "1.3.6.1.4.1.11129.2.1.30";

const EC_PUBLIC_KEY_OID: &str = "1.2.840.10045.2.1";
const PRIME256V1_OID: &str = "1.2.840.10045.3.1.7";
const SHA1_SIGNATURE_OIDS: [&str; 3] =
    ["1.2.840.113549.1.1.5", "1.3.14.3.2.29", "1.2.840.10045.4.1"];

// Android `Algorithm`, `EcCurve`, `KeyOrigin` and `KeyPurpose` values.
const ALGORITHM_EC: u64 = 3;
const CURVE_P256: u64 = 1;
const ORIGIN_GENERATED: u64 = 0;
const PURPOSE_SIGN: u64 = 2;

/// What the app and the operator accept in an attestation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyAttestationPolicy {
    /// The one package the attestation must name.
    pub package_name: String,
    /// Accepted SHA-256 digests of the app's signing certificate.
    pub signing_cert_digests: Vec<[u8; 32]>,
    /// Verified boot keys (SHA-256 digests) accepted with a SelfSigned boot
    /// state. Empty accepts stock Android only.
    pub boot_key_digests: Vec<[u8; 32]>,
}

/// A trust root or the verifier's setup was unusable.
#[derive(Debug, thiserror::Error)]
#[error("invalid Android attestation roots")]
pub struct RootsError;

/// Verifies Android Key Attestation chains offline against pinned roots and
/// the in-memory revocation list.
pub struct KeyAttestationVerifier {
    /// SubjectPublicKeyInfo DER of each trusted root.
    root_keys: Vec<Vec<u8>>,
    policy: KeyAttestationPolicy,
    revocations: Arc<RevocationList>,
    now: fn() -> i64,
}

/// Split a PEM bundle into DER certificates. Text outside the blocks, such
/// as comments, is ignored.
fn pem_certificates(pem: &str) -> Result<Vec<Vec<u8>>, RootsError> {
    const BEGIN: &str = "-----BEGIN CERTIFICATE-----";
    const END: &str = "-----END CERTIFICATE-----";
    let mut out = Vec::new();
    let mut rest = pem;
    while let Some(start) = rest.find(BEGIN) {
        let after = &rest[start + BEGIN.len()..];
        let end = after.find(END).ok_or(RootsError)?;
        let body: String = after[..end].split_whitespace().collect();
        out.push(STANDARD.decode(body).map_err(|_| RootsError)?);
        rest = &after[end + END.len()..];
    }
    if out.is_empty() {
        return Err(RootsError);
    }
    Ok(out)
}

impl KeyAttestationVerifier {
    /// Build a verifier that trusts the roots in `roots_pem`, usually
    /// [`GOOGLE_ROOTS_PEM`].
    pub fn new(
        roots_pem: &str,
        policy: KeyAttestationPolicy,
        revocations: Arc<RevocationList>,
        now: fn() -> i64,
    ) -> Result<Self, RootsError> {
        let root_keys = pem_certificates(roots_pem)?
            .iter()
            .map(|der| {
                let (rest, cert) = parse_x509_certificate(der).map_err(|_| RootsError)?;
                rest.is_empty()
                    .then(|| cert.public_key().raw.to_vec())
                    .ok_or(RootsError)
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            root_keys,
            policy,
            revocations,
            now,
        })
    }

    fn check(
        &self,
        chain: &[Vec<u8>],
        client_data: &[u8],
    ) -> Result<VerifiedAndroidKey, &'static str> {
        let now = (self.now)();
        let certs = chain
            .iter()
            .map(|der| match parse_x509_certificate(der) {
                Ok(([], cert)) => Ok(cert),
                _ => Err("certificate_encoding"),
            })
            .collect::<Result<Vec<_>, _>>()?;
        if certs.len() < 2 {
            return Err("chain_too_short");
        }
        // RFC 5280 allows each extension once; a duplicate hides a second
        // attestation extension from a first-match lookup.
        for cert in &certs {
            cert.extensions_map().map_err(|_| "certificate_encoding")?;
        }
        self.check_path(&certs, now)?;
        let serials: Vec<String> = certs
            .iter()
            .map(|cert| serial_key(cert.raw_serial()))
            .collect();
        self.revocations
            .check(&serials, now)
            .map_err(|failure| match failure {
                super::revocation::RevocationFailure::Unavailable => "revocation_list_unavailable",
                super::revocation::RevocationFailure::Stale => "revocation_list_stale",
                super::revocation::RevocationFailure::Listed => "certificate_revoked",
            })?;
        let description = attestation_extension(&certs)?;
        self.check_description(&description, client_data)?;
        Ok(VerifiedAndroidKey {
            public_key: p256_public_key(&certs[0])?,
        })
    }

    /// Pinned root, issuer links, signatures, CA flags and validity.
    fn check_path(&self, certs: &[X509Certificate<'_>], now: i64) -> Result<(), &'static str> {
        let root = certs.last().ok_or("chain_too_short")?;
        if !self
            .root_keys
            .iter()
            .any(|key| key.as_slice() == root.public_key().raw)
        {
            return Err("untrusted_root");
        }
        root.verify_signature(None).map_err(|_| "bad_signature")?;
        for pair in certs.windows(2) {
            let (cert, issuer) = (&pair[0], &pair[1]);
            if cert.issuer() != issuer.subject() {
                return Err("issuer_mismatch");
            }
            cert.verify_signature(Some(issuer.public_key()))
                .map_err(|_| "bad_signature")?;
        }
        for cert in certs {
            let algorithm = cert.signature_algorithm.algorithm.to_id_string();
            if SHA1_SIGNATURE_OIDS.contains(&algorithm.as_str()) {
                return Err("weak_signature");
            }
        }
        // The attestation key certificate (`certs[1]`) is exempt: some factory
        // keyboxes issue it with CA:FALSE and digitalSignature only (Google's
        // Sony Xperia 10 III sample), and Google's validator never checks these
        // flags. A Keystore leaf cannot stand in for it, because a
        // KeyDescription above the leaf is refused.
        for issuer in certs.iter().skip(2) {
            let is_ca = matches!(issuer.basic_constraints(), Ok(Some(ext)) if ext.value.ca);
            let may_sign = match issuer.key_usage() {
                Ok(Some(usage)) => usage.value.key_cert_sign(),
                Ok(None) => true,
                Err(_) => false,
            };
            if !is_ca || !may_sign {
                return Err("issuer_not_a_ca");
            }
        }
        let provisioned = certs[1..]
            .iter()
            .any(|cert| has_extension(cert, PROVISIONING_INFO_OID));
        if provisioned {
            let at = ASN1Time::from_timestamp(now).map_err(|_| "clock")?;
            if certs.iter().any(|cert| !cert.validity().is_valid_at(at)) {
                return Err("certificate_expired");
            }
        }
        Ok(())
    }

    fn check_description(
        &self,
        description: &KeyDescription,
        client_data: &[u8],
    ) -> Result<(), &'static str> {
        let hardware_level = |level: u64| level == TRUSTED_ENVIRONMENT || level == STRONG_BOX;
        if !hardware_level(description.attestation_security_level)
            || !hardware_level(description.keymaster_security_level)
        {
            return Err("not_hardware_backed");
        }
        if description.challenge.as_slice() != Sha256::digest(client_data).as_slice() {
            return Err("challenge_mismatch");
        }
        // Everything below comes from the hardware-enforced list only.
        let hardware = &description.hardware;
        if hardware.algorithm != Some(ALGORITHM_EC)
            || hardware.ec_curve != Some(CURVE_P256)
            || hardware.origin != Some(ORIGIN_GENERATED)
            || hardware.purposes.as_deref() != Some(&[PURPOSE_SIGN])
        {
            return Err("key_properties");
        }
        let root_of_trust = hardware.root_of_trust.as_ref().ok_or("no_root_of_trust")?;
        if !root_of_trust.device_locked {
            return Err("bootloader_unlocked");
        }
        match root_of_trust.verified_boot_state {
            VerifiedBootState::Verified => {}
            VerifiedBootState::SelfSigned => {
                if !self
                    .policy
                    .boot_key_digests
                    .iter()
                    .any(|digest| digest.as_slice() == root_of_trust.verified_boot_key)
                {
                    return Err("boot_key_not_accepted");
                }
            }
            VerifiedBootState::Unverified | VerifiedBootState::Failed => {
                return Err("boot_state_rejected")
            }
        }
        let application = description
            .application_id
            .as_ref()
            .ok_or("no_application_id")?;
        match application.package_names.as_slice() {
            [name] if name.as_slice() == self.policy.package_name.as_bytes() => {}
            _ => return Err("package_mismatch"),
        }
        if !application.signature_digests.iter().any(|digest| {
            self.policy
                .signing_cert_digests
                .iter()
                .any(|accepted| accepted.as_slice() == digest.as_slice())
        }) {
            return Err("signing_digest_mismatch");
        }
        Ok(())
    }
}

fn has_extension(cert: &X509Certificate<'_>, oid: &str) -> bool {
    cert.extensions()
        .iter()
        .any(|extension| extension.oid.to_id_string() == oid)
}

/// The attestation extension must be in the leaf and nowhere else: a copy in
/// any other certificate was not issued by the secure hardware.
fn attestation_extension(certs: &[X509Certificate<'_>]) -> Result<KeyDescription, &'static str> {
    if certs[1..]
        .iter()
        .any(|cert| has_extension(cert, KEY_DESCRIPTION_OID))
    {
        return Err("attestation_extension_misplaced");
    }
    // Uniqueness was checked with the whole chain's extension maps.
    let extension = certs[0]
        .extensions()
        .iter()
        .find(|extension| extension.oid.to_id_string() == KEY_DESCRIPTION_OID)
        .ok_or("attestation_extension_missing")?;
    key_description::parse(extension.value).ok_or("attestation_extension")
}

/// The leaf's key as an uncompressed SEC1 P-256 point.
fn p256_public_key(leaf: &X509Certificate<'_>) -> Result<Vec<u8>, &'static str> {
    let spki = leaf.public_key();
    let curve = spki
        .algorithm
        .parameters
        .as_ref()
        .and_then(|parameters| parameters.as_oid().ok())
        .map(|oid| oid.to_id_string());
    if spki.algorithm.algorithm.to_id_string() != EC_PUBLIC_KEY_OID
        || curve.as_deref() != Some(PRIME256V1_OID)
        || spki.subject_public_key.unused_bits != 0
    {
        return Err("leaf_key_not_p256");
    }
    let key = p256::PublicKey::from_sec1_bytes(spki.subject_public_key.data.as_ref())
        .map_err(|_| "leaf_key_not_p256")?;
    Ok(key.to_sec1_point(false).as_bytes().to_vec())
}

impl AndroidAttestationVerifier for KeyAttestationVerifier {
    fn verify(
        &self,
        chain: &[Vec<u8>],
        client_data: &[u8],
    ) -> Result<VerifiedAndroidKey, AndroidAttestationError> {
        self.check(chain, client_data).map_err(|reason| {
            tracing::warn!(
                operation = "attestation",
                platform = "android",
                reason,
                "Android Key Attestation rejected"
            );
            AndroidAttestationError::Invalid
        })
    }
}

#[cfg(test)]
#[path = "key_attestation_tests.rs"]
mod tests;
