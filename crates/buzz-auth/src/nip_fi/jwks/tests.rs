use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

struct FakeJwksFetcher {
    body: Result<String, JwksFetchError>,
    call_count: Arc<AtomicUsize>,
}

impl super::super::verifier::sealed::Sealed for FakeJwksFetcher {}

impl JwksFetcher for FakeJwksFetcher {
    fn fetch_jwks<'a>(
        &'a self,
        _uri: &'a str,
    ) -> impl std::future::Future<Output = Result<String, JwksFetchError>> + Send + 'a {
        let result = self.body.clone();
        self.call_count.fetch_add(1, Ordering::SeqCst);
        async move { result }
    }
}

fn minimal_jwks_json(kid: &str) -> String {
    format!(
        r#"{{"keys":[{{"kty":"EC","crv":"P-256","x":"f83OJ3D2xF1Bg8vub9tLe1gHMzV76e8Tus9uPHvRVEU","y":"x_FEzRu9m36HLN_tue659LNpXW6pCyStikYjKIWI5a0","use":"sig","alg":"ES256","kid":"{kid}"}}]}}"#
    )
}

fn make_config(issuer: &str) -> IssuerJwksConfig {
    IssuerJwksConfig {
        issuer: issuer.to_owned(),
        contract: JwksSourceContract::new(
            format!("https://{issuer}/.well-known/jwks.json"),
            300,
            3600,
        )
        .expect("valid test contract"),
    }
}

fn make_config_with_uri(issuer: &str, jwks_uri: &str) -> Option<IssuerJwksConfig> {
    JwksSourceContract::new(jwks_uri.to_owned(), 300, 3600).map(|contract| IssuerJwksConfig {
        issuer: issuer.to_owned(),
        contract,
    })
}

#[tokio::test]
async fn get_snapshot_returns_sealed_key_set_on_success() {
    let issuer = "https://id.example";
    let fetcher = FakeJwksFetcher {
        body: Ok(minimal_jwks_json("k1")),
        call_count: Arc::new(AtomicUsize::new(0)),
    };
    let source = ProductionJwksSource::new(vec![make_config(issuer)], fetcher).unwrap();

    let ks = source.get_snapshot(issuer).await.unwrap();
    assert_eq!(ks.issuer(), issuer);
}

#[tokio::test]
async fn get_snapshot_returns_none_for_unknown_issuer() {
    let fetcher = FakeJwksFetcher {
        body: Ok(minimal_jwks_json("k1")),
        call_count: Arc::new(AtomicUsize::new(0)),
    };
    let source =
        ProductionJwksSource::new(vec![make_config("https://id.example")], fetcher).unwrap();

    assert!(source.get_snapshot("https://other.example").await.is_none());
}

#[tokio::test]
async fn get_snapshot_returns_none_on_network_error_with_no_cache() {
    let fetcher = FakeJwksFetcher {
        body: Err(JwksFetchError::NetworkError),
        call_count: Arc::new(AtomicUsize::new(0)),
    };
    let issuer = "https://id.example";
    let source = ProductionJwksSource::new(vec![make_config(issuer)], fetcher).unwrap();

    assert!(source.get_snapshot(issuer).await.is_none());
}

#[tokio::test]
async fn get_snapshot_returns_none_on_oversized_response() {
    let fetcher = FakeJwksFetcher {
        body: Err(JwksFetchError::ResponseTooLarge),
        call_count: Arc::new(AtomicUsize::new(0)),
    };
    let issuer = "https://id.example";
    let source = ProductionJwksSource::new(vec![make_config(issuer)], fetcher).unwrap();

    assert!(source.get_snapshot(issuer).await.is_none());
}

#[tokio::test]
async fn get_snapshot_returns_none_on_parse_error() {
    let fetcher = FakeJwksFetcher {
        body: Err(JwksFetchError::ParseError),
        call_count: Arc::new(AtomicUsize::new(0)),
    };
    let issuer = "https://id.example";
    let source = ProductionJwksSource::new(vec![make_config(issuer)], fetcher).unwrap();

    assert!(source.get_snapshot(issuer).await.is_none());
}

#[tokio::test]
async fn parse_and_bound_rejects_empty_key_set() {
    let err = parse_and_bound_jwks(r#"{"keys":[]}"#).unwrap_err();
    assert_eq!(err, JwksFetchError::KeyCountBoundsViolation);
}

#[tokio::test]
async fn parse_and_bound_rejects_oversized_key_set() {
    let keys: Vec<String> = (0..=MAX_JWKS_KEYS)
        .map(|i| format!(
            r#"{{"kty":"EC","crv":"P-256","x":"f83OJ3D2xF1Bg8vub9tLe1gHMzV76e8Tus9uPHvRVEU","y":"x_FEzRu9m36HLN_tue659LNpXW6pCyStikYjKIWI5a0","kid":"k{i}"}}"#
        ))
        .collect();
    let body = format!(r#"{{"keys":[{}]}}"#, keys.join(","));
    assert_eq!(
        parse_and_bound_jwks(&body).unwrap_err(),
        JwksFetchError::KeyCountBoundsViolation
    );
}

#[tokio::test]
async fn new_rejects_empty_configs() {
    let fetcher = FakeJwksFetcher {
        body: Ok(minimal_jwks_json("k1")),
        call_count: Arc::new(AtomicUsize::new(0)),
    };
    assert!(ProductionJwksSource::new(vec![], fetcher).is_none());
}

/// Timing validation is now performed by `JwksSourceContract::new`. These
/// tests verify the contract constructor rejects bad timing, since an invalid
/// contract prevents building an `IssuerJwksConfig` entirely.
#[test]
fn contract_rejects_refresh_ge_hard_deadline() {
    assert!(JwksSourceContract::new(
        "https://id.example/.well-known/jwks.json".to_owned(),
        3600,
        3600,
    )
    .is_none());
}

#[test]
fn contract_rejects_zero_refresh_interval() {
    assert!(JwksSourceContract::new(
        "https://id.example/.well-known/jwks.json".to_owned(),
        0,
        3600,
    )
    .is_none());
}

#[test]
fn contract_rejects_timing_above_maximum() {
    assert!(JwksSourceContract::new(
        "https://id.example/.well-known/jwks.json".to_owned(),
        MAX_JWKS_TIMING_SECONDS + 1,
        MAX_JWKS_TIMING_SECONDS + 2,
    )
    .is_none());
}

#[tokio::test]
async fn new_rejects_duplicate_issuer() {
    let fetcher = FakeJwksFetcher {
        body: Ok(minimal_jwks_json("k1")),
        call_count: Arc::new(AtomicUsize::new(0)),
    };
    let issuer = "https://id.example";
    let config_a = make_config(issuer);
    let config_b = IssuerJwksConfig {
        issuer: issuer.to_owned(),
        contract: JwksSourceContract::new(
            "https://id.example/.well-known/jwks-alt.json".to_owned(),
            600,
            7200,
        )
        .unwrap(),
    };
    assert!(ProductionJwksSource::new(vec![config_a, config_b], fetcher).is_none());
}

/// URI validation is now performed by `JwksSourceContract::new`; an invalid
/// URI makes the contract `None` and prevents an `IssuerJwksConfig` from being
/// built at all. The tests below verify that `JwksSourceContract::new` rejects
/// the same invalid URIs that `ProductionJwksSource::new` previously checked.
#[test]
fn contract_rejects_non_https_jwks_uri() {
    assert!(make_config_with_uri(
        "https://id.example",
        "http://id.example/.well-known/jwks.json"
    )
    .is_none());
}

#[test]
fn contract_rejects_loopback_jwks_uri() {
    assert!(make_config_with_uri(
        "https://id.example",
        "https://127.0.0.1/.well-known/jwks.json"
    )
    .is_none());
}

#[test]
fn contract_rejects_private_ip_jwks_uri() {
    assert!(make_config_with_uri(
        "https://id.example",
        "https://10.0.0.1/.well-known/jwks.json"
    )
    .is_none());
}

#[test]
fn contract_rejects_jwks_uri_with_credentials() {
    assert!(make_config_with_uri(
        "https://id.example",
        "https://user:pass@id.example/.well-known/jwks.json"
    )
    .is_none());
}

#[test]
fn contract_rejects_jwks_uri_with_fragment() {
    assert!(make_config_with_uri(
        "https://id.example",
        "https://id.example/.well-known/jwks.json#keys"
    )
    .is_none());
}

/// `key_set()` fails closed (returns `None`) before any snapshot is warmed via
/// `get_snapshot` — the synchronous path never fetches.
#[tokio::test]
async fn sync_key_set_returns_none_before_warmup() {
    let fetcher = FakeJwksFetcher {
        body: Ok(minimal_jwks_json("k1")),
        call_count: Arc::new(AtomicUsize::new(0)),
    };
    let issuer = "https://id.example";
    let source = ProductionJwksSource::new(vec![make_config(issuer)], fetcher).unwrap();

    use crate::nip_fi::verifier::IssuerKeySource;
    assert!(source.key_set(issuer).is_none());
}

#[tokio::test]
async fn sync_key_set_returns_snapshot_after_warmup() {
    let fetcher = FakeJwksFetcher {
        body: Ok(minimal_jwks_json("k1")),
        call_count: Arc::new(AtomicUsize::new(0)),
    };
    let issuer = "https://id.example";
    let source = ProductionJwksSource::new(vec![make_config(issuer)], fetcher).unwrap();

    source.get_snapshot(issuer).await.unwrap();

    use crate::nip_fi::verifier::IssuerKeySource;
    let ks = source.key_set(issuer).unwrap();
    assert_eq!(ks.issuer(), issuer);
}

/// Identical document fetched twice must not advance the generation counter
/// — stable generation for unchanged JWKS prevents spurious revalidation.
#[tokio::test]
async fn generation_stable_for_identical_document() {
    let issuer = "https://id.example";
    let fetcher = FakeJwksFetcher {
        body: Ok(minimal_jwks_json("k1")),
        call_count: Arc::new(AtomicUsize::new(0)),
    };
    let config = IssuerJwksConfig {
        issuer: issuer.to_owned(),
        contract: JwksSourceContract::new(
            format!("https://{issuer}/.well-known/jwks.json"),
            1,
            3600,
        )
        .unwrap(),
    };
    let source = ProductionJwksSource::new(vec![config], fetcher).unwrap();

    use crate::nip_fi::verifier::IssuerKeySource;
    source.get_snapshot(issuer).await.unwrap();
    let gen1 = source.key_set(issuer).unwrap().generation();

    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    source.get_snapshot(issuer).await.unwrap();
    let gen2 = source.key_set(issuer).unwrap().generation();

    assert_eq!(gen1, gen2);
}

/// Changed document must advance the generation so key-rotation events are
/// visible [FI-TRACE-JWKS-ADD/REMOVE].
#[tokio::test]
async fn generation_advances_for_changed_document() {
    let issuer = "https://id.example";

    let bodies = Arc::new(std::sync::Mutex::new(vec![
        Ok::<String, JwksFetchError>(minimal_jwks_json("k2")),
        Ok(minimal_jwks_json("k1")),
    ]));

    struct MultiBodyFetcher {
        bodies: Arc<std::sync::Mutex<Vec<Result<String, JwksFetchError>>>>,
    }
    impl super::super::verifier::sealed::Sealed for MultiBodyFetcher {}
    impl JwksFetcher for MultiBodyFetcher {
        fn fetch_jwks<'a>(
            &'a self,
            _uri: &'a str,
        ) -> impl std::future::Future<Output = Result<String, JwksFetchError>> + Send + 'a {
            let result = self
                .bodies
                .lock()
                .unwrap()
                .pop()
                .unwrap_or(Err(JwksFetchError::NetworkError));
            async move { result }
        }
    }

    let config = IssuerJwksConfig {
        issuer: issuer.to_owned(),
        contract: JwksSourceContract::new(
            format!("https://{issuer}/.well-known/jwks.json"),
            1,
            3600,
        )
        .unwrap(),
    };
    let source = ProductionJwksSource::new(vec![config], MultiBodyFetcher { bodies }).unwrap();

    use crate::nip_fi::verifier::IssuerKeySource;
    source.get_snapshot(issuer).await.unwrap();
    let gen1 = source.key_set(issuer).unwrap().generation();

    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    source.get_snapshot(issuer).await.unwrap();
    let gen2 = source.key_set(issuer).unwrap().generation();

    assert!(gen2 > gen1, "gen1={gen1}, gen2={gen2}");
}

