//! The catalog-provenance coordinate carried on a copied persona
//! definition, split from `types.rs` (file-size cap).

use serde::{Deserialize, Serialize};

/// Where a persona copy came from in another owner's shared catalog.
///
/// The pair is the publication's NIP-AP coordinate minus the kind: the owner
/// who published it and the `d`-tag identifying the persona within that
/// owner's catalog. A copy carries a fresh local `id`, so this pair is the
/// only thing that can answer "is this catalog entry already added".
///
/// Field casing follows [`super::RelayMeshConfig`]: persisted records use snake_case
/// and the camelCase `alias`es accept the create payload the frontend sends
/// (`rename_all` on the request does not recurse into nested structs).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CatalogSource {
    #[serde(alias = "ownerPubkey")]
    pub owner_pubkey: String,
    #[serde(alias = "personaId")]
    pub persona_id: String,
}

impl CatalogSource {
    /// Normalize a coordinate arriving from the frontend.
    ///
    /// "Already added" is decided by comparing this pair against a
    /// publication's author and `d`-tag, so an un-normalized value silently
    /// fails to match and mints another copy — the exact duplicate the field
    /// exists to prevent. Owner pubkey: 64 hex, any case in, lowercase out
    /// (same contract as [`super::validate_respond_to_allowlist`]). Persona id: the
    /// publication's `d`-tag, trimmed and required.
    pub fn normalized(self) -> Result<Self, String> {
        let owner_pubkey = self.owner_pubkey.trim().to_ascii_lowercase();
        if owner_pubkey.len() != 64 || !owner_pubkey.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(format!(
                "invalid catalog source owner pubkey: '{owner_pubkey}' (must be 64 hex chars)"
            ));
        }
        let persona_id = self.persona_id.trim().to_string();
        if persona_id.is_empty() {
            return Err("catalog source persona id is required".to_string());
        }
        Ok(Self {
            owner_pubkey,
            persona_id,
        })
    }
}

#[cfg(test)]
mod tests;
