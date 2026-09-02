//! Provider construction for Buzz's supported provider set.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use goose_providers::anthropic::{AnthropicProviderBuilder, ANTHROPIC_API_VERSION};
use goose_providers::api_client::{ApiClient, AuthMethod};
use goose_providers::base::Provider;
use goose_providers::databricks::DatabricksProvider;
use goose_providers::databricks_auth::{
    DatabricksAuth, DatabricksOauthTokenProvider, DatabricksRefreshHook,
};
use goose_providers::databricks_v2::DatabricksV2Provider;
use goose_providers::ollama::{
    OllamaOptions, OllamaProviderBuilder, OLLAMA_DEFAULT_CHUNK_TIMEOUT_SECS, OLLAMA_DEFAULT_PORT,
    OLLAMA_PROVIDER_NAME,
};
use goose_providers::openai::{
    parse_custom_headers, parse_openai_base_url, OpenAiProviderBuilder, OPEN_AI_DEFAULT_BASE_PATH,
    OPEN_AI_VERSIONLESS_BASE_PATH,
};
use goose_providers::openrouter::OpenRouterProvider;

use crate::types::AgentError;

const DEFAULT_TIMEOUT_SECS: u64 = 600;

pub async fn build(provider_name: &str) -> Result<Arc<dyn Provider>, AgentError> {
    let provider = match provider_name {
        "anthropic" => anthropic(),
        "openai" | "openai-compat" | "openai_compat" | "relay-mesh" | "relay_mesh" => openai(),
        "openrouter" => openrouter(),
        "ollama" => ollama(),
        "databricks" => databricks(false),
        "databricks_v2" | "databricks-v2" => databricks(true),
        other => Err(anyhow::anyhow!(
            "unsupported Buzz provider {other:?}; supported providers: anthropic, openai, openai-compat, openrouter, ollama, databricks, databricks_v2"
        )),
    };
    provider.map_err(|error| crate::map_provider_error(&error.to_string()))
}

fn required_env(key: &str) -> anyhow::Result<String> {
    env(key).ok_or_else(|| anyhow::anyhow!("{key} is not set"))
}

fn env(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn env_parse<T: std::str::FromStr>(key: &str) -> Option<T> {
    env(key).and_then(|value| value.parse().ok())
}

fn anthropic() -> anyhow::Result<Arc<dyn Provider>> {
    let api_client = ApiClient::with_timeout_and_tls(
        env("ANTHROPIC_HOST").unwrap_or_else(|| "https://api.anthropic.com".to_string()),
        AuthMethod::ApiKey {
            header_name: "x-api-key".to_string(),
            key: required_env("ANTHROPIC_API_KEY")?,
        },
        Duration::from_secs(env_parse("ANTHROPIC_TIMEOUT").unwrap_or(DEFAULT_TIMEOUT_SECS)),
        None,
    )?
    .with_header("anthropic-version", ANTHROPIC_API_VERSION)?;
    Ok(Arc::new(AnthropicProviderBuilder::new(api_client).build()))
}

fn openai() -> anyhow::Result<Arc<dyn Provider>> {
    let raw_base = env("OPENAI_BASE_URL")
        .or_else(|| env("OPENAI_HOST"))
        .unwrap_or_else(|| "https://api.openai.com".to_string());
    let (host, query, has_v1) = parse_openai_base_url(&raw_base)?;
    let direct_openai = url::Url::parse(&host)
        .ok()
        .and_then(|url| url.host_str().map(str::to_owned))
        .is_some_and(|host| host.eq_ignore_ascii_case("api.openai.com"));
    let auth = env("OPENAI_API_KEY")
        .map(AuthMethod::BearerToken)
        .unwrap_or(AuthMethod::NoAuth);
    let mut api_client = ApiClient::with_timeout_and_tls(
        host,
        auth,
        Duration::from_secs(env_parse("OPENAI_TIMEOUT").unwrap_or(DEFAULT_TIMEOUT_SECS)),
        None,
    )?
    .with_query(query);
    let organization = env("OPENAI_ORGANIZATION");
    let project = env("OPENAI_PROJECT");
    if let Some(value) = &organization {
        api_client = api_client.with_header("OpenAI-Organization", value)?;
    }
    if let Some(value) = &project {
        api_client = api_client.with_header("OpenAI-Project", value)?;
    }
    let custom_headers = env("OPENAI_CUSTOM_HEADERS").map(parse_custom_headers);
    if let Some(headers) = &custom_headers {
        for (key, value) in headers {
            api_client = api_client.with_header(key, value)?;
        }
    }
    let base_path = env("OPENAI_BASE_PATH").unwrap_or_else(|| {
        if has_v1 {
            OPEN_AI_DEFAULT_BASE_PATH.to_string()
        } else {
            OPEN_AI_VERSIONLESS_BASE_PATH.to_string()
        }
    });
    Ok(Arc::new(
        OpenAiProviderBuilder::new(api_client)
            .base_path(base_path)
            .organization(organization)
            .project(project)
            .custom_headers(custom_headers)
            .preserve_thinking_context(!direct_openai)
            .build(),
    ))
}

fn openrouter() -> anyhow::Result<Arc<dyn Provider>> {
    let host = env("OPENROUTER_HOST").unwrap_or_else(|| "https://openrouter.ai".to_string());
    let api_client = ApiClient::new_with_tls(
        host,
        AuthMethod::BearerToken(required_env("OPENROUTER_API_KEY")?),
        None,
    )?
    .with_header("HTTP-Referer", "https://buzz.block.xyz")?
    .with_header("X-Title", "buzz-agent")?
    .with_header("X-OpenRouter-Categories", "cli-agent,productivity")?;
    let parameters = env("OPENROUTER_PARAMETERS")
        .map(|value| serde_json::from_str::<HashMap<String, serde_json::Value>>(&value))
        .transpose()?;
    Ok(Arc::new(OpenRouterProvider::new(
        api_client, parameters, None,
    )))
}

fn ollama() -> anyhow::Result<Arc<dyn Provider>> {
    let raw = env("OLLAMA_HOST").unwrap_or_else(|| "localhost".to_string());
    let with_scheme = if raw.contains("://") {
        raw
    } else {
        format!("http://{raw}")
    };
    let mut url = url::Url::parse(&with_scheme)?;
    if url.port().is_none() && matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "::1")) {
        url.set_port(Some(OLLAMA_DEFAULT_PORT))
            .map_err(|_| anyhow::anyhow!("invalid Ollama port"))?;
    }
    let timeout = env_parse("OLLAMA_TIMEOUT").unwrap_or(DEFAULT_TIMEOUT_SECS);
    let options = OllamaOptions {
        input_limit: env_parse::<usize>("GOOSE_INPUT_LIMIT").filter(|limit| *limit > 0),
        stream_usage: env("OLLAMA_STREAM_USAGE")
            .and_then(|value| value.parse().ok())
            .unwrap_or(true),
        chunk_timeout_secs: env_parse::<u64>("OLLAMA_STREAM_TIMEOUT")
            .or_else(|| env_parse("GOOSE_STREAM_TIMEOUT"))
            .or_else(|| env_parse("OLLAMA_TIMEOUT"))
            .filter(|timeout| *timeout > 0)
            .unwrap_or(OLLAMA_DEFAULT_CHUNK_TIMEOUT_SECS),
    };
    let api_client = ApiClient::with_timeout_and_tls(
        url.to_string(),
        AuthMethod::NoAuth,
        Duration::from_secs(timeout),
        None,
    )?;
    Ok(Arc::new(
        OllamaProviderBuilder::new(api_client)
            .name(OLLAMA_PROVIDER_NAME)
            .options(options)
            .build(),
    ))
}

