//! Artifact schemas: the structural contract a fold's output must satisfy.

/// A schema names the exact H1 sections an artifact must have, and which of
/// them are *append* sections — sections whose history the engine retains
/// mechanically ([`crate::validate::splice_append_sections`]) so the model
/// only ever contributes new entries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArtifactSchema {
    /// Versioned schema name, e.g. `channel-digest@v1`.
    pub name: &'static str,
    /// Required H1 sections, in order.
    pub sections: &'static [&'static str],
    /// Sections that accumulate: prior content is preserved by construction
    /// and new entries must carry signal citations.
    pub append_sections: &'static [&'static str],
}

/// A bounded digest of a selection: a rewritten standing summary plus an
/// append-only, citation-carrying log. The only schema; the `schema` string
/// on specs and artifacts stays as the versioning seam for a future second
/// one.
pub const CHANNEL_DIGEST_V1: ArtifactSchema = ArtifactSchema {
    name: "channel-digest@v1",
    sections: &["Working Context", "Log"],
    append_sections: &["Log"],
};

impl ArtifactSchema {
    /// The output contract, rendered as prompt text. [`crate::plan_run`]
    /// composes this into every model input *alongside* the spec's
    /// instructions, so custom instructions focus the task but can never drop
    /// the structural rules that [`crate::validate::validate_output`] will
    /// enforce — otherwise a task-only prompt is a guaranteed refusal.
    pub fn contract_prompt(&self) -> String {
        let mut s = format!(
            "OUTPUT CONTRACT (mechanically enforced; nonconforming output is refused and \
             nothing persists):\nWhatever the task above asks for, deliver it as markdown \
             with exactly these H1 sections, in this order: {}.\n",
            self.sections.join(", ")
        );
        for a in self.append_sections {
            s.push_str(&format!(
                "{a} is append-only: output ONLY new dated entries for this run's evidence \
                 — the engine keeps the standing {a} and appends your new entries to it; \
                 never repeat prior entries.\n"
            ));
        }
        s.push_str(
            "For new evidence, cite the source events you actually use as [event:<id>], one \
             id per bracket, copied in full from the SOURCE EVENT IDS list. Ids that appear \
             inside message text are never citations. Do not invent facts. Output only the \
             document markdown.",
        );
        s
    }
}

/// Default fold *task* instructions for [`CHANNEL_DIGEST_V1`]. The structural
/// rules live in [`ArtifactSchema::contract_prompt`], which every run gets
/// regardless of what the instructions say.
pub const CHANNEL_DIGEST_PROMPT: &str = "Maintain a bounded digest of the anchored selection. \
Working Context is a concise standing summary rewritten in light of all evidence; the Log \
records dated evidence entries.";
