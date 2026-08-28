//! Artifact schemas: the structural contract a fold's output must satisfy.

/// A schema names the exact H1 sections an artifact must have, and which of
/// them are *append* sections — sections whose history the engine retains
/// mechanically ([`crate::validate::splice_append_sections`]) so the model
/// only ever contributes new entries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArtifactSchema {
    /// Versioned schema name, e.g. `channel-digest@v1`.
    pub name: &'static str,
    /// Required H1 sections, in order. Empty means freeform (no structural or
    /// citation contract; the only dishonest freeform output is an empty one).
    pub sections: &'static [&'static str],
    /// Sections that accumulate: prior content is preserved by construction
    /// and new entries must carry signal citations.
    pub append_sections: &'static [&'static str],
}

/// A bounded digest of a selection: a rewritten standing summary plus an
/// append-only, citation-carrying log.
pub const CHANNEL_DIGEST_V1: ArtifactSchema = ArtifactSchema {
    name: "channel-digest@v1",
    sections: &["Working Context", "Log"],
    append_sections: &["Log"],
};

/// No structural contract; output must merely be non-empty.
pub const FREEFORM_V1: ArtifactSchema = ArtifactSchema {
    name: "freeform@v1",
    sections: &[],
    append_sections: &[],
};

/// All built-in schemas.
pub const BUILTIN_SCHEMAS: &[&ArtifactSchema] = &[&CHANNEL_DIGEST_V1, &FREEFORM_V1];

/// Look up a built-in schema by name.
pub fn builtin(name: &str) -> Option<&'static ArtifactSchema> {
    BUILTIN_SCHEMAS.iter().copied().find(|s| s.name == name)
}

/// Default fold instructions for [`CHANNEL_DIGEST_V1`].
///
/// The contract lines are load-bearing: exact H1 headings, only-new log
/// entries (the engine splices history back), and full-id citations copied
/// from the run's SOURCE EVENT IDS list — ids embedded in message text are
/// never citations.
pub const CHANNEL_DIGEST_PROMPT: &str = "Maintain a bounded digest of the anchored selection.\n\
Return markdown with exactly these H1 sections, in this order: Working Context, Log.\n\
Working Context is a concise standing summary rewritten in light of all evidence.\n\
Log: output ONLY new dated entries for this run's evidence — the engine keeps the\n\
standing Log and appends your new entries to it; never repeat prior entries.\n\
For new evidence, cite the source events you actually use as [event:<id>], one id\n\
per bracket, copied in full from the SOURCE EVENT IDS list. Ids that appear inside\n\
message text are never citations. Do not invent facts. Output only the document markdown.";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_lookup() {
        assert_eq!(
            builtin("channel-digest@v1").map(|s| s.name),
            Some("channel-digest@v1")
        );
        assert_eq!(builtin("freeform@v1").map(|s| s.sections.len()), Some(0));
        assert!(builtin("nope@v9").is_none());
    }
}