#[test]
fn validate_uri_accepts_valid_https() {
    assert!(validate_jwks_uri("https://id.example/.well-known/jwks.json").is_ok());
}

#[test]
fn validate_uri_accepts_public_ipv6() {
    assert!(validate_jwks_uri("https://[2606:4700::1]/.well-known/jwks.json").is_ok());
}

#[test]
fn validate_uri_rejects_http() {
    assert_eq!(
        validate_jwks_uri("http://id.example/.well-known/jwks.json").unwrap_err(),
        JwksFetchError::InvalidUri
    );
}

#[test]
fn validate_uri_rejects_loopback_ip() {
    assert_eq!(
        validate_jwks_uri("https://127.0.0.1/jwks.json").unwrap_err(),
        JwksFetchError::InvalidUri
    );
}

#[test]
fn validate_uri_rejects_private_ip() {
    assert_eq!(
        validate_jwks_uri("https://192.168.1.1/jwks.json").unwrap_err(),
        JwksFetchError::InvalidUri
    );
}

#[test]
fn validate_uri_rejects_link_local_ip() {
    assert_eq!(
        validate_jwks_uri("https://169.254.169.254/jwks.json").unwrap_err(),
        JwksFetchError::InvalidUri
    );
}

#[test]
fn validate_uri_rejects_documentation_ip_test_net_1() {
    // 192.0.2.0/24 — RFC 5737 TEST-NET-1, never globally routed.
    assert_eq!(
        validate_jwks_uri("https://192.0.2.1/jwks.json").unwrap_err(),
        JwksFetchError::InvalidUri
    );
}

#[test]
fn validate_uri_rejects_documentation_ip_test_net_2() {
    // 198.51.100.0/24 — RFC 5737 TEST-NET-2.
    assert_eq!(
        validate_jwks_uri("https://198.51.100.1/jwks.json").unwrap_err(),
        JwksFetchError::InvalidUri
    );
}

#[test]
fn validate_uri_rejects_documentation_ip_test_net_3() {
    // 203.0.113.0/24 — RFC 5737 TEST-NET-3.
    assert_eq!(
        validate_jwks_uri("https://203.0.113.1/jwks.json").unwrap_err(),
        JwksFetchError::InvalidUri
    );
}

#[test]
fn validate_uri_rejects_multicast_ip() {
    // 224.0.0.1 — all-hosts multicast group (224.0.0.0/4).
    assert_eq!(
        validate_jwks_uri("https://224.0.0.1/jwks.json").unwrap_err(),
        JwksFetchError::InvalidUri
    );
}

#[test]
fn validate_uri_rejects_reserved_class_e_ip() {
    // 240.0.0.1 — reserved class E (240.0.0.0/4).
    assert_eq!(
        validate_jwks_uri("https://240.0.0.1/jwks.json").unwrap_err(),
        JwksFetchError::InvalidUri
    );
}

#[test]
fn validate_uri_rejects_ietf_protocol_assignments_ipv4() {
    // 192.0.0.0/24 — IETF Protocol Assignments (non-global by default).
    // 192.0.0.1 is a representative interior address.
    assert_eq!(
        validate_jwks_uri("https://192.0.0.1/jwks.json").unwrap_err(),
        JwksFetchError::InvalidUri
    );
}

#[test]
fn validate_uri_accepts_ietf_protocol_assignments_pcp_turn_anycast() {
    // 192.0.0.9 (PCP anycast, RFC 7723) and 192.0.0.10 (TURN anycast, RFC 8155)
    // are the only globally-reachable exceptions inside 192.0.0.0/24.
    assert!(validate_jwks_uri("https://192.0.0.9/jwks.json").is_ok());
    assert!(validate_jwks_uri("https://192.0.0.10/jwks.json").is_ok());
}

#[test]
fn validate_uri_rejects_deprecated_6to4_anycast_ipv4() {
    // 192.88.99.0/24 — deprecated 6to4 relay anycast (RFC 7526).
    // Registry global field is None/blank; conservative posture: block.
    assert_eq!(
        validate_jwks_uri("https://192.88.99.1/jwks.json").unwrap_err(),
        JwksFetchError::InvalidUri
    );
}

