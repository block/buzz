//! Keyless FCM access tokens: Google workload identity federation from AWS.
//!
//! The credential file is Google's standard `external_account` config for an
//! AWS provider. Minting chains three hops under one deadline:
//!
//! 1. the pod's AWS credentials ([`crate::aws_credentials`]),
//! 2. Google STS, trading a SigV4-signed `GetCallerIdentity` subject token
//!    ([`crate::aws_sigv4`]) for a federated access token,
//! 3. the service account's `generateAccessToken`, trading that for an access
//!    token carrying the FCM scope.
//!
//! The file's IMDS fields (`region_url`, `url`, `imdsv2_session_token_url`)
//! are accepted and ignored: pods cannot reach instance metadata, so the
//! region and credentials come from the environment.

use chrono::DateTime;
use reqwest::StatusCode;
use serde::Deserialize;
use url::Url;

use crate::{
    aws_credentials::AwsCredentialSource,
    aws_sigv4,
    fcm::{FcmError, MintFailure, MintedToken},
};

const CLOUD_PLATFORM_SCOPE: &str = "https://www.googleapis.com/auth/cloud-platform";
const AWS_ENVIRONMENT_ID: &str = "aws1";

#[derive(Deserialize)]
struct ExternalAccount {
    audience: String,
    subject_token_type: String,
    token_url: String,
    service_account_impersonation_url: Option<String>,
    credential_source: CredentialSource,
}

/// Google's AWS credential source. `region_url`, `url` and
/// `imdsv2_session_token_url` only matter to instance metadata lookups.
#[derive(Deserialize)]
struct CredentialSource {
    environment_id: Option<String>,
    regional_cred_verification_url: Option<String>,
}

pub(crate) struct FederatedMinter {
    audience: String,
    subject_token_type: String,
    token_url: String,
    impersonation_url: String,
    verification_url: String,
    region: String,
    /// The scope the impersonated token carries.
    scope: &'static str,
    aws: AwsCredentialSource,
}

impl FederatedMinter {
    /// Validate an `external_account` config and the AWS environment it needs.
    /// `env` reads that environment. Nothing is fetched.
    pub(crate) fn from_config(
        config: serde_json::Value,
        scope: &'static str,
        env: impl Fn(&str) -> Option<String>,
    ) -> Result<Self, FcmError> {
        let file: ExternalAccount = serde_json::from_value(config).map_err(|_| {
            FcmError::Configuration(
                "an external_account credential needs audience, subject_token_type, token_url \
                 and credential_source",
            )
        })?;
        if file.credential_source.environment_id.as_deref() != Some(AWS_ENVIRONMENT_ID) {
            return Err(FcmError::Configuration(
                "only the AWS credential source (environment_id aws1) is supported",
            ));
        }
        let verification_url = file
            .credential_source
            .regional_cred_verification_url
            .filter(|url| url.contains("{region}"))
            .ok_or(FcmError::Configuration(
                "credential_source.regional_cred_verification_url with a {region} placeholder \
                 is required",
            ))?;
        let impersonation_url =
            file.service_account_impersonation_url
                .ok_or(FcmError::Configuration(
                    "service_account_impersonation_url is required: FCM needs the service account \
                 access token",
                ))?;
        if file.audience.is_empty() || file.subject_token_type.is_empty() {
            return Err(FcmError::Configuration(
                "audience and subject_token_type must not be empty",
            ));
        }
        for url in [&file.token_url, &impersonation_url] {
            Url::parse(url).map_err(|_| {
                FcmError::Configuration(
                    "token_url and service_account_impersonation_url must be URLs",
                )
            })?;
        }
        let region = ["AWS_REGION", "AWS_DEFAULT_REGION"]
            .iter()
            .find_map(|name| env(name).filter(|value| !value.is_empty()))
            .ok_or(FcmError::Configuration(
                "AWS_REGION or AWS_DEFAULT_REGION is required",
            ))?;
        // The region-scoped STS endpoint doubles as the web identity exchange
        // endpoint, so no partition suffix is assumed.
        let signing_url =
            Url::parse(&verification_url.replace("{region}", &region)).map_err(|_| {
                FcmError::Configuration("regional_cred_verification_url is not a valid URL")
            })?;
        let host = signing_url.host_str().ok_or(FcmError::Configuration(
            "regional_cred_verification_url has no host",
        ))?;
        let sts_url = match signing_url.port() {
            Some(port) => format!("{}://{host}:{port}/", signing_url.scheme()),
            None => format!("{}://{host}/", signing_url.scheme()),
        };
        Ok(Self {
            audience: file.audience,
            subject_token_type: file.subject_token_type,
            token_url: file.token_url,
            impersonation_url,
            verification_url,
            region,
            scope,
            aws: AwsCredentialSource::from_lookup(env, sts_url)?,
        })
    }

