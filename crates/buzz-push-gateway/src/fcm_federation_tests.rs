//! Federated token minting, driven through `TokenSource` against fake AWS
//! credential, Google STS and impersonation servers. No real credential or
//! network is involved.

use super::*;
use crate::aws_credentials::AwsCredentialSource;
use axum::{
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode as Status},
    routing::{get, post},
    Router,
};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

const NOW: i64 = 1_750_000_000;
const FCM_SCOPE_URL: &str = "https://www.googleapis.com/auth/firebase.messaging";

/// One fake hop: the response it gives and how long it takes to give it.
#[derive(Clone)]
struct Hop {
    response: (u16, String),
    delay: Duration,
}

impl Hop {
    fn ok(body: impl Into<String>) -> Self {
        Self {
            response: (200, body.into()),
            delay: Duration::ZERO,
        }
    }
}

struct Fake {
    aws: Hop,
    web_identity: Hop,
    sts: Hop,
    impersonation: Hop,
    /// Request count and last (headers, body) per hop, by path.
    requests: HashMap<&'static str, Vec<(HeaderMap, String)>>,
}

type Shared = Arc<Mutex<Fake>>;

fn expire_time(at: i64) -> String {
    chrono::DateTime::from_timestamp(at, 0)
        .unwrap()
        .to_rfc3339()
}

fn impersonated(n: usize, expires_at: i64) -> String {
    serde_json::json!({"accessToken": format!("fcm-{n}"), "expireTime": expire_time(expires_at)})
        .to_string()
}

fn new_fake() -> Shared {
    Arc::new(Mutex::new(Fake {
        aws: Hop::ok(
            r#"{"AccessKeyId":"AKIDPOD","SecretAccessKey":"pod-secret","Token":"pod-session","Expiration":"2030-01-01T00:00:00Z"}"#,
        ),
        web_identity: Hop::ok(
            "<AssumeRoleWithWebIdentityResponse xmlns=\"https://sts.amazonaws.com/doc/2011-06-15/\">\
             <AssumeRoleWithWebIdentityResult><Credentials>\
             <AccessKeyId>AKIDIRSA</AccessKeyId><SecretAccessKey>irsa-secret</SecretAccessKey>\
             <SessionToken>irsa-session</SessionToken><Expiration>2030-01-01T00:00:00Z</Expiration>\
             </Credentials></AssumeRoleWithWebIdentityResult></AssumeRoleWithWebIdentityResponse>",
        ),
        sts: Hop::ok(r#"{"access_token":"federated-1","token_type":"Bearer","expires_in":3599}"#),
        impersonation: Hop::ok(impersonated(1, NOW + 3600)),
        requests: HashMap::new(),
    }))
}

async fn serve_hop(
    fake: &Shared,
    name: &'static str,
    headers: HeaderMap,
    body: Bytes,
    pick: fn(&Fake) -> Hop,
) -> (Status, String) {
    let hop = {
        let mut f = fake.lock().unwrap();
        f.requests
            .entry(name)
            .or_default()
            .push((headers, String::from_utf8(body.to_vec()).unwrap()));
        pick(&f)
    };
    tokio::time::sleep(hop.delay).await;
    (Status::from_u16(hop.response.0).unwrap(), hop.response.1)
}

fn app(fake: Shared) -> Router {
    Router::new()
        .route(
            "/aws",
            get(|State(f): State<Shared>, h: HeaderMap| async move {
                serve_hop(&f, "aws", h, Bytes::new(), |f| f.aws.clone()).await
            }),
        )
        .route(
            "/",
            post(
                |State(f): State<Shared>, h: HeaderMap, b: Bytes| async move {
                    serve_hop(&f, "web_identity", h, b, |f| f.web_identity.clone()).await
                },
            ),
        )
        .route(
            "/google/sts",
            post(
                |State(f): State<Shared>, h: HeaderMap, b: Bytes| async move {
                    serve_hop(&f, "sts", h, b, |f| f.sts.clone()).await
                },
            ),
        )
        .route(
            "/google/impersonate",
            post(
                |State(f): State<Shared>, h: HeaderMap, b: Bytes| async move {
                    serve_hop(&f, "impersonation", h, b, |f| f.impersonation.clone()).await
                },
            ),
        )
        .route(
            "/v1/projects/example-project/messages:send",
            post(
                |State(f): State<Shared>, h: HeaderMap, b: Bytes| async move {
                    serve_hop(&f, "send", h, b, |_| Hop::ok("{}")).await
                },
            ),
        )
        .with_state(fake)
}

async fn serve_fake(fake: &Shared) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let app = app(fake.clone());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    base
}