#[test]
fn validate_uri_rejects_ietf_protocol_assignments_v6_interior() {
    // 2001:2::1 — interior of 2001::/23 IETF Protocol Assignments (non-global).
    assert_eq!(
        validate_jwks_uri("https://[2001:2::1]/jwks.json").unwrap_err(),
        JwksFetchError::InvalidUri
    );
}

#[test]
fn validate_uri_accepts_ietf_protocol_assignments_v6_global_exception() {
    // 2001:1::1 (PCP anycast, RFC 7723) — globally reachable exception inside 2001::/23.
    assert!(validate_jwks_uri("https://[2001:1::1]/jwks.json").is_ok());
}

#[test]
fn validate_uri_rejects_discard_only_v6() {
    // 100::1 — 100::/64 Discard-Only address space (RFC 6666).
    assert_eq!(
        validate_jwks_uri("https://[100::1]/jwks.json").unwrap_err(),
        JwksFetchError::InvalidUri
    );
}

#[test]
fn validate_uri_rejects_documentation_v6_3fff() {
    // 3fff::1 — 3fff::/20 Documentation space (RFC 9637).
    assert_eq!(
        validate_jwks_uri("https://[3fff::1]/jwks.json").unwrap_err(),
        JwksFetchError::InvalidUri
    );
}

#[test]
fn validate_uri_rejects_srv6_sids_v6() {
    // 5f00::1 — 5f00::/16 SRv6 SID space (RFC 9252).
    assert_eq!(
        validate_jwks_uri("https://[5f00::1]/jwks.json").unwrap_err(),
        JwksFetchError::InvalidUri
    );
}

#[test]
fn validate_uri_rejects_credentials() {
    assert_eq!(
        validate_jwks_uri("https://user:pass@id.example/jwks.json").unwrap_err(),
        JwksFetchError::InvalidUri
    );
}

#[test]
fn validate_uri_rejects_fragment() {
    assert_eq!(
        validate_jwks_uri("https://id.example/jwks.json#section").unwrap_err(),
        JwksFetchError::InvalidUri
    );
}

#[test]
fn validate_uri_rejects_unparseable() {
    assert_eq!(
        validate_jwks_uri("not a url").unwrap_err(),
        JwksFetchError::InvalidUri
    );
}

#[tokio::test]
async fn http_fetcher_rejects_http_uri_before_connection() {
    let fetcher = HttpJwksFetcher::new();
    let err = fetcher
        .fetch_jwks("http://id.example/.well-known/jwks.json")
        .await
        .unwrap_err();
    assert_eq!(err, JwksFetchError::InvalidUri);
}

#[tokio::test]
async fn http_fetcher_rejects_credentials_uri_before_connection() {
    let fetcher = HttpJwksFetcher::new();
    let err = fetcher
        .fetch_jwks("https://user:pass@id.example/.well-known/jwks.json")
        .await
        .unwrap_err();
    assert_eq!(err, JwksFetchError::InvalidUri);
}

#[tokio::test]
async fn http_fetcher_rejects_fragment_uri_before_connection() {
    let fetcher = HttpJwksFetcher::new();
    let err = fetcher
        .fetch_jwks("https://id.example/.well-known/jwks.json#section")
        .await
        .unwrap_err();
    assert_eq!(err, JwksFetchError::InvalidUri);
}

#[tokio::test]
async fn http_fetcher_rejects_private_ip_uri_before_connection() {
    let fetcher = HttpJwksFetcher::new();
    let err = fetcher
        .fetch_jwks("https://10.0.0.1/.well-known/jwks.json")
        .await
        .unwrap_err();
    assert_eq!(err, JwksFetchError::InvalidUri);
}

#[tokio::test]
async fn resolve_ssrf_rejects_ipv6_loopback_fast_path() {
    let err = super::resolve_and_check_ssrf("::1", 443).await.unwrap_err();
    assert_eq!(err, JwksFetchError::InvalidUri);
}

#[tokio::test]
async fn resolve_ssrf_accepts_public_ipv6_fast_path() {
    let ip = super::resolve_and_check_ssrf("2606:4700::1", 443)
        .await
        .unwrap();
    assert_eq!(ip, "2606:4700::1".parse::<std::net::IpAddr>().unwrap());
}

/// The public fetcher rejects an IPv6 loopback JWKS URI before any network
/// connection is attempted. `fetch_jwks_inner` calls `validate_jwks_uri` as
/// its first step; `validate_jwks_uri` parses the URI, extracts the host via
/// `Url::host()`, and rejects any address matched by the shared enumerated
/// deny policy as
/// `InvalidUri`. `::1` (loopback) never reaches the extraction or
/// resolved-target enforcement stages. Bracket-free extraction and
/// resolved-target value-flow evidence is covered by the dedicated
/// `resolved_target_and_pin_key_seam_public_ipv6_and_fec0_rejection` test;
/// connector-boundary behavior is a separate runtime concern.
#[tokio::test]
async fn http_fetcher_rejects_ipv6_loopback_uri_as_invalid() {
    // https://[::1]/... is rejected by validate_jwks_uri (SSRF: loopback)
    // before extraction or resolved-target enforcement runs.
    let fetcher = HttpJwksFetcher::new();
    let err = fetcher
        .fetch_jwks("https://[::1]/.well-known/jwks.json")
        .await
        .unwrap_err();
    assert_eq!(
        err,
        JwksFetchError::InvalidUri,
        "IPv6 loopback URI must be rejected as InvalidUri, not NetworkError"
    );
}

/// Rejected private IPv6 site-local URI at the pre-connection SSRF boundary.
/// fec0::/10 (deprecated site-local, RFC 3879) must deny as InvalidUri.
#[tokio::test]
async fn http_fetcher_rejects_ipv6_site_local_uri_before_connection() {
    let fetcher = HttpJwksFetcher::new();
    let err = fetcher
        .fetch_jwks("https://[fec0::1]/.well-known/jwks.json")
        .await
        .unwrap_err();
    assert_eq!(err, JwksFetchError::InvalidUri);
}

/// `with_deadline` fires before the outer guard: removing `tokio::time::timeout`
/// inside `with_deadline` leaves the pending future unresolved and the outer guard fires.
#[tokio::test(start_paused = true)]
async fn with_deadline_fires_before_outer_guard() {
    let inner = super::with_deadline(
        std::future::pending::<Result<String, JwksFetchError>>(),
        std::time::Duration::ZERO,
    );
    let result = tokio::time::timeout(std::time::Duration::from_secs(1), inner).await;
    assert_eq!(
        result.expect("outer guard fired — with_deadline timeout seam missing"),
        Err(JwksFetchError::NetworkError),
    );
}

// A fetcher whose per-call behaviour is scripted by an explicit sequence of steps.
// Each call pops the next step: signals `entered` on entry, then blocks until
// its release channel resolves.
struct FetchStep {
    entered: tokio::sync::oneshot::Sender<()>,
    release: tokio::sync::oneshot::Receiver<String>,
}

struct ScriptedFetcher {
    steps: std::sync::Mutex<std::collections::VecDeque<FetchStep>>,
    call_count: Arc<AtomicUsize>,
}

impl super::super::verifier::sealed::Sealed for ScriptedFetcher {}

impl JwksFetcher for ScriptedFetcher {
    fn fetch_jwks<'a>(
        &'a self,
        _uri: &'a str,
    ) -> impl std::future::Future<Output = Result<String, JwksFetchError>> + Send + 'a {
        self.call_count.fetch_add(1, Ordering::SeqCst);
        let step = self.steps.lock().unwrap().pop_front();
        async move {
            match step {
                Some(FetchStep { entered, release }) => {
                    let _ = entered.send(());
                    release.await.map_err(|_| JwksFetchError::NetworkError)
                }
                None => Err(JwksFetchError::NetworkError),
            }
        }
    }
}

fn script(steps: impl IntoIterator<Item = FetchStep>) -> ScriptedFetcher {
    ScriptedFetcher {
        steps: std::sync::Mutex::new(steps.into_iter().collect()),
        call_count: Arc::new(AtomicUsize::new(0)),
    }
}

fn pending_step() -> (
    FetchStep,
    tokio::sync::oneshot::Receiver<()>,
    tokio::sync::oneshot::Sender<String>,
) {
    let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = tokio::sync::oneshot::channel::<String>();
    // release_tx is returned to the caller; the fetch future is genuinely
    // pending until the caller drops or sends it — not resolved immediately.
    (
        FetchStep {
            entered: entered_tx,
            release: release_rx,
        },
        entered_rx,
        release_tx,
    )
}

