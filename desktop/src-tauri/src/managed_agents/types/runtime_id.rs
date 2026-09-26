use serde::{Deserialize, Deserializer};

/// Read the unpublished pilot identity as the canonical Goose runtime.
pub(crate) fn deserialize_runtime_id<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    Ok(Option::<String>::deserialize(deserializer)?.map(|id| {
        if id == "goose-bundled" {
            "goose".to_string()
        } else {
            id
        }
    }))
}