fn databricks(v2: bool) -> anyhow::Result<Arc<dyn Provider>> {
    let host = required_env("DATABRICKS_HOST")?;
    let auth = match env("DATABRICKS_TOKEN") {
        Some(token) => DatabricksAuth::token(token),
        None => DatabricksAuth::oauth(host.clone()),
    };
    let oauth_token_source = match &auth {
        DatabricksAuth::OAuth { .. } => Some(buzz_model_catalog::auth::PkceOAuthTokenSource::new(
            buzz_model_catalog::databricks_pkce_config(&host),
        )?),
        DatabricksAuth::Token(_) => None,
    };
    let oauth_token_provider = oauth_token_source
        .as_ref()
        .map(|source| buzz_oauth_token_provider(Arc::clone(source)));
    let refresh_hook = oauth_token_source.map(buzz_oauth_refresh_hook);
    let token_resolver = Some(Arc::new(|| env("DATABRICKS_TOKEN")) as _);
    if v2 {
        let retry = DatabricksV2Provider::load_retry_config(env);
        Ok(Arc::new(DatabricksV2Provider::new(
            host,
            auth,
            retry,
            None,
            oauth_token_provider,
            token_resolver,
            None,
            refresh_hook,
        )?))
    } else {
        let retry = DatabricksProvider::load_retry_config(env);
        Ok(Arc::new(DatabricksProvider::new(
            host,
            auth,
            retry,
            None,
            oauth_token_provider,
            token_resolver,
            None,
            None,
            refresh_hook,
            None,
        )?))
    }
}

fn buzz_oauth_token_provider(
    source: Arc<buzz_model_catalog::auth::PkceOAuthTokenSource>,
) -> DatabricksOauthTokenProvider {
    Arc::new(move |_host, _client_id, _redirect_url, _scopes| {
        let source = Arc::clone(&source);
        Box::pin(async move {
            use buzz_model_catalog::auth::TokenSource;
            source
                .bearer_no_browser()
                .await
                .map_err(|error| anyhow::anyhow!("{error}"))
        })
    })
}

fn buzz_oauth_refresh_hook(
    source: Arc<buzz_model_catalog::auth::PkceOAuthTokenSource>,
) -> DatabricksRefreshHook {
    Arc::new(move || source.reject_current_bearer())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn unsupported_provider_fails_clearly() {
        let error = match build("not-a-provider").await {
            Ok(_) => panic!("unsupported provider was accepted"),
            Err(error) => error.to_string(),
        };
        assert!(error.contains("unsupported Buzz provider"));
        assert!(error.contains("openrouter"));
    }
}
