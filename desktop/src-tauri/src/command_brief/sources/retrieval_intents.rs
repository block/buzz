use serde::Serialize;

use super::{truncate_utf8, AdviserId, RagSnapshotAssurance};
use crate::command_brief::personas::specialist_definitions;

const MAX_RETRIEVAL_QUERY_BYTES: usize = 2048;
const RAG_TOOL: &str = "search_knowledge_base";
const MEMORY_TOOL: &str = "command_memory_context";
const COLLECTION_SCOPE: &str = "verified_catalogue";
const OBSERVED_COLLECTION_SCOPE: &str = "observed_catalogue";
const DOCTRINE_COLLECTIONS: &[&str] = &["ADF Doctrine"];

/// One native-owned, bounded retrieval request for a fixed specialist.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FixedRetrievalIntent {
    adviser: AdviserId,
    rag_tool: &'static str,
    memory_tool: &'static str,
    collection_scope: &'static str,
    doctrine_query: String,
    context_query: String,
}

impl FixedRetrievalIntent {
    pub(crate) const fn adviser(&self) -> AdviserId {
        self.adviser
    }

    pub(crate) const fn rag_tool(&self) -> &'static str {
        self.rag_tool
    }

    pub(crate) const fn memory_tool(&self) -> &'static str {
        self.memory_tool
    }

    pub(crate) const fn collection_scope(&self) -> &'static str {
        self.collection_scope
    }

    pub(crate) fn query(&self) -> &str {
        &self.context_query
    }

    pub(crate) fn doctrine_query(&self) -> &str {
        &self.doctrine_query
    }

    pub(crate) fn context_query(&self) -> &str {
        &self.context_query
    }

    pub(crate) const fn doctrine_collections(&self) -> &'static [&'static str] {
        DOCTRINE_COLLECTIONS
    }
}

pub(super) fn fixed_retrieval_intents(
    co_request: &str,
    assurance: RagSnapshotAssurance,
) -> Vec<FixedRetrievalIntent> {
    let (source_description, collection_scope) = match assurance {
        RagSnapshotAssurance::SignedSnapshot => (
            "verified local catalogue and conflict-safe command memory",
            COLLECTION_SCOPE,
        ),
        RagSnapshotAssurance::TrustedLanObserved => (
            "approved trusted-LAN catalogue and observed command memory",
            OBSERVED_COLLECTION_SCOPE,
        ),
    };
    specialist_definitions()
        .iter()
        .map(|persona| {
            let context_prefix = format!(
                "{} Use only the {source_description}. CO request: ",
                persona.purpose,
            );
            let context_remaining =
                MAX_RETRIEVAL_QUERY_BYTES.saturating_sub(context_prefix.len());
            let (context_request, _) = truncate_utf8(co_request, context_remaining);
            let doctrine_prefix = format!(
                "{} Identify applicable ADF doctrine, planning guidance, responsibilities, constraints, and relevant process steps before forming advice. CO request: ",
                persona.purpose,
            );
            let doctrine_remaining =
                MAX_RETRIEVAL_QUERY_BYTES.saturating_sub(doctrine_prefix.len());
            let (doctrine_request, _) = truncate_utf8(co_request, doctrine_remaining);
            FixedRetrievalIntent {
                adviser: persona.adviser,
                rag_tool: RAG_TOOL,
                memory_tool: MEMORY_TOOL,
                collection_scope,
                doctrine_query: format!("{doctrine_prefix}{doctrine_request}"),
                context_query: format!("{context_prefix}{context_request}"),
            }
        })
        .collect()
}
