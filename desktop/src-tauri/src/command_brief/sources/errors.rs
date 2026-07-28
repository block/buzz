/// Stable, redacted failures which the orchestrator may expose as run status.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum SourceCollectionError {
    Cancelled,
    InvalidRequest,
    InvalidTime,
    RagUnavailable,
    RagStale,
    RagInvalid,
    SnapshotChanged,
    ConflictingSourceIdentity,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct SourceReadError {
    code: &'static str,
}

impl SourceReadError {
    pub(crate) const fn new(code: &'static str) -> Self {
        Self { code }
    }

    pub(super) const fn code(self) -> &'static str {
        self.code
    }
}
