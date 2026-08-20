use super::{EffectiveAgentEnv, Requirement};

pub(super) fn require_safe_official_origin(
    effective: &EffectiveAgentEnv,
    missing: &mut Vec<Requirement>,
) {
    let safe = effective
        .env
        .get("OPENAI_COMPAT_BASE_URL")
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .is_none_or(|value| value.trim_end_matches('/') == "https://api.openai.com/v1");
    if !safe {
        missing.push(Requirement::ConfigInvalid {
            message: "remove `OPENAI_COMPAT_BASE_URL` or switch the provider to `openai-compat`"
                .to_string(),
        });
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    #[test]
    fn custom_origin_is_unsafe_for_official_openai() {
        let effective = EffectiveAgentEnv {
            env: BTreeMap::from([(
                "OPENAI_COMPAT_BASE_URL".to_string(),
                "https://gateway.example/v1".to_string(),
            )]),
            config_file_path: None,
            effective_command: "buzz-agent".to_string(),
        };
        let mut missing = Vec::new();

        require_safe_official_origin(&effective, &mut missing);

        assert_eq!(
            missing,
            vec![Requirement::ConfigInvalid {
                message:
                    "remove `OPENAI_COMPAT_BASE_URL` or switch the provider to `openai-compat`"
                        .to_string()
            }]
        );
    }
}
