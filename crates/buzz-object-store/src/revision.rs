//! Provider-safe revision tokens and conditional-write vocabulary.
//!
//! A revision is whatever the backing provider uses to name "the version of
//! this object I just observed": an ETag on S3, an object generation on Google
//! Cloud Storage. The two are not interchangeable, and the type keeps them
//! from being confused — a generation handed to an S3 `If-Match` would be a
//! silent correctness bug, so [`Revision::expect_s3_etag`] rejects it instead.

use crate::error::ObjectStoreError;

/// Which provider minted a [`Revision`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderKind {
    /// S3 and S3-compatible backends (AWS S3, MinIO, Railway).
    S3,
    /// Google Cloud Storage, using native object generations.
    Gcs,
}

impl std::fmt::Display for ProviderKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::S3 => f.write_str("s3"),
            Self::Gcs => f.write_str("gcs"),
        }
    }
}

/// An opaque, provider-qualified object revision.
///
/// Revisions are produced by reads and successful conditional writes, and are
/// consumed by the next conditional write. They are never parsed, compared
/// across providers, or synthesised by callers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Revision {
    /// S3 entity tag, verbatim as the provider returned it (quotes included).
    S3Etag(String),
    /// Google Cloud Storage object generation.
    GcsGeneration(i64),
}

impl Revision {
    /// Which provider this revision belongs to.
    pub fn provider(&self) -> ProviderKind {
        match self {
            Self::S3Etag(_) => ProviderKind::S3,
            Self::GcsGeneration(_) => ProviderKind::Gcs,
        }
    }

    /// Borrow the S3 entity tag, rejecting a revision from another provider.
    ///
    /// This is the guard that keeps a provider swap from degrading into a
    /// blind overwrite: a caller holding a GCS generation cannot accidentally
    /// predicate an S3 `If-Match` on it.
    pub fn expect_s3_etag(&self) -> Result<&str, ObjectStoreError> {
        match self {
            Self::S3Etag(tag) => Ok(tag),
            other => Err(ObjectStoreError::RevisionMismatch {
                expected: ProviderKind::S3,
                actual: other.provider(),
            }),
        }
    }

    /// Read the GCS object generation, rejecting a revision from another provider.
    pub fn expect_gcs_generation(&self) -> Result<i64, ObjectStoreError> {
        match self {
            Self::GcsGeneration(generation) => Ok(*generation),
            other => Err(ObjectStoreError::RevisionMismatch {
                expected: ProviderKind::Gcs,
                actual: other.provider(),
            }),
        }
    }
}

/// Precondition for a conditional write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WriteCondition {
    /// Create-only: commit iff the object does not yet exist.
    Absent,
    /// Compare-and-swap: commit iff the object is still at this revision.
    Matches(Revision),
}

/// Outcome of a conditional write.
///
/// `Conflict` is *not* an error — it is the ordinary result of losing a
/// compare-and-swap race, and callers must classify it as such (retry, or
/// report a non-fast-forward) rather than as a backend failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConditionalWrite {
    /// The write committed; the new revision predicates the next write.
    Committed(Revision),
    /// The precondition did not hold; nothing was written.
    Conflict,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn revision_reports_its_provider() {
        assert_eq!(
            Revision::S3Etag("\"abc\"".into()).provider(),
            ProviderKind::S3
        );
        assert_eq!(Revision::GcsGeneration(7).provider(), ProviderKind::Gcs);
    }

    #[test]
    fn s3_accessor_accepts_an_s3_revision() {
        let revision = Revision::S3Etag("\"abc\"".into());
        assert_eq!(revision.expect_s3_etag().unwrap(), "\"abc\"");
    }

    #[test]
    fn s3_accessor_rejects_a_gcs_revision() {
        let err = Revision::GcsGeneration(7).expect_s3_etag().unwrap_err();
        assert!(matches!(
            err,
            ObjectStoreError::RevisionMismatch {
                expected: ProviderKind::S3,
                actual: ProviderKind::Gcs,
            }
        ));
    }

    #[test]
    fn gcs_accessor_rejects_an_s3_revision() {
        let err = Revision::S3Etag("\"abc\"".into())
            .expect_gcs_generation()
            .unwrap_err();
        assert!(matches!(
            err,
            ObjectStoreError::RevisionMismatch {
                expected: ProviderKind::Gcs,
                actual: ProviderKind::S3,
            }
        ));
    }

    #[test]
    fn provider_kind_renders_a_stable_label() {
        assert_eq!(ProviderKind::S3.to_string(), "s3");
        assert_eq!(ProviderKind::Gcs.to_string(), "gcs");
    }
}