fn ready_step(body: String) -> (FetchStep, tokio::sync::oneshot::Receiver<()>) {
    let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = tokio::sync::oneshot::channel::<String>();
    let _ = release_tx.send(body);
    (
        FetchStep {
            entered: entered_tx,
            release: release_rx,
        },
        entered_rx,
    )
}

fn blocking_step() -> (
    FetchStep,
    tokio::sync::oneshot::Receiver<()>,
    tokio::sync::oneshot::Sender<String>,
) {
    let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = tokio::sync::oneshot::channel::<String>();
    (
        FetchStep {
            entered: entered_tx,
            release: release_rx,
        },
        entered_rx,
        release_tx,
    )
}

/// A second concurrent `get_snapshot` while the first fetch is in progress must
/// not start a second fetch — the RAII permit coalesces callers.
#[tokio::test]
async fn concurrent_refresh_coalesces_without_second_fetch() {
    let (step, entered_rx, release_tx) = blocking_step();
    let fetcher = script([step]);
    let call_count = Arc::clone(&fetcher.call_count);

    let issuer = "https://id.example";
    let source = Arc::new(ProductionJwksSource::new(vec![make_config(issuer)], fetcher).unwrap());

    let source2 = Arc::clone(&source);
    let issuer_owned = issuer.to_owned();
    let first = tokio::spawn(async move { source2.get_snapshot(&issuer_owned).await });

    entered_rx.await.unwrap(); // first fetch holds the permit

    let second_result = source.get_snapshot(issuer).await;
    let count_after_second = call_count.load(Ordering::SeqCst);

    let _ = release_tx.send(minimal_jwks_json("k1"));
    let first_result = first.await.unwrap();

    assert!(first_result.is_some());
    assert!(second_result.is_none());
    assert_eq!(count_after_second, 1);
}

/// Aborting the first caller releases the RAII permit; the next call on the same
/// source fetches and succeeds. A manual boolean cleared only on success would
/// leave the permit poisoned.
#[tokio::test]
async fn aborted_first_caller_releases_permit_for_next_caller() {
    let (step1, entered_rx_1, _release_tx_1) = pending_step();
    let (step2, _entered_rx_2) = ready_step(minimal_jwks_json("k2"));

    let fetcher = script([step1, step2]);
    let call_count = Arc::clone(&fetcher.call_count);

    let issuer = "https://id.example";
    let source = Arc::new(ProductionJwksSource::new(vec![make_config(issuer)], fetcher).unwrap());

    {
        let source2 = Arc::clone(&source);
        let issuer_owned = issuer.to_owned();
        let first = tokio::spawn(async move { source2.get_snapshot(&issuer_owned).await });
        entered_rx_1.await.unwrap();
        first.abort();
        let _ = first.await;
        // _release_tx_1 drops here: the fetch future was blocked on an open
        // receiver when abort fired — not resolved via an error path.
    }

    let result = source.get_snapshot(issuer).await;
    assert!(result.is_some());
    assert_eq!(call_count.load(Ordering::SeqCst), 2);
}

/// An expired snapshot must never be served — both `get_snapshot` and the
/// synchronous `key_set` path return `None` after the hard deadline passes.
#[tokio::test]
async fn expired_snapshot_never_served_after_hard_deadline() {
    let issuer = "https://id.example";
    let config = IssuerJwksConfig {
        issuer: issuer.to_owned(),
        contract: JwksSourceContract::new(
            "https://id.example/.well-known/jwks.json".to_owned(),
            1,
            2,
        )
        .unwrap(),
    };
    let bodies = Arc::new(std::sync::Mutex::new(vec![
        Err::<String, JwksFetchError>(JwksFetchError::NetworkError),
        Ok(minimal_jwks_json("k1")),
    ]));
    struct FailAfterFirstFetcher {
        bodies: Arc<std::sync::Mutex<Vec<Result<String, JwksFetchError>>>>,
    }
    impl super::super::verifier::sealed::Sealed for FailAfterFirstFetcher {}
    impl JwksFetcher for FailAfterFirstFetcher {
        fn fetch_jwks<'a>(
            &'a self,
            _uri: &'a str,
        ) -> impl std::future::Future<Output = Result<String, JwksFetchError>> + Send + 'a {
            let result = self
                .bodies
                .lock()
                .unwrap()
                .pop()
                .unwrap_or(Err(JwksFetchError::NetworkError));
            async move { result }
        }
    }
    let source = ProductionJwksSource::new(vec![config], FailAfterFirstFetcher { bodies }).unwrap();

    assert!(
        source.get_snapshot(issuer).await.is_some(),
        "initial fetch must succeed"
    );

    tokio::time::sleep(std::time::Duration::from_secs(3)).await;

    assert!(
        source.get_snapshot(issuer).await.is_none(),
        "expired snapshot must not be served after hard deadline"
    );

    use crate::nip_fi::verifier::IssuerKeySource;
    assert!(
        source.key_set(issuer).is_none(),
        "key_set must not serve an expired snapshot"
    );
}

