//! The gateway's AWS identity, for signing the federation subject token.
//!
//! Three sources, chosen once at startup from the environment in this order:
//!
//! 1. Static `AWS_ACCESS_KEY_ID` / `AWS_SECRET_ACCESS_KEY` /
//!    `AWS_SESSION_TOKEN`.
//! 2. IAM roles for service accounts: `AWS_ROLE_ARN` and
//!    `AWS_WEB_IDENTITY_TOKEN_FILE`, exchanged at STS
//!    `AssumeRoleWithWebIdentity`.
//! 3. EKS Pod Identity: `AWS_CONTAINER_CREDENTIALS_FULL_URI` and
//!    `AWS_CONTAINER_AUTHORIZATION_TOKEN_FILE`, read from the agent's
//!    credentials endpoint.
//!
//! EC2 instance metadata is deliberately unsupported: pods cannot reach it.
//! Nothing is fetched at startup, and credentials are fetched on every mint
//! (about once an hour), so no AWS credential is cached here.

use std::{fmt, path::PathBuf};

use reqwest::StatusCode;
use serde::Deserialize;
use url::{Host, Url};

use crate::fcm::{FcmError, MintFailure};

const ROLE_SESSION_NAME: &str = "buzz-push-gateway";
const STS_API_VERSION: &str = "2011-06-15";
/// The EKS Pod Identity Agent's link-local addresses.
const POD_IDENTITY_IPV4: [u8; 4] = [169, 254, 170, 23];
const POD_IDENTITY_IPV6: [u16; 8] = [0xfd00, 0x0ec2, 0, 0, 0, 0, 0, 0x23];

#[derive(Clone, PartialEq, Eq)]
pub struct AwsCredentials {
    pub access_key_id: String,
    pub secret_access_key: String,
    pub session_token: Option<String>,
}

impl fmt::Debug for AwsCredentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AwsCredentials").finish_non_exhaustive()
    }
}

