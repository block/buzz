use std::time::Duration;

use futures_util::StreamExt;
use reqwest::{Client, Response};
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use super::lmstudio::{
    cloud_chief_payload, cloud_specialist_payload, parse_cloud_chief_output,
    parse_cloud_specialist_output, AdviserExecutionError, AdviserExecutionErrorCode,
    ChiefOfStaffConsolidation, ChiefOfStaffRequest, SpecialistAdviserRequest,
};
use super::types::AdviserContribution;
use crate::command_services::trusted_lan::{CloudProviderConfig, TrustedLanConfig};
use crate::secret_store::SecretStore;

const MAXIMUM_CLOUD_RESPONSE_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CloudProvider {
    LiteLlm,
    OpenAi,
}

#[derive(Clone)]
struct CloudRoute {
    provider: CloudProvider,
    endpoint: String,
    model: String,
    token: String,
}

impl std::fmt::Debug for CloudRoute {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CloudRoute")
            .field("provider", &self.provider)
            .field("endpoint", &self.endpoint)
            .field("model", &self.model)
            .field("token", &"[REDACTED]")
            .finish()
    }
}

#[derive(Clone)]
pub(crate) struct CloudAdviserClient {
    http: Client,
    litellm: Option<CloudRoute>,
    openai: Option<CloudRoute>,
}

impl std::fmt::Debug for CloudAdviserClient {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CloudAdviserClient")
            .field("litellm_configured", &self.litellm.is_some())
            .field("openai_configured", &self.openai.is_some())
            .finish()
    }
}

impl CloudAdviserClient {
    pub(crate) fn from_config(
        config: &TrustedLanConfig,
        timeout: Duration,
    ) -> Result<Self, AdviserExecutionError> {
        let http = Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(timeout.min(Duration::from_secs(5)))
            .read_timeout(timeout)
            .timeout(timeout)
            .build()
            .map_err(|_| policy_error())?;
        let store = SecretStore::shared(crate::app_state::keyring_service());
        Ok(Self {
            http,
            litellm: load_route(store, config.litellm(), CloudProvider::LiteLlm),
            openai: load_route(store, config.openai(), CloudProvider::OpenAi),
        })
    }

    pub(crate) fn available(&self, provider: CloudProvider) -> bool {
        self.route(provider).is_some()
    }

    pub(crate) async fn run_specialist(
        &self,
        provider: CloudProvider,
        request: &SpecialistAdviserRequest,
        cancellation: CancellationToken,
    ) -> Result<AdviserContribution, AdviserExecutionError> {
        let (system, input) = cloud_specialist_payload(request)?;
        let terminal = self
            .complete(provider, system, &input, cancellation)
            .await?;
        parse_cloud_specialist_output(request, &terminal)
    }

    pub(crate) async fn run_chief_of_staff(
        &self,
        provider: CloudProvider,
        request: &ChiefOfStaffRequest,
        cancellation: CancellationToken,
    ) -> Result<ChiefOfStaffConsolidation, AdviserExecutionError> {
        let (system, input) = cloud_chief_payload(request)?;
        let terminal = self
            .complete(provider, system, &input, cancellation)
            .await?;
        parse_cloud_chief_output(request, &terminal)
    }

    async fn complete(
        &self,
        provider: CloudProvider,
        system: &str,
        input: &str,
        cancellation: CancellationToken,
    ) -> Result<String, AdviserExecutionError> {
        if cancellation.is_cancelled() {
            return Err(cancelled());
        }
        let route = self.route(provider).ok_or_else(authentication_error)?;
        let body = match provider {
            CloudProvider::LiteLlm => json!({
                "model": route.model,
                "stream": false,
                "max_completion_tokens": 8192,
                "response_format": {"type": "json_object"},
                "messages": [
                    {"role": "system", "content": system},
                    {"role": "user", "content": input}
                ]
            }),
            CloudProvider::OpenAi => json!({
                "model": route.model,
                "max_output_tokens": 8192,
                "instructions": system,
                "input": input,
                "text": {"format": {"type": "json_object"}}
            }),
        };
        let request = self
            .http
            .post(&route.endpoint)
            .bearer_auth(&route.token)
            .json(&body);
        let response = tokio::select! {
            biased;
            _ = cancellation.cancelled() => return Err(cancelled()),
            response = request.send() => response.map_err(map_transport_error)?,
        };
        parse_provider_response(provider, response).await
    }

