use std::{error::Error, fmt};

/// The certificate check that rejected an attestation, so operators can tell
/// an untrusted path from a malformed or misused certificate.
#[derive(Debug, PartialEq)]
pub enum CertificateCheck {
    /// `x5c` is not an array, or is empty. A missing `x5c` fails the CBOR
    /// schema first, as `InvalidCbor`.
    Chain,
    /// The configured Apple root is not exactly one PEM certificate.
    Root,
    /// The certificate at this `x5c` index is not exactly one DER X.509
    /// certificate with unique extensions.
    Encoding { index: usize },
    /// webpki rejected the path; holds its error.
    Path(String),
    /// The certificate at this `x5c` index lacks the key usage its position requires.
    KeyUsage { index: usize },
}

impl fmt::Display for CertificateCheck {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Chain => write!(f, "x5c is not a non-empty array"),
            Self::Root => write!(f, "invalid configured root"),
            Self::Encoding { index } => write!(f, "invalid encoding at x5c[{index}]"),
            Self::Path(error) => write!(f, "path rejected: {error}"),
            Self::KeyUsage { index } => write!(f, "invalid key usage at x5c[{index}]"),
        }
    }
}

#[derive(Debug, PartialEq)]
pub enum AppAttestError {
    /// Certificate structure or trust validation failed.
    InvalidCertificate(CertificateCheck),
    /// The proof is not valid standard base64.
    InvalidBase64,
    /// CBOR is malformed, ambiguous, or does not match the operation schema.
    InvalidCbor,
    /// An operational parsing budget was exceeded.
    ResourceLimit,
    InvalidNonce,
    InvalidPublicKey,
    InvalidCounter,
    InvalidCredentialID,
    InvalidAAGUID,
    InvalidSignature,
    InvalidAppID,
    InvalidFormat,
    AuthenticatorDataTooShort,
}

impl fmt::Display for AppAttestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidCertificate(check) => {
                write!(f, "invalid certificate path or structure ({check})")
            }
            Self::InvalidBase64 => write!(f, "invalid base64 encoding"),
            Self::InvalidCbor => write!(f, "invalid CBOR structure"),
            Self::ResourceLimit => write!(f, "verification resource limit"),
            AppAttestError::InvalidNonce => write!(f, "invalid nonce"),
            AppAttestError::InvalidPublicKey => write!(f, "invalid public key"),
            AppAttestError::InvalidCounter => write!(f, "invalid counter"),
            AppAttestError::InvalidCredentialID => write!(f, "invalid credential ID"),
            AppAttestError::InvalidAAGUID => write!(f, "invalid AAGUID"),
            AppAttestError::InvalidSignature => write!(f, "invalid signature"),
            AppAttestError::InvalidAppID => write!(f, "invalid App ID"),
            AppAttestError::InvalidFormat => {
                write!(f, "invalid attestation format (expected apple-appattest)")
            }
            AppAttestError::AuthenticatorDataTooShort => {
                write!(f, "authenticator data is too short")
            }
        }
    }
}

impl Error for AppAttestError {}

impl From<minicbor::decode::Error> for AppAttestError {
    fn from(_: minicbor::decode::Error) -> Self {
        Self::InvalidCbor
    }
}