    pub(crate) async fn mint(
        &self,
        client: &reqwest::Client,
        now: i64,
    ) -> Result<MintedToken, MintFailure> {
        let credentials = self.aws.credentials(client).await?;
        let subject_token = aws_sigv4::subject_token(
            &self.verification_url,
            &self.region,
            &self.audience,
            &credentials,
            now,
        )
        .map_err(|_| MintFailure::rejected("aws signing", None))?;

        const STS_HOP: &str = "google sts";
        let body = url::form_urlencoded::Serializer::new(String::new())
            .append_pair(
                "grant_type",
                "urn:ietf:params:oauth:grant-type:token-exchange",
            )
            .append_pair(
                "requested_token_type",
                "urn:ietf:params:oauth:token-type:access_token",
            )
            .append_pair("subject_token_type", &self.subject_token_type)
            .append_pair("audience", &self.audience)
            .append_pair("scope", CLOUD_PLATFORM_SCOPE)
            .append_pair("subject_token", &subject_token)
            .finish();
        let response = client
            .post(&self.token_url)
            .header(
                reqwest::header::CONTENT_TYPE,
                "application/x-www-form-urlencoded",
            )
            .body(body)
            .send()
            .await
            .map_err(|_| MintFailure::unavailable(STS_HOP, None))?;
        #[derive(Deserialize)]
        struct FederatedToken {
            access_token: String,
        }
        let federated: FederatedToken = decode(response, STS_HOP).await?;
        if federated.access_token.is_empty() {
            return Err(MintFailure::rejected(STS_HOP, None));
        }

        const IMPERSONATION_HOP: &str = "service account impersonation";
        let response = client
            .post(&self.impersonation_url)
            .bearer_auth(&federated.access_token)
            .json(&serde_json::json!({ "scope": [self.scope] }))
            .send()
            .await
            .map_err(|_| MintFailure::unavailable(IMPERSONATION_HOP, None))?;
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct ImpersonatedToken {
            access_token: String,
            expire_time: String,
        }
        let token: ImpersonatedToken = decode(response, IMPERSONATION_HOP).await?;
        if token.access_token.is_empty() {
            return Err(MintFailure::rejected(IMPERSONATION_HOP, None));
        }
        let expires_at = DateTime::parse_from_rfc3339(&token.expire_time)
            .map_err(|_| MintFailure::unavailable(IMPERSONATION_HOP, None))?
            .timestamp();
        Ok(MintedToken {
            value: token.access_token,
            lifetime_seconds: expires_at.saturating_sub(now),
        })
    }
}

/// Decode a Google token response. 5xx and 429 mean Google is unhealthy; any
/// other failure status means it refused the credential.
async fn decode<T: serde::de::DeserializeOwned>(
    response: reqwest::Response,
    hop: &'static str,
) -> Result<T, MintFailure> {
    let status = response.status();
    if status.is_server_error() || status == StatusCode::TOO_MANY_REQUESTS {
        return Err(MintFailure::unavailable(hop, Some(status.as_u16())));
    }
    if !status.is_success() {
        return Err(MintFailure::rejected(hop, Some(status.as_u16())));
    }
    response
        .json()
        .await
        .map_err(|_| MintFailure::unavailable(hop, None))
}