    fn route(&self, provider: CloudProvider) -> Option<&CloudRoute> {
        match provider {
            CloudProvider::LiteLlm => self.litellm.as_ref(),
            CloudProvider::OpenAi => self.openai.as_ref(),
        }
    }
}

fn load_route(
    store: &SecretStore,
    config: &CloudProviderConfig,
    provider: CloudProvider,
) -> Option<CloudRoute> {
    if !config.enabled() {
        return None;
    }
    let token = store.load(config.keychain_key()).ok().flatten()?;
    if token.is_empty() || token.len() > 4096 || token.bytes().any(|byte| byte.is_ascii_control()) {
        return None;
    }
    Some(CloudRoute {
        provider,
        endpoint: config.endpoint().to_string(),
        model: config.model().to_string(),
        token,
    })
}

async fn parse_provider_response(
    provider: CloudProvider,
    response: Response,
) -> Result<String, AdviserExecutionError> {
    let status = response.status();
    if matches!(status.as_u16(), 401 | 403) {
        return Err(authentication_error());
    }
    if status.is_redirection() || !status.is_success() {
        return Err(AdviserExecutionError::new(
            AdviserExecutionErrorCode::Transport,
            "cloud provider rejected adviser request",
        ));
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAXIMUM_CLOUD_RESPONSE_BYTES as u64)
    {
        return Err(invalid_output());
    }
    let mut bytes = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(map_transport_error)?;
        if bytes
            .len()
            .checked_add(chunk.len())
            .is_none_or(|length| length > MAXIMUM_CLOUD_RESPONSE_BYTES)
        {
            return Err(invalid_output());
        }
        bytes.extend_from_slice(&chunk);
    }
    let value: Value = serde_json::from_slice(&bytes).map_err(|_| invalid_output())?;
    match provider {
        CloudProvider::LiteLlm => value
            .get("choices")
            .and_then(Value::as_array)
            .and_then(|choices| choices.first())
            .and_then(|choice| choice.get("message"))
            .and_then(|message| message.get("content"))
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(invalid_output),
        CloudProvider::OpenAi => {
            if let Some(text) = value.get("output_text").and_then(Value::as_str) {
                return Ok(text.to_string());
            }
            value
                .get("output")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter(|item| item.get("type").and_then(Value::as_str) == Some("message"))
                .filter_map(|item| item.get("content").and_then(Value::as_array))
                .flatten()
                .find_map(|item| {
                    matches!(
                        item.get("type").and_then(Value::as_str),
                        Some("output_text" | "text")
                    )
                    .then(|| item.get("text").and_then(Value::as_str))
                    .flatten()
                })
                .map(str::to_string)
                .ok_or_else(invalid_output)
        }
    }
}

fn map_transport_error(error: reqwest::Error) -> AdviserExecutionError {
    if error.is_timeout() {
        AdviserExecutionError::new(
            AdviserExecutionErrorCode::Timeout,
            "cloud adviser request timed out",
        )
    } else {
        AdviserExecutionError::new(
            AdviserExecutionErrorCode::Transport,
            "cloud adviser transport failed",
        )
    }
}

const fn cancelled() -> AdviserExecutionError {
    AdviserExecutionError::new(
        AdviserExecutionErrorCode::Cancelled,
        "adviser execution cancelled",
    )
}

const fn authentication_error() -> AdviserExecutionError {
    AdviserExecutionError::new(
        AdviserExecutionErrorCode::Authentication,
        "cloud provider authentication unavailable",
    )
}

const fn invalid_output() -> AdviserExecutionError {
    AdviserExecutionError::new(
        AdviserExecutionErrorCode::InvalidOutput,
        "cloud adviser output rejected",
    )
}

const fn policy_error() -> AdviserExecutionError {
    AdviserExecutionError::new(
        AdviserExecutionErrorCode::PolicyRejected,
        "cloud route policy rejected",
    )
}