#[derive(Debug)]
pub enum AwsCredentialSource {
    Static(AwsCredentials),
    WebIdentity {
        role_arn: String,
        token_file: PathBuf,
        /// The STS endpoint to post `AssumeRoleWithWebIdentity` to.
        sts_url: String,
    },
    PodIdentity {
        url: String,
        token_file: Option<PathBuf>,
    },
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct ContainerCredentials {
    access_key_id: String,
    secret_access_key: String,
    token: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct AssumeRoleWithWebIdentityResponse {
    assume_role_with_web_identity_result: AssumeRoleResult,
}
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct AssumeRoleResult {
    credentials: StsCredentials,
}
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct StsCredentials {
    access_key_id: String,
    secret_access_key: String,
    session_token: String,
}

impl AwsCredentialSource {
    /// Pick the source named by the environment `get` reads. `sts_url` is
    /// where a web identity token is exchanged. Fails when no supported
    /// source is configured or its settings are incomplete.
    pub fn from_lookup(
        get: impl Fn(&str) -> Option<String>,
        sts_url: String,
    ) -> Result<Self, FcmError> {
        let get = |name: &str| get(name).filter(|value| !value.is_empty());
        if let Some(access_key_id) = get("AWS_ACCESS_KEY_ID") {
            let secret_access_key = get("AWS_SECRET_ACCESS_KEY").ok_or(FcmError::Configuration(
                "AWS_ACCESS_KEY_ID is set without AWS_SECRET_ACCESS_KEY",
            ))?;
            return Ok(Self::Static(AwsCredentials {
                access_key_id,
                secret_access_key,
                session_token: get("AWS_SESSION_TOKEN"),
            }));
        }
        if let Some(token_file) = get("AWS_WEB_IDENTITY_TOKEN_FILE") {
            let role_arn = get("AWS_ROLE_ARN").ok_or(FcmError::Configuration(
                "AWS_WEB_IDENTITY_TOKEN_FILE is set without AWS_ROLE_ARN",
            ))?;
            return Ok(Self::WebIdentity {
                role_arn,
                token_file: token_file.into(),
                sts_url,
            });
        }
        if let Some(url) = get("AWS_CONTAINER_CREDENTIALS_FULL_URI") {
            validate_pod_identity_url(&url)?;
            return Ok(Self::PodIdentity {
                url,
                token_file: get("AWS_CONTAINER_AUTHORIZATION_TOKEN_FILE").map(Into::into),
            });
        }
        Err(FcmError::Configuration(
            "no AWS credentials: set AWS_ACCESS_KEY_ID and AWS_SECRET_ACCESS_KEY, or \
             AWS_ROLE_ARN and AWS_WEB_IDENTITY_TOKEN_FILE, or \
             AWS_CONTAINER_CREDENTIALS_FULL_URI",
        ))
    }

    /// The caller's current AWS credentials.
    pub(crate) async fn credentials(
        &self,
        client: &reqwest::Client,
    ) -> Result<AwsCredentials, MintFailure> {
        match self {
            Self::Static(credentials) => Ok(credentials.clone()),
            Self::WebIdentity {
                role_arn,
                token_file,
                sts_url,
            } => {
                // The kubelet rotates the projected token: read it every time.
                let token = read_secret_file(token_file)?;
                let body = url::form_urlencoded::Serializer::new(String::new())
                    .append_pair("Action", "AssumeRoleWithWebIdentity")
                    .append_pair("Version", STS_API_VERSION)
                    .append_pair("RoleArn", role_arn)
                    .append_pair("RoleSessionName", ROLE_SESSION_NAME)
                    .append_pair("WebIdentityToken", &token)
                    .finish();
                let response = client
                    .post(sts_url)
                    .header(
                        reqwest::header::CONTENT_TYPE,
                        "application/x-www-form-urlencoded",
                    )
                    .body(body)
                    .send()
                    .await
                    .map_err(|_| MintFailure::unavailable("aws web identity", None))?;
                let text = response_text(response, "aws web identity").await?;
                let parsed: AssumeRoleWithWebIdentityResponse = quick_xml::de::from_str(&text)
                    .map_err(|_| MintFailure::unavailable("aws web identity", None))?;
                let c = parsed.assume_role_with_web_identity_result.credentials;
                Ok(AwsCredentials {
                    access_key_id: c.access_key_id,
                    secret_access_key: c.secret_access_key,
                    session_token: Some(c.session_token),
                })
            }
            Self::PodIdentity { url, token_file } => {
                let mut request = client.get(url);
                if let Some(path) = token_file {
                    request =
                        request.header(reqwest::header::AUTHORIZATION, read_secret_file(path)?);
                }
                let response = request
                    .send()
                    .await
                    .map_err(|_| MintFailure::unavailable("aws pod identity", None))?;
                let text = response_text(response, "aws pod identity").await?;
                let c: ContainerCredentials = serde_json::from_str(&text)
                    .map_err(|_| MintFailure::unavailable("aws pod identity", None))?;
                Ok(AwsCredentials {
                    access_key_id: c.access_key_id,
                    secret_access_key: c.secret_access_key,
                    session_token: Some(c.token),
                })
            }
        }
    }
}

/// An unreadable token file is a deployment fault that retrying will not fix.
fn read_secret_file(path: &PathBuf) -> Result<String, MintFailure> {
    std::fs::read_to_string(path)
        .map(|content| content.trim_end().to_owned())
        .map_err(|_| MintFailure::rejected("aws token file", None))
}

async fn response_text(
    response: reqwest::Response,
    hop: &'static str,
) -> Result<String, MintFailure> {
    let status = response.status();
    if status.is_server_error() || status == StatusCode::TOO_MANY_REQUESTS {
        return Err(MintFailure::unavailable(hop, Some(status.as_u16())));
    }
    if !status.is_success() {
        return Err(MintFailure::rejected(hop, Some(status.as_u16())));
    }
    response
        .text()
        .await
        .map_err(|_| MintFailure::unavailable(hop, None))
}

/// The authorization token is a credential, so it may only travel over https
/// or to a loopback or Pod Identity Agent address, as the AWS SDKs require.
fn validate_pod_identity_url(value: &str) -> Result<(), FcmError> {
    let url = Url::parse(value).map_err(|_| {
        FcmError::Configuration("AWS_CONTAINER_CREDENTIALS_FULL_URI is not a valid URL")
    })?;
    let trusted = match (url.scheme(), url.host()) {
        ("https", Some(_)) => true,
        ("http", Some(Host::Ipv4(ip))) => ip.is_loopback() || ip.octets() == POD_IDENTITY_IPV4,
        ("http", Some(Host::Ipv6(ip))) => ip.is_loopback() || ip.segments() == POD_IDENTITY_IPV6,
        ("http", Some(Host::Domain(domain))) => domain == "localhost",
        _ => false,
    };
    if trusted {
        Ok(())
    } else {
        Err(FcmError::Configuration(
            "AWS_CONTAINER_CREDENTIALS_FULL_URI must be https, or http to loopback or the \
             EKS Pod Identity Agent",
        ))
    }
}
