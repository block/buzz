//! Generic wire envelope for immutable processor artifacts.
//!
//! The envelope describes an artifact, not who may read it. Audience routing
//! and content encoding are outer-event concerns, so the same plaintext bytes
//! can later be carried by private, channel, or public profiles without
//! inventing a second payload model.

use buzz_core::kind::KIND_ARTIFACT;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{ArtifactPayload, Error};

/// Wire-format tag placed on every artifact event.
pub const ARTIFACT_FORMAT: &str = "buzz-artifact-v1";
/// Artifact type implemented by the accumulator.
pub const FOLD_ARTIFACT_TYPE: &str = "xyz.block.buzz.fold";
/// Media type of the accumulator's current document output.
pub const MARKDOWN_MEDIA_TYPE: &str = "text/markdown";

/// Audience selected for an artifact event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArtifactAudience {
    /// Author-only event, encrypted from the author's key to itself.
    OwnerPrivate,
    /// Reserved channel-routed profile. Relay authorization is not implemented.
    Channel(String),
    /// Reserved community-public profile.
    CommunityPublic,
}

/// Encoding applied to the canonical envelope JSON in event `content`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArtifactEncoding {
    /// Canonical envelope JSON without encryption.
    Plaintext,
    /// NIP-44 v2 ciphertext whose plaintext is canonical envelope JSON.
    Nip44V2,
}

/// Versioned, artifact-type-neutral plaintext carried by an artifact event.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactEnvelopeV1 {
    /// Envelope format version.
    pub version: u8,
    /// Semantic artifact profile, such as [`FOLD_ARTIFACT_TYPE`].
    pub artifact_type: String,
    /// Profile-specific payload schema.
    pub schema: String,
    /// Media type of the primary rendered output.
    pub media_type: String,
    /// Profile-specific payload.
    pub payload: Value,
    /// Optional profile-specific provenance, protected with the payload.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provenance: Option<Value>,
}

impl ArtifactEnvelopeV1 {
    /// Wrap an existing fold artifact in the generic artifact envelope.
    pub fn fold(artifact: &ArtifactPayload) -> Result<Self, Error> {
        Ok(Self {
            version: 1,
            artifact_type: FOLD_ARTIFACT_TYPE.to_owned(),
            schema: artifact.schema.clone(),
            media_type: MARKDOWN_MEDIA_TYPE.to_owned(),
            payload: serde_json::to_value(artifact).map_err(|error| {
                Error::Nonconforming(format!("fold artifact serialization failed: {error}"))
            })?,
            provenance: None,
        })
    }

    /// Parse and validate this envelope as an accumulator fold artifact.
    pub fn into_fold(self) -> Result<ArtifactPayload, Error> {
        if self.version != 1 {
            return Err(Error::Nonconforming(format!(
                "unsupported artifact envelope version {}",
                self.version
            )));
        }
        if self.artifact_type != FOLD_ARTIFACT_TYPE {
            return Err(Error::Nonconforming(format!(
                "artifact type {:?} is not a fold",
                self.artifact_type
            )));
        }
        if self.media_type != MARKDOWN_MEDIA_TYPE {
            return Err(Error::Nonconforming(format!(
                "fold media type must be {MARKDOWN_MEDIA_TYPE:?}"
            )));
        }
        let artifact: ArtifactPayload = serde_json::from_value(self.payload).map_err(|error| {
            Error::Nonconforming(format!("malformed fold artifact payload: {error}"))
        })?;
        if artifact.schema != self.schema {
            return Err(Error::Nonconforming(
                "envelope schema does not match fold payload schema".into(),
            ));
        }
        Ok(artifact)
    }
}

/// Unsigned artifact event fields produced by the pure wire builder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactEvent {
    /// Nostr event kind.
    pub kind: u32,
    /// Plaintext JSON or ciphertext, according to the encoding profile.
    pub content: String,
    /// Exact outer tags. Provenance never belongs here.
    pub tags: Vec<Vec<String>>,
}