/// A Google `external_account` config for an AWS provider whose Google hops
/// are served at `base`.
fn config(base: &str) -> serde_json::Value {
    serde_json::json!({
        "type": "external_account",
        "audience": "//iam.googleapis.com/projects/1/locations/global/workloadIdentityPools/example-pool/providers/example-aws",
        "subject_token_type": "urn:ietf:params:aws:token-type:aws4_request",
        "token_url": format!("{base}/google/sts"),
        "service_account_impersonation_url": format!("{base}/google/impersonate"),
        "credential_source": {
            "environment_id": "aws1",
            "region_url": "http://169.254.169.254/latest/meta-data/placement/availability-zone",
            "url": "http://169.254.169.254/latest/meta-data/iam/security-credentials",
            "regional_cred_verification_url":
                format!("{base}/sts/{{region}}?Action=GetCallerIdentity&Version=2011-06-15"),
        },
    })
}

/// Environment for the EKS Pod Identity source, with the token in `token_file`.
fn pod_env(base: &str, token_file: &std::path::Path) -> HashMap<String, String> {
    HashMap::from([
        ("AWS_REGION".to_owned(), "us-east-1".to_owned()),
        (
            "AWS_CONTAINER_CREDENTIALS_FULL_URI".to_owned(),
            format!("{base}/aws"),
        ),
        (
            "AWS_CONTAINER_AUTHORIZATION_TOKEN_FILE".to_owned(),
            token_file.to_str().unwrap().to_owned(),
        ),
    ])
}

/// A file in the system temp directory, deleted when dropped.
struct TempFile(std::path::PathBuf);

