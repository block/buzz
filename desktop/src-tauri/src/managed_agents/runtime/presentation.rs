use crate::managed_agents::known_acp_runtime;

pub(super) struct RuntimePresentation {
    pub(super) icon_url: Option<String>,
    pub(super) avatar_url: Option<String>,
    pub(super) superseded_avatar_urls: Vec<String>,
    pub(super) supports_buzz_model_config: Option<bool>,
}

pub(super) fn runtime_presentation_for_summary(effective_command: &str) -> RuntimePresentation {
    let runtime = known_acp_runtime(effective_command);
    RuntimePresentation {
        icon_url: runtime.map(|runtime| runtime.icon_url.to_string()),
        avatar_url: runtime.map(|runtime| runtime.avatar_url.to_string()),
        superseded_avatar_urls: runtime
            .map(|runtime| {
                runtime
                    .superseded_avatar_urls
                    .iter()
                    .map(|url| (*url).to_string())
                    .collect()
            })
            .unwrap_or_default(),
        supports_buzz_model_config: runtime
            .map(|runtime| runtime.model_env_var.is_some() || runtime.supports_acp_model_switching),
    }
}

#[cfg(test)]
mod tests {
    use super::runtime_presentation_for_summary;
    use crate::managed_agents::known_acp_runtime;

    #[test]
    fn runtime_avatar_is_catalog_derived_without_process_state() {
        let runtime = known_acp_runtime("devin").expect("Devin must remain a known runtime");
        let presentation = runtime_presentation_for_summary("devin");

        assert_eq!(presentation.icon_url.as_deref(), Some(runtime.icon_url));
        assert_eq!(presentation.avatar_url.as_deref(), Some(runtime.avatar_url));
        assert_eq!(
            presentation.superseded_avatar_urls,
            runtime.superseded_avatar_urls
        );
        assert_eq!(presentation.supports_buzz_model_config, Some(false));

        let custom = runtime_presentation_for_summary("custom-agent");
        assert!(custom.icon_url.is_none());
        assert!(custom.avatar_url.is_none());
        assert!(custom.superseded_avatar_urls.is_empty());
        assert_eq!(custom.supports_buzz_model_config, None);
    }
}
