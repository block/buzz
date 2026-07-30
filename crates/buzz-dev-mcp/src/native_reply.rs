use rmcp::ErrorData;
use schemars::JsonSchema;
use serde::Deserialize;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReplyParams {
    /// Channel UUID from the current Buzz prompt context.
    pub channel_id: String,
    /// Reply text to publish.
    #[schemars(length(min = 1, max = 65536))]
    pub content: String,
    /// Event ID supplied as the current prompt's reply destination.
    pub reply_to: String,
    /// Thread root for nested agent-to-agent replies. Omit for ordinary
    /// human-facing replies, where `reply_to` is already the root.
    #[serde(default)]
    pub thread_root: Option<String>,
    /// Exact hex public keys to notify. Names alone are not accepted.
    #[serde(default)]
    #[schemars(length(max = 50))]
    pub mention_pubkeys: Vec<String>,
}

pub async fn run(params: ReplyParams) -> Result<String, ErrorData> {
    let receipt = buzz_cli::native::reply_from_env(
        &params.channel_id,
        &params.content,
        &params.reply_to,
        params.thread_root.as_deref(),
        &params.mention_pubkeys,
    )
    .await
    .map_err(|error| ErrorData::internal_error(error, None))?;

    serde_json::to_string(&receipt)
        .map_err(|error| ErrorData::internal_error(error.to_string(), None))
}
