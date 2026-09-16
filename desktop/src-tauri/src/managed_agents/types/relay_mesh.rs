use serde::{Deserialize, Serialize};

/// Typed relay-mesh configuration carried on a [`super::ManagedAgentRecord`].
///
/// Feature-independent on purpose: the field is always present in the record
/// schema so saved agents round-trip identically whether or not the `mesh-llm`
/// feature is compiled in.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RelayMeshConfig {
    /// The served model id this agent routes to (e.g. "Qwen3").
    ///
    /// `alias` because this struct crosses two boundaries with different
    /// casing conventions: the TS create request sends camelCase
    /// (`relayMesh: { modelRef }` — `rename_all` on the request does not
    /// recurse into nested structs), while persisted records use snake_case.
    /// Serialization stays `model_ref` so saved records are stable.
    #[serde(alias = "modelRef")]
    pub model_ref: String,
}