/// Two issuers are fully isolated: distinct key material, independent generation
/// counters, no cross-issuer forgery. Three distinct P-256 keypairs (A1, A2,
/// B1) driven through `ProductionJwksSource` into `FederatedAssertionVerifier`.
#[tokio::test]
async fn two_issuer_keys_and_generations_are_isolated() {
    use crate::nip_fi::{
        FederatedAssertionVerifier, FreshnessClass, IssuerPolicy, IssuerRegistry, TokenClass,
    };
    use jsonwebtoken::{Algorithm, EncodingKey, Header};
    use serde_json::json;

    // Three genuinely distinct P-256 keypairs (PKCS#8 PEM + public JWK coords).
    const PKCS8_A1: &str = "-----BEGIN PRIVATE KEY-----\n\
        MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgcnxDM4EiirH9dHUE\n\
        WZc759TX4s5PAn8kO5ovXSnGxCWhRANCAARFb6ZnsfkqOOXyEhj3KBQphGKF4vTa\n\
        zhebbavbZ1ZoklqkF1cGg+jTO7rONAVEzXvXUWtV6CdDV+rybiVmFP2w\n\
        -----END PRIVATE KEY-----\n";
    const X_A1: &str = "RW-mZ7H5Kjjl8hIY9ygUKYRiheL02s4Xm22r22dWaJI";
    const Y_A1: &str = "WqQXVwaD6NM7us40BUTNe9dRa1XoJ0NX6vJuJWYU_bA";

    const PKCS8_A2: &str = "-----BEGIN PRIVATE KEY-----\n\
        MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgMKMRn6EQMn67Z6tu\n\
        DbUTZWzrQpbRRTL3SJSMSd+EDG2hRANCAATGgMYxftLlZ11AIANHcr0b13pWkaLy\n\
        lkOeBZRG0bBMoUesLN7EdVYhtzcrCeNJh031QuO+UDWcwOmShbeR43x6\n\
        -----END PRIVATE KEY-----\n";
    const X_A2: &str = "xoDGMX7S5WddQCADR3K9G9d6VpGi8pZDngWURtGwTKE";
    const Y_A2: &str = "R6ws3sR1ViG3NysJ40mHTfVC475QNZzA6ZKFt5HjfHo";

    const PKCS8_B1: &str = "-----BEGIN PRIVATE KEY-----\n\
        MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgKcmDf3+zDWyC96/X\n\
        Gv8aYK552uF5aE6nXKzxAfl4fSWhRANCAATf0ccbp1c4mMd6WvSuliv5ZAS8iIWL\n\
        Ne2tqOfFa0hRpa41DANab1/EuDGi7PtIo8xSYwkaoib1MAJlfLvRMjQA\n\
        -----END PRIVATE KEY-----\n";
    const X_B1: &str = "39HHG6dXOJjHelr0rpYr-WQEvIiFizXtrajnxWtIUaU";
    const Y_B1: &str = "rjUMA1pvX8S4MaLs-0ijzFJjCRqiJvUwAmV8u9EyNAA";

    const KID_A1: &str = "a-key-1";
    const KID_A2: &str = "a-key-2";
    const KID_B1: &str = "b-key-1";

    let issuer_a = "https://a.example";
    let issuer_b = "https://b.example";
    let audience = "https://relay.example";

    fn jwks_str(kid: &str, x: &str, y: &str) -> String {
        format!(
            r#"{{"keys":[{{"kty":"EC","crv":"P-256","use":"sig","alg":"ES256","kid":"{kid}","x":"{x}","y":"{y}"}}]}}"#
        )
    }

    fn sign(pkcs8_pem: &str, kid: &str, iss: &str, aud: &str) -> String {
        let now = chrono::Utc::now().timestamp();
        // nostr_pubkey is required unconditionally by spec v2.
        let claims = json!({"iss": iss, "aud": aud, "sub": "u",
                            "nostr_pubkey": "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef",
                            "iat": now, "exp": now + 600});
        let mut hdr = Header::new(Algorithm::ES256);
        hdr.kid = Some(kid.to_owned());
        hdr.typ = Some("nip-fi+jwt".to_owned());
        let key = EncodingKey::from_ec_pem(pkcs8_pem.as_bytes()).expect("valid EC PEM");
        jsonwebtoken::encode(&hdr, &claims, &key).expect("sign")
    }

    fn policy(issuer: &str, aud: &str) -> IssuerPolicy {
        let contract = JwksSourceContract::new(
            format!(
                "https://{}/jwks.json",
                issuer.trim_start_matches("https://")
            ),
            1,
            3600,
        )
        .expect("valid contract");
        IssuerPolicy::new(
            issuer.to_owned(),
            vec![aud.to_owned()],
            TokenClass::DedicatedNipFi,
            FreshnessClass::OfflineJwt,
            vec![Algorithm::ES256],
            60,
            3600,
            None,
            contract,
        )
        .expect("valid policy")
    }

    fn configs(issuer_a: &str, issuer_b: &str) -> (IssuerJwksConfig, IssuerJwksConfig) {
        (
            IssuerJwksConfig {
                issuer: issuer_a.to_owned(),
                contract: JwksSourceContract::new(
                    "https://a.example/.well-known/jwks.json".to_owned(),
                    1,
                    3600,
                )
                .unwrap(),
            },
            IssuerJwksConfig {
                issuer: issuer_b.to_owned(),
                contract: JwksSourceContract::new(
                    "https://b.example/.well-known/jwks.json".to_owned(),
                    1,
                    3600,
                )
                .unwrap(),
            },
        )
    }

    struct TwoFetcher {
        a: std::sync::Mutex<std::collections::VecDeque<String>>,
        b: String,
    }
    impl super::super::verifier::sealed::Sealed for TwoFetcher {}
    impl JwksFetcher for TwoFetcher {
        fn fetch_jwks<'a>(
            &'a self,
            uri: &'a str,
        ) -> impl std::future::Future<Output = Result<String, JwksFetchError>> + Send + 'a {
            let result = if uri.contains("a.example") {
                self.a
                    .lock()
                    .unwrap()
                    .pop_front()
                    .map(Ok)
                    .unwrap_or(Err(JwksFetchError::NetworkError))
            } else {
                Ok(self.b.clone())
            };
            async move { result }
        }
    }

    let mut registry = IssuerRegistry::new();
    registry.insert(policy(issuer_a, audience));
    registry.insert(policy(issuer_b, audience));

    // Pre-rotation: source serves A1 and B1.
    let (cfg_a, cfg_b) = configs(issuer_a, issuer_b);
    let pre = ProductionJwksSource::new(
        vec![cfg_a, cfg_b],
        TwoFetcher {
            a: std::sync::Mutex::new([jwks_str(KID_A1, X_A1, Y_A1)].into()),
            b: jwks_str(KID_B1, X_B1, Y_B1),
        },
    )
    .unwrap();
    pre.get_snapshot(issuer_a).await.unwrap();
    pre.get_snapshot(issuer_b).await.unwrap();

    let v_pre = FederatedAssertionVerifier::new(registry.clone(), pre);
    v_pre
        .verify(&sign(PKCS8_A1, KID_A1, issuer_a, audience))
        .expect("A1 token must verify pre-rotation");
    v_pre
        .verify(&sign(PKCS8_B1, KID_B1, issuer_b, audience))
        .expect("B1 token must verify pre-rotation");
    v_pre
        .verify(&sign(PKCS8_B1, KID_A1, issuer_a, audience))
        .expect_err("B1 key must not forge issuer A");

    // Post-rotation: fresh source, A rotates A1→A2, B unchanged.
    let (cfg_a2, cfg_b2) = configs(issuer_a, issuer_b);
    let post = ProductionJwksSource::new(
        vec![cfg_a2, cfg_b2],
        TwoFetcher {
            a: std::sync::Mutex::new(
                [jwks_str(KID_A1, X_A1, Y_A1), jwks_str(KID_A2, X_A2, Y_A2)].into(),
            ),
            b: jwks_str(KID_B1, X_B1, Y_B1),
        },
    )
    .unwrap();
    post.get_snapshot(issuer_a).await.unwrap();
    post.get_snapshot(issuer_b).await.unwrap();

    use crate::nip_fi::verifier::IssuerKeySource;
    let gen_a_pre = post.key_set(issuer_a).unwrap().generation();
    let gen_b_stable = post.key_set(issuer_b).unwrap().generation();

    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    post.get_snapshot(issuer_a).await.unwrap();

    let gen_a_post = post.key_set(issuer_a).unwrap().generation();
    let gen_b_post = post.key_set(issuer_b).unwrap().generation();
    assert!(
        gen_a_post > gen_a_pre,
        "A generation must advance after rotation"
    );
    assert_eq!(
        gen_b_post, gen_b_stable,
        "B generation must not advance when only A rotates"
    );

    let v_post = FederatedAssertionVerifier::new(registry, post);
    v_post
        .verify(&sign(PKCS8_A2, KID_A2, issuer_a, audience))
        .expect("A2 token must verify post-rotation");
    v_post
        .verify(&sign(PKCS8_A1, KID_A1, issuer_a, audience))
        .expect_err("old A1 token must fail after A2 rotation");
    v_post
        .verify(&sign(PKCS8_B1, KID_A1, issuer_a, audience))
        .expect_err("B1 key must not forge issuer A post-rotation");
    v_post
        .verify(&sign(PKCS8_B1, KID_B1, issuer_b, audience))
        .expect("B1 token must still verify post-rotation");
}

