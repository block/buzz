use std::{error::Error, fmt};

#[derive(Debug, PartialEq)]
pub enum AppAttestError {
    /// Certificate structure or trust validation failed.
    InvalidCertificate,
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
            Self::InvalidCertificate => write!(f, "invalid certificate path or structure"),
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
