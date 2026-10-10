//! Activation is an operator cutover, not a rolling feature flag.

use super::{parse_bool, ConfigError};

pub(super) fn enabled_from_env(stable_key: bool) -> Result<bool, ConfigError> {
    let enabled = parse_bool("BUZZ_NIP_CL_ENABLED", false)?;
    let cutover = std::env::var("BUZZ_NIP_CL_WRITER_CUTOVER").ok();
    validate_activation(enabled, cutover.as_deref(), stable_key)
}

fn validate_activation(
    enabled: bool,
    cutover: Option<&str>,
    stable_key: bool,
) -> Result<bool, ConfigError> {
    if enabled && (!stable_key || cutover != Some("offline-v1")) {
        return Err(ConfigError::InvalidValue(
            "BUZZ_NIP_CL_ENABLED requires BUZZ_RELAY_PRIVATE_KEY and \
             BUZZ_NIP_CL_WRITER_CUTOVER=offline-v1 after completing \
             docs/channel-labels-rollout.md; the declaration does not fence old writers"
                .into(),
        ));
    }
    Ok(enabled)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn activation_requires_both_stable_identity_and_explicit_cutover() {
        for enabled in [false, true] {
            for stable_key in [false, true] {
                for cutover in [None, Some(""), Some("true"), Some("offline-v1")] {
                    let result = validate_activation(enabled, cutover, stable_key);
                    if enabled && !(stable_key && cutover == Some("offline-v1")) {
                        assert!(result.is_err());
                    } else {
                        assert_eq!(result.unwrap(), enabled);
                    }
                }
            }
        }
    }
}