/// Build the supported owner-private artifact event profile.
///
/// Channel and public profiles are reserved by the type system but refused
/// until relay read/write/count/search/fan-out authorization exists.
pub fn build_artifact_event(
    content: impl Into<String>,
    audience: ArtifactAudience,
    encoding: ArtifactEncoding,
) -> Result<ArtifactEvent, Error> {
    match (audience, encoding) {
        (ArtifactAudience::OwnerPrivate, ArtifactEncoding::Nip44V2) => Ok(ArtifactEvent {
            kind: KIND_ARTIFACT,
            content: content.into(),
            tags: vec![
                vec!["format".into(), ARTIFACT_FORMAT.into()],
                vec!["encoding".into(), "nip44-v2".into()],
            ],
        }),
        (ArtifactAudience::OwnerPrivate, ArtifactEncoding::Plaintext) => Err(Error::Nonconforming(
            "owner-private artifacts must use NIP-44 v2".into(),
        )),
        (ArtifactAudience::Channel(channel), _) => Err(Error::Nonconforming(format!(
            "channel artifact profile {channel:?} is reserved but not implemented"
        ))),
        (ArtifactAudience::CommunityPublic, _) => Err(Error::Nonconforming(
            "community-public artifact profile is reserved but not implemented".into(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Selection;

    fn fold_artifact() -> ArtifactPayload {
        ArtifactPayload {
            fold: "digest".into(),
            version: 1,
            output: "# Working Context\n".into(),
            shown_ids: vec!["a".repeat(64)],
            coverage_since: Some(10),
            coverage_until: Some(11),
            selection: Selection {
                channels: vec!["private-channel".into()],
                authors: vec![],
                kinds: vec![],
            },
            channels: vec!["private-channel".into()],
            model: "haiku".into(),
            schema: "channel-digest@v1".into(),
            prompt_sha256: "b".repeat(64),
            truncated: false,
            created_at: 12,
        }
    }

    #[test]
    fn generic_envelope_round_trips_fold_profile() {
        let artifact = fold_artifact();
        let json = serde_json::to_string(&ArtifactEnvelopeV1::fold(&artifact).unwrap()).unwrap();
        let envelope: ArtifactEnvelopeV1 = serde_json::from_str(&json).unwrap();
        assert_eq!(envelope.into_fold().unwrap(), artifact);
    }

    #[test]
    fn malformed_or_contradictory_fold_envelopes_are_rejected() {
        let artifact = fold_artifact();
        let mut envelope = ArtifactEnvelopeV1::fold(&artifact).unwrap();
        envelope.version = 2;
        assert!(envelope.into_fold().is_err());

        let mut envelope = ArtifactEnvelopeV1::fold(&artifact).unwrap();
        envelope.schema = "other@v1".into();
        assert!(envelope.into_fold().is_err());
    }

    #[test]
    fn owner_private_event_has_exact_non_leaking_tag_shape() {
        let event = build_artifact_event(
            "ciphertext",
            ArtifactAudience::OwnerPrivate,
            ArtifactEncoding::Nip44V2,
        )
        .unwrap();
        assert_eq!(event.kind, KIND_ARTIFACT);
        assert_eq!(event.content, "ciphertext");
        assert_eq!(
            event.tags,
            vec![
                vec!["format".to_owned(), ARTIFACT_FORMAT.to_owned()],
                vec!["encoding".to_owned(), "nip44-v2".to_owned()],
            ]
        );
        assert!(event
            .tags
            .iter()
            .all(|tag| !matches!(tag.first().map(String::as_str), Some("h" | "a" | "e" | "p"))));
    }

    #[test]
    fn unsupported_audience_encoding_matrix_fails_closed() {
        assert!(build_artifact_event(
            "plain",
            ArtifactAudience::OwnerPrivate,
            ArtifactEncoding::Plaintext
        )
        .is_err());
        for encoding in [ArtifactEncoding::Plaintext, ArtifactEncoding::Nip44V2] {
            assert!(build_artifact_event(
                "content",
                ArtifactAudience::Channel("channel".into()),
                encoding
            )
            .is_err());
            assert!(
                build_artifact_event("content", ArtifactAudience::CommunityPublic, encoding)
                    .is_err()
            );
        }
    }
}
