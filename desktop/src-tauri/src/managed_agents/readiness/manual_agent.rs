use super::{EffectiveAgentEnv, Requirement};

pub(super) fn requirements(effective: &EffectiveAgentEnv) -> Vec<Requirement> {
    let mut missing = Vec::new();
    let base_url_valid = effective
        .env
        .get("MANUAL_AGENT_BASE_URL")
        .map(String::as_str)
        .is_some_and(|raw| {
            let Ok(url) = url::Url::parse(raw.trim()) else {
                return false;
            };
            url.scheme() == "https"
                && url.host_str().is_some_and(is_tailnet_hostname)
                && url.username().is_empty()
                && url.password().is_none()
                && url.query().is_none()
                && url.fragment().is_none()
                && matches!(url.path(), "" | "/")
        });
    if !base_url_valid {
        missing.push(Requirement::EnvKey {
            key: "MANUAL_AGENT_BASE_URL".to_string(),
        });
    }

    let token_valid = effective
        .env
        .get("MANUAL_AGENT_TOKEN")
        .is_some_and(|token| token.trim().len() >= 32);
    if !token_valid {
        missing.push(Requirement::EnvKey {
            key: "MANUAL_AGENT_TOKEN".to_string(),
        });
    }
    missing
}

fn is_tailnet_hostname(host: &str) -> bool {
    host.to_ascii_lowercase()
        .strip_suffix(".ts.net")
        .is_some_and(|prefix| !prefix.is_empty() && !prefix.ends_with('.'))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    fn effective(pairs: &[(&str, &str)]) -> EffectiveAgentEnv {
        EffectiveAgentEnv {
            env: pairs
                .iter()
                .map(|(key, value)| (key.to_string(), value.to_string()))
                .collect::<BTreeMap<_, _>>(),
            runtime_id: None,
            effective_args: Vec::new(),
            config_file_path: None,
            effective_command: "buzz-manual-agent-acp".to_string(),
        }
    }

    #[test]
    fn requires_tailnet_https_url_and_app_password() {
        let missing = requirements(&effective(&[]));
        assert_eq!(missing.len(), 2);

        let invalid = requirements(&effective(&[
            ("MANUAL_AGENT_BASE_URL", "https://example.com"),
            ("MANUAL_AGENT_TOKEN", "short"),
        ]));
        assert_eq!(invalid.len(), 2);

        let ready = requirements(&effective(&[
            (
                "MANUAL_AGENT_BASE_URL",
                "https://device.example-tailnet.ts.net:8787",
            ),
            ("MANUAL_AGENT_TOKEN", "test-app-password-0123456789-abcdef"),
        ]));
        assert!(ready.is_empty());
    }
}