/// Public-API regression: one long-lived [`FederatedAssertionVerifier`] backed
/// by a shared `Arc<ProductionJwksSource>` observes key rotation through the
/// same cache it was constructed with — it does NOT need to be rebuilt when
/// keys rotate.
///
/// Scenario:
///  A1  →  initial key set (generation 1)
///  A2  →  rotated key set (generation 2, committed after a refresh interval)
///
/// The verifier is constructed once before A2 is known, then the source is
/// refreshed in-place (simulating a normal JWKS rotation). The same verifier
/// must then reject A1-signed tokens and accept A2-signed tokens, because it
/// reads from the shared cache.
///
/// Mutation (correctness): change `Arc<ProductionJwksSource>` to a plain
/// `ProductionJwksSource` (no sharing). The verifier would hold its own
/// copy of the pre-rotation cache and could not observe the refresh. A2 tokens
/// would fail and A1 tokens would pass — the test turns red on both assertions.
#[tokio::test]
async fn shared_arc_source_verifier_observes_rotation() {
    use crate::nip_fi::{
        FederatedAssertionVerifier, FreshnessClass, IssuerPolicy, IssuerRegistry, TokenClass,
    };
    use jsonwebtoken::{Algorithm, EncodingKey, Header};
    use serde_json::json;
    use std::sync::Arc;

    // Two genuinely distinct P-256 keypairs (re-use the constants from the
    // two-issuer test).
    const PKCS8_A1: &str = "-----BEGIN PRIVATE KEY-----\n\
        MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgcnxDM4EiirH9dHUE\n\
        WZc759TX4s5PAn8kO5ovXSnGxCWhRANCAARFb6ZnsfkqOOXyEhj3KBQphGKF4vTa\n\
        zhebbavbZ1ZoklqkF1cGg+jTO7rONAVEzXvXUWtV6CdDV+rybiVmFP2w\n\
        -----END PRIVATE KEY-----\n";
    const X_A1: &str = "RW-mZ7H5Kjjl8hIY9ygUKYRiheL02s4Xm22r22dWaJI";
    const Y_A1: &str = "WqQXVwaD6NM7us40BUTNe9dRa1XoJ0NX6vJuJWYU_bA";

    const PKCS8_A2: &str = "-----BEGIN PRIVATE KEY-----\n\
        MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgMKMRn6EQMn67Z6tu\n\
        DbUTZWzrQpbRRTL3SJSMSd+EDG2hRANCAATGgMYxftLlZ11AIANHcr0b13pWkaLy\n\
        lkOeBZRG0bBMoUesLN7EdVYhtzcrCeNJh031QuO+UDWcwOmShbeR43x6\n\
        -----END PRIVATE KEY-----\n";
    const X_A2: &str = "xoDGMX7S5WddQCADR3K9G9d6VpGi8pZDngWURtGwTKE";
    const Y_A2: &str = "R6ws3sR1ViG3NysJ40mHTfVC475QNZzA6ZKFt5HjfHo";

    const KID_A1: &str = "arc-key-1";
    const KID_A2: &str = "arc-key-2";

    let issuer = "https://arc-issuer.example";
    let audience = "https://relay.example";

    fn jwks_str(kid: &str, x: &str, y: &str) -> String {
        format!(
            r#"{{"keys":[{{"kty":"EC","crv":"P-256","use":"sig","alg":"ES256","kid":"{kid}","x":"{x}","y":"{y}"}}]}}"#
        )
    }

    fn sign_token(pkcs8_pem: &str, kid: &str, iss: &str, aud: &str) -> String {
        let now = chrono::Utc::now().timestamp();
        // nostr_pubkey is required unconditionally by spec v2.
        let claims = json!({"iss": iss, "aud": aud, "sub": "u",
                            "nostr_pubkey": "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef",
                            "iat": now, "exp": now + 600});
        let mut hdr = Header::new(Algorithm::ES256);
        hdr.kid = Some(kid.to_owned());
        hdr.typ = Some("nip-fi+jwt".to_owned());
        let key = EncodingKey::from_ec_pem(pkcs8_pem.as_bytes()).expect("valid EC PEM");
        jsonwebtoken::encode(&hdr, &claims, &key).expect("sign")
    }

    // Scripted fetcher: first call returns A1 JWKS, second call returns A2 JWKS.
    let bodies = Arc::new(std::sync::Mutex::new(vec![
        Ok::<String, JwksFetchError>(jwks_str(KID_A2, X_A2, Y_A2)), // popped second
        Ok(jwks_str(KID_A1, X_A1, Y_A1)),                           // popped first
    ]));

    struct RotatingFetcher {
        bodies: Arc<std::sync::Mutex<Vec<Result<String, JwksFetchError>>>>,
    }
    impl super::super::verifier::sealed::Sealed for RotatingFetcher {}
    impl JwksFetcher for RotatingFetcher {
        fn fetch_jwks<'a>(
            &'a self,
            _uri: &'a str,
        ) -> impl std::future::Future<Output = Result<String, JwksFetchError>> + Send + 'a {
            let result = self
                .bodies
                .lock()
                .unwrap()
                .pop()
                .unwrap_or(Err(JwksFetchError::NetworkError));
            async move { result }
        }
    }

    let jwks_contract =
        JwksSourceContract::new(format!("https://{issuer}/.well-known/jwks.json"), 1, 3600)
            .unwrap();
    let config = IssuerJwksConfig {
        issuer: issuer.to_owned(),
        contract: jwks_contract.clone(),
    };

    // Wrap the source in Arc — this is the sharing path under test.
    let source =
        Arc::new(ProductionJwksSource::new(vec![config], RotatingFetcher { bodies }).unwrap());

    // Warm the cache with A1 JWKS.
    source.get_snapshot(issuer).await.unwrap();

    // Build the verifier from an Arc clone. This is the one long-lived
    // verifier we never rebuild.
    let mut registry = IssuerRegistry::new();
    registry.insert(
        IssuerPolicy::new(
            issuer.to_owned(),
            vec![audience.to_owned()],
            TokenClass::DedicatedNipFi,
            FreshnessClass::OfflineJwt,
            vec![Algorithm::ES256],
            60,
            3600,
            None,
            jwks_contract,
        )
        .unwrap(),
    );
    let verifier = FederatedAssertionVerifier::new(registry, Arc::clone(&source));

    // Pre-rotation: A1 token verifies.
    verifier
        .verify(&sign_token(PKCS8_A1, KID_A1, issuer, audience))
        .expect("A1 token must verify before rotation");

    // Advance past the refresh interval so the next get_snapshot triggers a
    // re-fetch (which will return A2 JWKS from the scripted fetcher).
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    source.get_snapshot(issuer).await.unwrap();

    // Post-rotation: the SAME verifier (never rebuilt) must now see A2 keys.
    // This proves the verifier reads from the shared Arc cache, not a
    // snapshot captured at construction time.
    //
    // Mutation: if the verifier held a plain `ProductionJwksSource` (cloned
    // at construction), it would serve the pre-rotation A1 snapshot forever —
    // A2 would fail and A1 would still pass, turning both assertions red.
    verifier
        .verify(&sign_token(PKCS8_A2, KID_A2, issuer, audience))
        .expect("A2 token must verify through the shared Arc after rotation");
    verifier
        .verify(&sign_token(PKCS8_A1, KID_A1, issuer, audience))
        .expect_err("old A1 token must be rejected after rotation (kid no longer in JWKS)");
}

/// **Fix 1 — URI canonicalization convergence/divergence oracle.**
///
/// `JwksSourceContract::new` must store the `Url`-normalized form of the URI,
/// not the caller's raw input bytes. This means:
/// - An uppercase host (`EXAMPLE.COM`) normalizes to lowercase (`example.com`)
///   and produces the same `AssertionPolicyId` as the lowercase form.
/// - An explicit default HTTPS port (`:443`) is removed by `Url` normalization
///   and produces the same ID as the form without the port.
/// - A genuinely different host always produces a distinct ID.
///
/// Mutation (correctness): changing `JwksSourceContract::new` to store the raw
/// input `jwks_uri` instead of `parsed.to_string()` causes the uppercase-host
/// and explicit-port variant tests to fail — the raw bytes differ, the SHA-256
/// hash diverges, and `assert_eq!` on the policy IDs turns red.
#[test]
fn jwks_contract_uri_canonicalization_convergence_and_divergence() {
    use crate::nip_fi::{config::IssuerPolicy, FreshnessClass, TokenClass};
    use jsonwebtoken::Algorithm;

    fn make_policy(jwks_uri: &str) -> Option<crate::nip_fi::AssertionPolicyId> {
        let contract = JwksSourceContract::new(jwks_uri.to_owned(), 300, 3600)?;
        IssuerPolicy::new(
            "https://issuer.example".to_owned(),
            vec!["https://aud.example".to_owned()],
            TokenClass::DedicatedNipFi,
            FreshnessClass::OfflineJwt,
            vec![Algorithm::ES256],
            30,
            600,
            None,
            contract,
        )
        .ok()
        .map(|p| p.id())
    }

    let canonical =
        make_policy("https://issuer.example/.well-known/jwks.json").expect("canonical form");

    // Equivalent spellings must converge after `Url` normalization.
    let uppercase_host =
        make_policy("https://ISSUER.EXAMPLE/.well-known/jwks.json").expect("uppercase host");
    assert_eq!(
        canonical, uppercase_host,
        "uppercase host must normalize to lowercase and produce identical policy ID; \
         mutation: store raw input bytes → this diverges"
    );

    let explicit_port =
        make_policy("https://issuer.example:443/.well-known/jwks.json").expect("explicit port");
    assert_eq!(
        canonical, explicit_port,
        "explicit default HTTPS port :443 must be stripped by Url normalization; \
         mutation: store raw input bytes → this diverges"
    );

    // A genuinely different host MUST diverge (not accidentally collapse).
    let different_host =
        make_policy("https://other.example/.well-known/jwks.json").expect("different host");
    assert_ne!(
        canonical, different_host,
        "different JWKS host must produce distinct policy ID"
    );

    // A different path MUST diverge.
    let different_path =
        make_policy("https://issuer.example/.well-known/other-jwks.json").expect("different path");
    assert_ne!(
        canonical, different_path,
        "different JWKS path must produce distinct policy ID"
    );

    // Dot-segment path that resolves to the same resource MUST converge.
    // `Url::parse` resolves `./jwks.json` relative paths during parsing, so
    // `/.well-known/./jwks.json` normalises to `/.well-known/jwks.json`.
    // Mutation: store raw input bytes -> the dot-segment form remains in the
    // stored URI, the SHA-256 hash diverges, and `assert_eq!` turns red.
    let dot_segment =
        make_policy("https://issuer.example/.well-known/./jwks.json").expect("dot-segment path");
    assert_eq!(
        canonical, dot_segment,
        "dot-segment-equivalent path must normalize and produce identical policy ID; \
         mutation: store raw input bytes -> this diverges"
    );
}

