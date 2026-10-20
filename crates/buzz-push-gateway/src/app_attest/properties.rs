//! Authenticated optional Apple properties. These never authorize a request.
use super::{cbor::Value, error::AppAttestError};

/// Bounded observations, without coercing unsupported optional representations.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClientProperties {
    /// Text version when representable within the diagnostic length budget.
    pub bundle_version: Option<String>,
    /// Raw four-byte category. The profile does not assume undocumented endian.
    pub validation_category: Option<[u8; 4]>,
    /// A known property was present with an unsupported type or diagnostic size.
    pub opaque_known_property: bool,
    /// Unknown identifiers ignored after structural and cryptographic checks.
    pub unknown_extensions: usize,
}
impl ClientProperties {
    pub(super) fn from_extensions(value: &Value) -> Result<Self, AppAttestError> {
        value.text_keys()?;
        let mut out = Self::default();
        for (key, value) in value.map()? {
            match key {
                Value::Text(name) if name == "apple_bundle_version_01" => match value {
                    Value::Text(version) if version.len() <= 128 => {
                        out.bundle_version = Some(version.clone())
                    }
                    _ => out.opaque_known_property = true,
                },
                Value::Text(name) if name == "apple_validation_category_01" => match value {
                    Value::Bytes(category) if category.len() == 4 => {
                        out.validation_category = category.as_slice().try_into().ok()
                    }
                    _ => out.opaque_known_property = true,
                },
                _ => out.unknown_extensions += 1,
            }
        }
        Ok(out)
    }
}
