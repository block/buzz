//! Catalog provenance for a team copied from another owner's catalog, and for
//! each member within it. Split from `types.rs` (file-size cap), alongside
//! [`super::CatalogSource`].

use serde::{Deserialize, Serialize};

/// Normalize an owner pubkey arriving from outside the backend.
///
/// Shares [`super::CatalogSource::normalized`]'s contract: 64 hex, any case
/// in, lowercase out. An un-normalized value silently fails to match a
/// publication, re-enabling the duplicate add that provenance prevents.
fn normalized_owner_pubkey(value: &str) -> Result<String, String> {
    let owner_pubkey = value.trim().to_ascii_lowercase();
    if owner_pubkey.len() != 64 || !owner_pubkey.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(format!(
            "invalid catalog source owner pubkey: '{owner_pubkey}' (must be 64 hex chars)"
        ));
    }
    Ok(owner_pubkey)
}

/// Where a team copy came from in another owner's shared catalog.
///
/// Deliberately NOT [`super::CatalogSource`]: that type is the kind:30175
/// persona coordinate `(owner_pubkey, persona_id)`, and a 30178 team d-tag
/// resolved in the 30175 namespace addresses a different event. Reusing one
/// type for two kinds would let a team's provenance match a persona's.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TeamCatalogSource {
    #[serde(alias = "ownerPubkey")]
    pub owner_pubkey: String,
    /// The publication's `d`-tag — the team's id in the publisher's namespace.
    #[serde(alias = "teamDTag")]
    pub team_d_tag: String,
}

impl TeamCatalogSource {
    pub fn normalized(self) -> Result<Self, String> {
        let owner_pubkey = normalized_owner_pubkey(&self.owner_pubkey)?;
        let team_d_tag = self.team_d_tag.trim().to_string();
        if team_d_tag.is_empty() {
            return Err("catalog source team d-tag is required".to_string());
        }
        Ok(Self {
            owner_pubkey,
            team_d_tag,
        })
    }
}

/// Where a persona copy came from within a published team.
///
/// The full A1 provenance triple plus a version stamp:
/// `(owner_pubkey, team_d_tag)` says which publication, `member_key` which
/// member inside it, `projection_hash` which version. All four are required
/// for safe reuse — matching the triple alone would let two versions of one
/// published member share a mutable local definition, so an add of the newer
/// would silently rewrite the copy made from the older.
///
/// `member_key` is opaque, NOT a kind:30175 coordinate: the publisher may
/// never have shared that member individually, and its presence in a team
/// publication grants no read access to a persona coordinate.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TeamMemberCatalogSource {
    #[serde(alias = "ownerPubkey")]
    pub owner_pubkey: String,
    #[serde(alias = "teamDTag")]
    pub team_d_tag: String,
    #[serde(alias = "memberKey")]
    pub member_key: String,
    /// Hash of the member projection this copy was built from.
    #[serde(alias = "projectionHash")]
    pub projection_hash: String,
}

#[cfg(test)]
mod tests;