impl std::ops::Deref for TempFile {
    type Target = std::path::Path;
    fn deref(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Write `content` to a fresh file that lives as long as the returned guard.
fn temp_file(content: &str) -> TempFile {
    let path = std::env::temp_dir().join(format!("buzz-fcm-test-{}", uuid::Uuid::new_v4()));
    std::fs::write(&path, content).unwrap();
    TempFile(path)
}

fn federated(
    base: &str,
    env: HashMap<String, String>,
    now: fn() -> i64,
) -> Result<FcmTransport, FcmError> {
    FcmTransport::with_endpoints(
        "example-project",
        &serde_json::to_vec(&config(base)).unwrap(),
        base,
        now,
        move |name| env.get(name).cloned(),
    )
}

fn count(fake: &Shared, hop: &str) -> usize {
    fake.lock()
        .unwrap()
        .requests
        .get(hop)
        .map_or(0, |requests| requests.len())
}

fn last(fake: &Shared, hop: &str) -> (HeaderMap, String) {
    fake.lock().unwrap().requests[hop].last().unwrap().clone()
}

fn form(body: &str) -> HashMap<String, String> {
    url::form_urlencoded::parse(body.as_bytes())
        .into_owned()
        .collect()
}

fn attempt() -> DeliveryAttempt {
    DeliveryAttempt {
        request_id: uuid::Uuid::nil(),
        expires_at: NOW + 600,
    }
}

#[tokio::test]
async fn a_send_mints_through_aws_google_sts_and_impersonation_once() {
    let fake = new_fake();
    let base = serve_fake(&fake).await;
    let token_file = temp_file("pod-identity-token\n");
    let transport = federated(&base, pod_env(&base, &token_file), || NOW).unwrap();
    // Building the transport called nothing.
    assert_eq!(count(&fake, "aws") + count(&fake, "sts"), 0);

    for _ in 0..2 {
        assert_eq!(
            transport.send(attempt(), "registration-token").await,
            DeliveryOutcome::Accepted
        );
    }
    // One mint serves both sends.
    for hop in ["aws", "sts", "impersonation"] {
        assert_eq!(count(&fake, hop), 1, "{hop}");
    }
    assert_eq!(
        last(&fake, "aws").0["authorization"],
        "pod-identity-token",
        "the agent token file's content, without its trailing newline"
    );

    let sts = form(&last(&fake, "sts").1);
    assert_eq!(
        sts["grant_type"],
        "urn:ietf:params:oauth:grant-type:token-exchange"
    );
    assert_eq!(
        sts["requested_token_type"],
        "urn:ietf:params:oauth:token-type:access_token"
    );
    assert_eq!(
        sts["subject_token_type"],
        "urn:ietf:params:aws:token-type:aws4_request"
    );
    assert_eq!(
        sts["scope"],
        "https://www.googleapis.com/auth/cloud-platform"
    );
    let audience = config(&base)["audience"].as_str().unwrap().to_owned();
    assert_eq!(sts["audience"], audience);
    let subject: serde_json::Value = serde_json::from_str(
        &url::form_urlencoded::parse(format!("x={}", sts["subject_token"]).as_bytes())
            .next()
            .unwrap()
            .1,
    )
    .unwrap();
    assert_eq!(subject["method"], "POST");
    assert_eq!(
        subject["url"],
        format!("{base}/sts/us-east-1?Action=GetCallerIdentity&Version=2011-06-15")
    );
    let headers: HashMap<String, String> = subject["headers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| {
            (
                h["key"].as_str().unwrap().to_owned(),
                h["value"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    assert_eq!(headers["x-goog-cloud-target-resource"], audience);
    assert_eq!(headers["x-amz-security-token"], "pod-session");
    assert!(headers["Authorization"].starts_with("AWS4-HMAC-SHA256 Credential=AKIDPOD/20250615/us-east-1/sts/aws4_request, SignedHeaders=host;x-amz-date;x-amz-security-token;x-goog-cloud-target-resource, Signature="));

    let (impersonation_headers, impersonation_body) = last(&fake, "impersonation");
    assert_eq!(impersonation_headers["authorization"], "Bearer federated-1");
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&impersonation_body).unwrap(),
        serde_json::json!({"scope": [FCM_SCOPE_URL]})
    );
    assert_eq!(FCM_SCOPE_URL, FCM_SCOPE);
    for (headers, _) in &fake.lock().unwrap().requests["send"] {
        assert_eq!(headers["authorization"], "Bearer fcm-1");
    }
}

#[tokio::test]
async fn web_identity_and_static_credentials_sign_the_subject_token() {
    // IRSA: the token file's content is exchanged at the STS endpoint derived
    // from the verification URL, and the assumed role signs.
    let fake = new_fake();
    let base = serve_fake(&fake).await;
    let token_file = temp_file("projected-web-identity-token\n");
    let env = HashMap::from([
        ("AWS_REGION".to_owned(), "us-east-1".to_owned()),
        (
            "AWS_ROLE_ARN".to_owned(),
            "arn:aws:iam::123456789012:role/example".to_owned(),
        ),
        (
            "AWS_WEB_IDENTITY_TOKEN_FILE".to_owned(),
            token_file.to_str().unwrap().to_owned(),
        ),
    ]);
    let transport = federated(&base, env, || NOW).unwrap();
    assert_eq!(transport.tokens.access_token().await.ok().unwrap(), "fcm-1");
    let web = form(&last(&fake, "web_identity").1);
    assert_eq!(web["Action"], "AssumeRoleWithWebIdentity");
    assert_eq!(web["RoleArn"], "arn:aws:iam::123456789012:role/example");
    assert_eq!(web["WebIdentityToken"], "projected-web-identity-token");
    assert_eq!(count(&fake, "aws"), 0);
    let subject = form(&last(&fake, "sts").1)["subject_token"].clone();
    assert!(subject.contains("AKIDIRSA"), "{subject}");
    assert!(subject.contains("irsa-session"), "{subject}");

    // Static keys need no endpoint at all.
    let fake = new_fake();
    let base = serve_fake(&fake).await;
    let env = HashMap::from([
        ("AWS_DEFAULT_REGION".to_owned(), "us-east-1".to_owned()),
        ("AWS_ACCESS_KEY_ID".to_owned(), "AKIDSTATIC".to_owned()),
        (
            "AWS_SECRET_ACCESS_KEY".to_owned(),
            "static-secret".to_owned(),
        ),
    ]);
    let transport = federated(&base, env, || NOW).unwrap();
    assert_eq!(transport.tokens.access_token().await.ok().unwrap(), "fcm-1");
    let subject = form(&last(&fake, "sts").1)["subject_token"].clone();
    assert!(subject.contains("AKIDSTATIC"), "{subject}");
    assert!(!subject.contains("x-amz-security-token"), "{subject}");
    assert_eq!(count(&fake, "aws") + count(&fake, "web_identity"), 0);
}

#[tokio::test]
async fn each_hop_failure_is_classified_rejected_or_unavailable() {
    type Set = fn(&mut Fake, Hop);
    let aws: Set = |f, hop| f.aws = hop;
    let sts: Set = |f, hop| f.sts = hop;
    let impersonation: Set = |f, hop| f.impersonation = hop;
    let status = |code: u16| Hop {
        response: (code, "{}".into()),
        delay: Duration::ZERO,
    };
    let rows: [(&str, Set, Hop, TokenFault); 21] = [
        ("aws 500", aws, status(500), TokenFault::Unavailable),
        ("aws 429", aws, status(429), TokenFault::Unavailable),
        ("aws 403", aws, status(403), TokenFault::Rejected),
        (
            "aws garbage",
            aws,
            Hop::ok("not json"),
            TokenFault::Unavailable,
        ),
        ("sts 500", sts, status(500), TokenFault::Unavailable),
        ("sts 503", sts, status(503), TokenFault::Unavailable),
        ("sts 429", sts, status(429), TokenFault::Unavailable),
        ("sts 400", sts, status(400), TokenFault::Rejected),
        ("sts 401", sts, status(401), TokenFault::Rejected),
        ("sts 403", sts, status(403), TokenFault::Rejected),
        (
            "sts garbage",
            sts,
            Hop::ok("not json"),
            TokenFault::Unavailable,
        ),
        (
            "sts empty token",
            sts,
            Hop::ok(r#"{"access_token":""}"#),
            TokenFault::Rejected,
        ),
        (
            "impersonation 500",
            impersonation,
            status(500),
            TokenFault::Unavailable,
        ),
        (
            "impersonation 429",
            impersonation,
            status(429),
            TokenFault::Unavailable,
        ),
        (
            "impersonation 400",
            impersonation,
            status(400),
            TokenFault::Rejected,
        ),
        (
            "impersonation 403",
            impersonation,
            status(403),
            TokenFault::Rejected,
        ),
        (
            "impersonation garbage",
            impersonation,
            Hop::ok("not json"),
            TokenFault::Unavailable,
        ),
        (
            "impersonation empty token",
            impersonation,
            Hop::ok(r#"{"accessToken":"","expireTime":"2030-01-01T00:00:00Z"}"#),
            TokenFault::Rejected,
        ),
        (
            "impersonation bad expireTime",
            impersonation,
            Hop::ok(r#"{"accessToken":"t","expireTime":"soon"}"#),
            TokenFault::Unavailable,
        ),
        (
            "impersonation missing expireTime",
            impersonation,
            Hop::ok(r#"{"accessToken":"t"}"#),
            TokenFault::Unavailable,
        ),
        ("aws unreachable", aws, status(200), TokenFault::Unavailable),
    ];
    for (label, set, hop, expected) in rows {
        let fake = new_fake();
        set(&mut fake.lock().unwrap(), hop);
        let base = serve_fake(&fake).await;
        let token_file = temp_file("token");
        let mut env = pod_env(&base, &token_file);
        if label == "aws unreachable" {
            // Nothing listens on a port that was just released.
            let closed = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = closed.local_addr().unwrap().port();
            drop(closed);
            env.insert(
                "AWS_CONTAINER_CREDENTIALS_FULL_URI".into(),
                format!("http://127.0.0.1:{port}/aws"),
            );
        }
        let transport = federated(&base, env, || NOW).unwrap();
        let result = transport.tokens.access_token().await;
        assert_eq!(result.err(), Some(expected), "{label}");
        // A hop that failed stops the chain: later hops are never called.
        if label.starts_with("aws") {
            assert_eq!(count(&fake, "sts"), 0, "{label}");
        }
        if label.starts_with("sts") {
            assert_eq!(count(&fake, "impersonation"), 0, "{label}");
        }
    }
}

#[tokio::test]
async fn an_unreadable_aws_token_file_is_rejected_not_retried() {
    let fake = new_fake();
    let base = serve_fake(&fake).await;
    let missing = std::env::temp_dir().join(format!("buzz-fcm-missing-{}", uuid::Uuid::new_v4()));
    let transport = federated(&base, pod_env(&base, &missing), || NOW).unwrap();
    assert_eq!(
        transport.tokens.access_token().await.err(),
        Some(TokenFault::Rejected)
    );
    assert_eq!(count(&fake, "aws"), 0);
}

#[tokio::test]
async fn a_failed_hop_keeps_the_valid_cached_token_and_lifetime_comes_from_expire_time() {
    use std::sync::atomic::{AtomicI64, Ordering};
    static CLOCK: AtomicI64 = AtomicI64::new(NOW);
    fn clock() -> i64 {
        CLOCK.load(Ordering::SeqCst)
    }
    let fake = new_fake();
    // expireTime says 30 minutes, not the usual hour.
    fake.lock().unwrap().impersonation = Hop::ok(impersonated(1, NOW + 1800));
    let base = serve_fake(&fake).await;
    let token_file = temp_file("token");
    let transport = Arc::new(federated(&base, pod_env(&base, &token_file), clock).unwrap());
    assert_eq!(
        transport.send(attempt(), "registration-token").await,
        DeliveryOutcome::Accepted
    );

    // The refresh margin is measured from expireTime: due 120 s before it.
    CLOCK.store(NOW + 1800 - 119, Ordering::SeqCst);
    fake.lock().unwrap().impersonation.response = (503, "{}".into());
    assert_eq!(
        transport.send(attempt(), "registration-token").await,
        DeliveryOutcome::Accepted,
        "the valid token serves while its refresh runs"
    );
    for _ in 0..500 {
        if count(&fake, "impersonation") == 2 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(
        count(&fake, "impersonation"),
        2,
        "the refresh was attempted"
    );
    // The background refresh holds `fetch` until it has recorded its failure.
    drop(transport.tokens.fetch.lock().await);
    assert_eq!(
        transport.send(attempt(), "registration-token").await,
        DeliveryOutcome::Accepted
    );
    for (headers, _) in &fake.lock().unwrap().requests["send"] {
        assert_eq!(
            headers["authorization"], "Bearer fcm-1",
            "the token was kept"
        );
    }
    assert_eq!(
        count(&fake, "impersonation"),
        2,
        "backoff stops a retry storm"
    );

    // Once the kept token expires, the recorded failure fails fast.
    CLOCK.store(NOW + 1800, Ordering::SeqCst);
    assert_eq!(
        transport.send(attempt(), "registration-token").await,
        DeliveryOutcome::Retry {
            retry_after_seconds: None
        }
    );
}

#[tokio::test]
async fn one_deadline_bounds_the_whole_chain_of_hops() {
    let delayed = |fake: &Shared| {
        let f = &mut *fake.lock().unwrap();
        for hop in [&mut f.aws, &mut f.sts, &mut f.impersonation] {
            hop.delay = Duration::from_millis(250);
        }
    };
    let build = |base: &str, deadline: Duration| {
        let token_file = temp_file("token");
        let mut transport = federated(base, pod_env(base, &token_file), || NOW).unwrap();
        Arc::get_mut(&mut transport.tokens).unwrap().deadline = deadline;
        // The token file must outlive the mint, so it travels with the transport.
        (transport, token_file)
    };

    // Three 250 ms hops fit a 5 s deadline...
    let fake = new_fake();
    delayed(&fake);
    let base = serve_fake(&fake).await;
    let (transport, _token_file) = build(&base, Duration::from_secs(5));
    assert_eq!(transport.tokens.access_token().await.ok().unwrap(), "fcm-1");

    // ...but no single hop outlasts a 375 ms deadline, yet the chain does:
    // it fires during the second hop, so impersonation is never reached.
    let fake = new_fake();
    delayed(&fake);
    let base = serve_fake(&fake).await;
    let (transport, _token_file) = build(&base, Duration::from_millis(375));
    assert_eq!(
        transport.tokens.access_token().await.err(),
        Some(TokenFault::Unavailable)
    );
    assert_eq!(count(&fake, "aws"), 1);
    assert_eq!(count(&fake, "impersonation"), 0);
}

fn rejected(config: serde_json::Value, env: &HashMap<String, String>) -> Option<&'static str> {
    let env = env.clone();
    match FcmTransport::with_endpoints(
        "example-project",
        &serde_json::to_vec(&config).unwrap(),
        FCM_BASE_URL,
        || NOW,
        move |name| env.get(name).cloned(),
    ) {
        Ok(_) => None,
        Err(FcmError::Configuration(reason)) => Some(reason),
        Err(other) => panic!("expected a configuration error, got {other}"),
    }
}

#[tokio::test]
async fn startup_validates_the_file_and_environment_without_calling_anyone() {
    let fake = new_fake();
    let base = serve_fake(&fake).await;
    let token_file = temp_file("token");
    let env = pod_env(&base, &token_file);
    assert_eq!(rejected(config(&base), &env), None);
    assert_eq!(
        fake.lock().unwrap().requests.len(),
        0,
        "startup must not call AWS or Google"
    );

    let edit = |change: &dyn Fn(&mut serde_json::Value)| {
        let mut value = config(&base);
        change(&mut value);
        value
    };
    let without = |key: &'static str| edit(&move |v| drop(v.as_object_mut().unwrap().remove(key)));
    let source = |value: serde_json::Value| edit(&move |v| v["credential_source"] = value.clone());
    let cases = [
        (
            "no impersonation url",
            without("service_account_impersonation_url"),
        ),
        ("no audience", without("audience")),
        ("no token url", without("token_url")),
        ("empty audience", edit(&|v| v["audience"] = "".into())),
        ("bad token url", edit(&|v| v["token_url"] = "nope".into())),
        ("file source", source(serde_json::json!({"file": "/token"}))),
        (
            "url source",
            source(
                serde_json::json!({"url": "https://example.org/token", "format": {"type": "text"}}),
            ),
        ),
        (
            "executable source",
            source(serde_json::json!({"executable": {"command": "x"}})),
        ),
        (
            "other environment",
            source(
                serde_json::json!({"environment_id": "aws2", "regional_cred_verification_url": "https://sts.{region}.example.org"}),
            ),
        ),
        (
            "no verification url",
            source(serde_json::json!({"environment_id": "aws1"})),
        ),
        (
            "no region placeholder",
            source(
                serde_json::json!({"environment_id": "aws1", "regional_cred_verification_url": "https://sts.example.org?Action=GetCallerIdentity"}),
            ),
        ),
    ];
    for (label, config) in cases {
        assert!(rejected(config, &env).is_some(), "{label} must be rejected");
    }

    let without_env = |name: &str| {
        let mut env = env.clone();
        env.remove(name);
        env
    };
    let env_cases = [
        ("no region", without_env("AWS_REGION")),
        (
            "no credentials",
            HashMap::from([("AWS_REGION".to_owned(), "us-east-1".to_owned())]),
        ),
        (
            "access key without secret",
            HashMap::from([
                ("AWS_REGION".to_owned(), "us-east-1".to_owned()),
                ("AWS_ACCESS_KEY_ID".to_owned(), "AKID".to_owned()),
            ]),
        ),
        (
            "web identity without role",
            HashMap::from([
                ("AWS_REGION".to_owned(), "us-east-1".to_owned()),
                (
                    "AWS_WEB_IDENTITY_TOKEN_FILE".to_owned(),
                    "/token".to_owned(),
                ),
            ]),
        ),
        (
            "token over plain http to a remote host",
            HashMap::from([
                ("AWS_REGION".to_owned(), "us-east-1".to_owned()),
                (
                    "AWS_CONTAINER_CREDENTIALS_FULL_URI".to_owned(),
                    "http://credentials.example.org/v1".to_owned(),
                ),
            ]),
        ),
    ];
    for (label, env) in env_cases {
        assert!(
            rejected(config(&base), &env).is_some(),
            "{label} must be rejected"
        );
    }

    // The EKS Pod Identity Agent's own address is accepted over http.
    let mut agent = env.clone();
    agent.insert(
        "AWS_CONTAINER_CREDENTIALS_FULL_URI".into(),
        "http://169.254.170.23/v1/credentials".into(),
    );
    assert_eq!(rejected(config(&base), &agent), None);
}

#[test]
fn the_aws_credential_source_is_chosen_by_precedence() {
    let pick = |pairs: &[(&str, &str)]| {
        let env: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        AwsCredentialSource::from_lookup(move |n| env.get(n).cloned(), "https://sts/".into()).map(
            |source| match source {
                AwsCredentialSource::Static(_) => "static",
                AwsCredentialSource::WebIdentity { .. } => "web identity",
                AwsCredentialSource::PodIdentity { .. } => "pod identity",
            },
        )
    };
    let all = [
        ("AWS_ACCESS_KEY_ID", "AKID"),
        ("AWS_SECRET_ACCESS_KEY", "secret"),
        ("AWS_ROLE_ARN", "arn"),
        ("AWS_WEB_IDENTITY_TOKEN_FILE", "/t"),
        (
            "AWS_CONTAINER_CREDENTIALS_FULL_URI",
            "https://agent.example.org/c",
        ),
    ];
    assert_eq!(pick(&all).unwrap(), "static");
    assert_eq!(pick(&all[2..]).unwrap(), "web identity");
    assert_eq!(pick(&all[4..]).unwrap(), "pod identity");
    // Only instance metadata style settings are not a source.
    assert!(pick(&[("AWS_CONTAINER_CREDENTIALS_RELATIVE_URI", "/v2/credentials")]).is_err());
    // Empty values count as unset.
    assert!(pick(&[("AWS_ACCESS_KEY_ID", "")]).is_err());
}
