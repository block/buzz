use std::sync::Arc;
use std::time::Duration;

use buzz_client::{
    AuthContext, BuzzClient as ReusableBuzzClient, ClientConfig, ClientError, CommunityEndpoint,
    RetryPolicy,
};
use nostr::Keys;

use crate::error::CliError;

pub async fn build_reusable_client(
    relay_url: &str,
    keys: &Keys,
    auth_tag_json: Option<&str>,
) -> Result<ReusableBuzzClient, CliError> {
    let endpoint = CommunityEndpoint::parse(relay_url)
        .map_err(ClientError::from)
        .map_err(map_client_error)?;
    let retry = RetryPolicy::new(3, Duration::from_millis(500), Duration::from_millis(1500))
        .map_err(map_client_error)?;
    let config = ClientConfig::new(
        env_duration_secs("BUZZ_TIMEOUT_SECS", 30),
        env_duration_secs("BUZZ_CONNECT_TIMEOUT_SECS", 15),
        retry,
    )
    .map_err(map_client_error)?;
    let auth = match auth_tag_json {
        Some(json) => AuthContext::nip_oa(json).map_err(map_client_error)?,
        None => AuthContext::signer_only(),
    };
    ReusableBuzzClient::builder(endpoint, Arc::new(keys.clone()))
        .config(config)
        .auth_context(auth)
        .build()
        .await
        .map_err(map_client_error)
}

pub fn map_client_error(error: ClientError) -> CliError {
    match error {
        ClientError::Endpoint(error) => CliError::Usage(error.to_string()),
        ClientError::Configuration(message) | ClientError::InvalidInput(message) => {
            CliError::Usage(message)
        }
        ClientError::Authentication(message) => CliError::Auth(message),
        ClientError::Signer(error) => CliError::Key(error.to_string()),
        ClientError::HttpClient(error) => CliError::Other(error.to_string()),
        ClientError::Network(error) => CliError::Network(error),
        ClientError::Relay { status, reason, .. } => CliError::Relay {
            status,
            body: reason,
        },
        ClientError::Serialization(error) => CliError::Other(error.to_string()),
    }
}

fn env_duration_secs(name: &str, default: u64) -> Duration {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|seconds| *seconds > 0)
        .map(Duration::from_secs)
        .unwrap_or_else(|| Duration::from_secs(default))
}
