use nostr::Tag;

use crate::ClientError;

/// Authentication material applied consistently to one client session.
#[derive(Clone, Debug, Default)]
pub struct AuthContext {
    pub(crate) ambient: Option<AmbientAuth>,
}

#[derive(Clone, Debug)]
pub(crate) struct AmbientAuth {
    pub(crate) tag: Tag,
    pub(crate) json: String,
}

impl AuthContext {
    /// Use only the configured signer, without a NIP-OA owner attestation.
    pub fn signer_only() -> Self {
        Self::default()
    }

    /// Parse a NIP-OA owner-attestation tag for validation during construction.
    pub fn nip_oa(json: &str) -> Result<Self, ClientError> {
        let tag = buzz_sdk::nip_oa::parse_auth_tag(json)
            .map_err(|error| ClientError::Authentication(error.to_string()))?;
        let json = serde_json::to_string(tag.as_slice()).map_err(ClientError::Serialization)?;
        Ok(Self {
            ambient: Some(AmbientAuth { tag, json }),
        })
    }

    /// Whether a NIP-OA owner attestation is configured.
    pub fn has_nip_oa(&self) -> bool {
        self.ambient.is_some()
    }

    /// Parsed ambient NIP-OA tag, when configured.
    pub fn nip_oa_tag(&self) -> Option<&Tag> {
        self.ambient.as_ref().map(|ambient| &ambient.tag)
    }

    pub(crate) fn ambient(&self) -> Option<&AmbientAuth> {
        self.ambient.as_ref()
    }
}