/// **Fix 2 — Public bracketed-IPv6 JWKS URI through the resolved-target and pin-input seam.**
///
/// This seam test is network-free: both public `2606:4700::1` and site-local
/// `fec0::1` are IP literals, so `resolve_and_check_ssrf` takes the fast path
/// (`host.parse::<IpAddr>()` then `is_not_global_unicast`) without any DNS
/// lookup.
///
/// The seam covers the three stages `fetch_jwks_inner` traverses in order:
/// 1. `extract_url_host_and_port` — typed `Url::host()` yields bare
///    `"2606:4700::1"`, not the bracketed `"[2606:4700::1]"` that
///    `host_str()` returns.
/// 2. `resolve_and_check_ssrf(host, port)` — fast path: `host.parse::<IpAddr>()`
///    succeeds only for the bare form, passes `is_not_global_unicast`, and
///    returns the `IpAddr`.
/// 3. Reqwest `.resolve(host, SocketAddr::new(ip, port))` uses the raw `host`
///    string as its pin key. The key must equal the URL authority form —
///    bare for IPv6, brackets forbidden.
///
/// This test proves that the extracted host string is bare (the correct input
/// form for `reqwest::ClientBuilder::resolve`). It does not exercise the
/// reqwest connector; connector-boundary behavior is a runtime concern.
///
/// For `fec0::1`: `extract_url_host_and_port` still extracts the bare address;
/// `resolve_and_check_ssrf` rejects it via `is_not_global_unicast`.
///
/// ## Mutation oracle
/// Replace `Some(url::Host::Ipv6(addr)) => addr.to_string()` with
/// `Some(url::Host::Ipv6(addr)) => format!("[{}]", addr)` in
/// `extract_url_host_and_port`. The bracketed string is returned.
/// - `"[2606:4700::1]".parse::<IpAddr>()` fails → SSRF fast path unreachable
///   → public acceptance assertion flips red.
/// - `is_not_global_unicast` is never called on `fec0::1` (the parse also
///   fails) → `resolve_and_check_ssrf` returns `NetworkError` not `InvalidUri`
///   → fec0 rejection-kind assertion flips red.
/// - The pin-input equality assertion also flips red (bracket mismatch).
#[tokio::test]
async fn resolved_target_and_pin_key_seam_public_ipv6_and_fec0_rejection() {
    use buzz_core::network::is_not_global_unicast;

    // ── Stage 1: extraction ───────────────────────────────────────────────────
    let uri = "https://[2606:4700::1]/.well-known/jwks.json";
    let (host, port) =
        super::extract_url_host_and_port(uri).expect("public IPv6 URI must be parseable");
    assert_eq!(
        host, "2606:4700::1",
        "host must be bare (mutation: bracket → IpAddr::parse fails)"
    );
    assert_eq!(port, 443u16, "default HTTPS port");

    // ── Stage 2: IpAddr resolution (SSRF fast path) ───────────────────────────
    // `host.parse::<IpAddr>()` succeeds only for the bare form. This is exactly
    // the fast path in `resolve_and_check_ssrf` that bypasses DNS.
    let ip: std::net::IpAddr = host
        .parse()
        .expect("bracket-free host must parse as IpAddr; mutation: bracketed form fails here");
    assert!(ip.is_ipv6(), "must be an IPv6 address");

    // `is_not_global_unicast` must return false for a public address.
    assert!(
        !is_not_global_unicast(&ip),
        "2606:4700::1 must pass as globally reachable; mutation: SSRF check would reject it"
    );

    // Confirm resolve_and_check_ssrf accepts the public address (network-free fast path).
    let resolved = super::resolve_and_check_ssrf(&host, port)
        .await
        .expect("public IPv6 must be accepted by SSRF check");
    assert_eq!(
        resolved, ip,
        "resolved address must equal the IpAddr parsed from the bare host"
    );

    // ── Stage 3: pin-key string form ────────────────────────────────────────
    // The host string extracted by `extract_url_host_and_port` is the value
    // passed to reqwest's `.resolve(host, ...)`. For a reqwest pin to apply,
    // the key passed to `.resolve()` must equal the URL authority form. For
    // IPv6 literals the URL authority form is bare (no brackets), so the
    // extracted host must also be bare. This assertion verifies that the
    // extracted host string is bare — it does not directly exercise the
    // reqwest connector, but proves the input to the pin call is correct.
    let socket_addr = std::net::SocketAddr::new(resolved, port);
    let expected_pin_key = "2606:4700::1";
    assert_eq!(
        host, expected_pin_key,
        "extracted host must equal the bare URL authority for use as reqwest pin key; \
         mutation: bracketed extraction returns \"[2606:4700::1]\" (differs from authority form)"
    );
    // Sanity: confirm the SocketAddr is valid (no panic = key formation succeeded).
    let _ = socket_addr;

    // ── fec0::/10 rejection through the same seam ────────────────────────────
    // Stage 1: extraction succeeds (SSRF decision is downstream).
    let fec0_uri = "https://[fec0::1]/.well-known/jwks.json";
    let (fec0_host, fec0_port) =
        super::extract_url_host_and_port(fec0_uri).expect("extraction succeeds for fec0 URI");
    assert_eq!(fec0_host, "fec0::1", "fec0 host must be bare");
    assert_eq!(fec0_port, 443u16);

    // Stage 2: IpAddr parse succeeds for the bare form.
    let fec0_ip: std::net::IpAddr = fec0_host
        .parse()
        .expect("bracket-free fec0 host parses as IpAddr; mutation: bracketed form fails here");

    // is_not_global_unicast must block fec0::/10 (deprecated site-local, RFC 3879).
    assert!(
        is_not_global_unicast(&fec0_ip),
        "fec0::1 must be rejected by is_not_global_unicast; mutation: wrong bracket form \
         bypasses this check (parse fails, NetworkError not InvalidUri)"
    );

    // resolve_and_check_ssrf must return InvalidUri for fec0::1.
    let fec0_err = super::resolve_and_check_ssrf(&fec0_host, fec0_port)
        .await
        .unwrap_err();
    assert_eq!(
        fec0_err,
        JwksFetchError::InvalidUri,
        "fec0::1 must be rejected as InvalidUri, not NetworkError; \
         mutation: bracketed form -> parse fails -> DNS path -> NetworkError (red)"
    );
}

/// **Fix 3 — Unchanged verifier observes A1→A2 rotation beyond A1's original absolute deadline.**
///
/// Uses an injectable clock (`new_with_clock`) to advance controlled `now` past
/// A1's immutable hard deadline without wall-clock sleep. A1's deadline is
/// computed at first-fetch time (T0) and never mutated. The clock then advances
/// to T0 + HARD_DEADLINE_SECS + 1, beyond A1's original absolute deadline.
/// `get_snapshot` fires because the snapshot is expired, fetches A2, and the
/// one unchanged verifier (never rebuilt) must reflect the new keys.
///
/// ## Mutation oracles
/// 1. **Sharing:** Replace `Arc::clone(&source)` passed to the verifier with a
///    fresh `Arc::new(second_source)` built from the same configs but independent,
///    sharing the same controlled clock. Warm the independent source with a
///    separate A1 fetch before advancing the clock. After advancement,
///    `key_set()` on the verifier's independent source filters the expired A1
///    snapshot (`filter(|c| now < c.hard_deadline)`) and returns no keys —
///    the verifier never re-fetches and never observes A2. The A2-accept
///    assertion flips red reliably, because the verifier never observes A2.
///    The A1-reject assertion stays green: the independent cache is also
///    expired (same advanced clock), so that source also returns no A1 keys —
///    A1 tokens are still rejected, but through expiry of the independent
///    cache rather than through shared-arc rotation. **A2 acceptance is the
///    reliable shared-source oracle here.**
///
/// Note: the expiry-purge (`state.snapshot = None` in `get_snapshot`) is
/// correctness-critical for concurrent callers: it clears the expired snapshot
/// before permit acquisition, so a caller that loses the permit race and falls
/// back to `state.snapshot` receives `None` rather than an expired snapshot.
/// A1 rejection after the deadline is also enforced independently by the `key_set`
/// read path (`filter(|c| now < c.hard_deadline)`), but the purge is what
/// prevents the fallback path from serving a stale snapshot to concurrent
/// refresh losers, so no separate purge mutation oracle is claimed here.
#[tokio::test]
async fn shared_arc_source_verifier_rejects_expired_a1_accepts_a2() {
    use crate::nip_fi::{
        FederatedAssertionVerifier, FreshnessClass, IssuerPolicy, IssuerRegistry, TokenClass,
    };
    use jsonwebtoken::{Algorithm, EncodingKey, Header};
    use serde_json::json;
    use std::sync::atomic::{AtomicI64, Ordering};
    use std::sync::Arc;

    // Two distinct P-256 keypairs (reuse constants from shared_arc test).
    const PKCS8_A1: &str = "-----BEGIN PRIVATE KEY-----\n\
        MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgcnxDM4EiirH9dHUE\
        WZc759TX4s5PAn8kO5ovXSnGxCWhRANCAARFb6ZnsfkqOOXyEhj3KBQphGKF4vTa\
        zhebbavbZ1ZoklqkF1cGg+jTO7rONAVEzXvXUWtV6CdDV+rybiVmFP2w\
        \n-----END PRIVATE KEY-----\n";
    const X_A1: &str = "RW-mZ7H5Kjjl8hIY9ygUKYRiheL02s4Xm22r22dWaJI";
    const Y_A1: &str = "WqQXVwaD6NM7us40BUTNe9dRa1XoJ0NX6vJuJWYU_bA";

    const PKCS8_A2: &str = "-----BEGIN PRIVATE KEY-----\n\
        MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgMKMRn6EQMn67Z6tu\
        DbUTZWzrQpbRRTL3SJSMSd+EDG2hRANCAATGgMYxftLlZ11AIANHcr0b13pWkaLy\
        lkOeBZRG0bBMoUesLN7EdVYhtzcrCeNJh031QuO+UDWcwOmShbeR43x6\
        \n-----END PRIVATE KEY-----\n";
    const X_A2: &str = "xoDGMX7S5WddQCADR3K9G9d6VpGi8pZDngWURtGwTKE";
    const Y_A2: &str = "R6ws3sR1ViG3NysJ40mHTfVC475QNZzA6ZKFt5HjfHo";

    const KID_A1: &str = "exp-key-1";
    const KID_A2: &str = "exp-key-2";
    const HARD_DEADLINE_SECS: u64 = 3600;

    let issuer = "https://exp-issuer.example";
    let audience = "https://exp-relay.example";

    fn jwks_str(kid: &str, x: &str, y: &str) -> String {
        format!(
            r#"{{"keys":[{{"kty":"EC","crv":"P-256","use":"sig","alg":"ES256","kid":"{kid}","x":"{x}","y":"{y}"}}]}}"#
        )
    }

    fn sign_token(pkcs8_pem: &str, kid: &str, iss: &str, aud: &str) -> String {
        let wall_now = chrono::Utc::now().timestamp();
        // nostr_pubkey is required unconditionally by spec v2.
        let claims = json!({"iss": iss, "aud": aud, "sub": "u",
                            "nostr_pubkey": "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef",
                            "iat": wall_now, "exp": wall_now + 600});
        let mut hdr = Header::new(Algorithm::ES256);
        hdr.kid = Some(kid.to_owned());
        hdr.typ = Some("nip-fi+jwt".to_owned());
        let key = EncodingKey::from_ec_pem(pkcs8_pem.as_bytes()).expect("valid EC PEM");
        jsonwebtoken::encode(&hdr, &claims, &key).expect("sign")
    }

    // Scripted fetcher: first call -> A1, second call -> A2.
    let bodies = Arc::new(std::sync::Mutex::new(vec![
        Ok::<String, JwksFetchError>(jwks_str(KID_A2, X_A2, Y_A2)), // popped second
        Ok(jwks_str(KID_A1, X_A1, Y_A1)),                           // popped first
    ]));

    struct RotatingFetcher {
        bodies: Arc<std::sync::Mutex<Vec<Result<String, JwksFetchError>>>>,
    }
    impl super::super::verifier::sealed::Sealed for RotatingFetcher {}
    impl JwksFetcher for RotatingFetcher {
        fn fetch_jwks<'a>(
            &'a self,
            _uri: &'a str,
        ) -> impl std::future::Future<Output = Result<String, JwksFetchError>> + Send + 'a {
            let result = self
                .bodies
                .lock()
                .unwrap()
                .pop()
                .unwrap_or(Err(JwksFetchError::NetworkError));
            async move { result }
        }
    }

    let jwks_contract = JwksSourceContract::new(
        format!("https://{issuer}/.well-known/jwks.json"),
        1,
        HARD_DEADLINE_SECS,
    )
    .unwrap();

    // Controlled clock: atomic epoch-seconds, starts at real T0.
    let t0 = chrono::Utc::now().timestamp();
    let clock = Arc::new(AtomicI64::new(t0));
    let clock2 = Arc::clone(&clock);
    let now_fn: Arc<dyn Fn() -> chrono::DateTime<chrono::Utc> + Send + Sync> =
        Arc::new(move || {
            chrono::DateTime::from_timestamp(clock2.load(Ordering::SeqCst), 0)
                .unwrap_or(chrono::DateTime::UNIX_EPOCH)
        });

    let config = IssuerJwksConfig {
        issuer: issuer.to_owned(),
        contract: jwks_contract.clone(),
    };
    // Mutation oracle 1 (sharing): pass a second independent Arc to the verifier,
    // separately warmed with A1 before advancing the clock. After advancement,
    // A2-accept flips red (verifier never observes A2 keys); A1-reject stays
    // green (independent cache also expired, so A1 keys are absent there too).
    let source = Arc::new(
        ProductionJwksSource::new_with_clock(
            vec![config],
            RotatingFetcher { bodies },
            Arc::clone(&now_fn),
        )
        .unwrap(),
    );

    // Step 1: warm cache with A1 JWKS (first scripted fetch at T0).
    let snap_a1 = source.get_snapshot(issuer).await.unwrap();
    let gen_a1 = snap_a1.generation();
    // A1's hard deadline is T0 + HARD_DEADLINE_SECS; never mutated by this test.
    let deadline_a1 = snap_a1.hard_deadline();

    // Step 2: build the ONE long-lived verifier.
    let mut registry = IssuerRegistry::new();
    registry.insert(
        IssuerPolicy::new(
            issuer.to_owned(),
            vec![audience.to_owned()],
            TokenClass::DedicatedNipFi,
            FreshnessClass::OfflineJwt,
            vec![Algorithm::ES256],
            60,
            HARD_DEADLINE_SECS,
            None,
            jwks_contract,
        )
        .unwrap(),
    );
    let verifier = FederatedAssertionVerifier::new(registry, Arc::clone(&source));

    // Pre-advancement: A1 verifies.
    verifier
        .verify(&sign_token(PKCS8_A1, KID_A1, issuer, audience))
        .expect("A1 token must verify before clock advances past its deadline");

    // Step 3: advance clock past A1's original hard deadline (no sleep).
    clock.store(t0 + HARD_DEADLINE_SECS as i64 + 1, Ordering::SeqCst);

    // Step 4: re-fetch through the SAME shared source.
    // Expiry purge fires (now > A1 deadline), second scripted response is A2.
    let snap_a2 = source.get_snapshot(issuer).await.unwrap();
    let gen_a2 = snap_a2.generation();
    let deadline_a2 = snap_a2.hard_deadline();

    assert!(
        gen_a2 > gen_a1,
        "generation must advance: A1={gen_a1} A2={gen_a2}"
    );
    // A2's deadline is computed at advanced clock time, so it is later than A1's.
    assert!(
        deadline_a2 > deadline_a1,
        "A2 deadline must be later than A1's original"
    );

    // Step 5: the SAME unchanged verifier reflects A2 keys.
    verifier
        .verify(&sign_token(PKCS8_A2, KID_A2, issuer, audience))
        .expect(
            "A2 token must verify through the unchanged verifier after A1 deadline expired; \
             mutation oracle: use independent Arc -> A2-accept flips red (reliable oracle)",
        );
    verifier
        .verify(&sign_token(PKCS8_A1, KID_A1, issuer, audience))
        .expect_err("A1 must be rejected after expiry + rotation");
}
