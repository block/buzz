use super::*;
#[cfg(unix)]
use crate::auth::TokenSource;
use serde_json::json;
#[cfg(unix)]
use std::sync::atomic::Ordering;

#[path = "databricks_test_support.rs"]
mod support;
use support::*;

fn strict(server: &Server, root: &Path, opener: Arc<Opener>) -> DatabricksConnection {
    DatabricksConnection::build(&server.base, root, opener, server.builder()).unwrap()
}
#[cfg(unix)]
fn legacy(server: &Server, root: &Path, opener: Arc<Opener>) -> Arc<PkceOAuthTokenSource> {
    PkceOAuthTokenSource::new_with(
        crate::llm::databricks_pkce_config(&server.base, Some(root.into())),
        opener,
    )
    .unwrap()
}

#[test]
fn public_constructor_requires_explicit_https_origin_and_root() {
    let root = tempfile::tempdir().unwrap();
    for host in [
        "",
        "localhost",
        "http://localhost",
        "https://user:pass@localhost",
        "https://localhost/path",
        "https://localhost?secret",
        "https://localhost#secret",
        " https://localhost",
        "https://local\nhost",
        "https://localhost\\evil",
        "https:localhost",
        "https://@localhost",
        "https://localhost/path/..",
    ] {
        assert!(
            DatabricksConnection::new(host, root.path(), Arc::new(Opener::default())).is_err(),
            "{host:?}"
        );
    }
    assert!(DatabricksConnection::new(
        "https://workspace.invalid",
        Path::new("relative"),
        Arc::new(Opener::default())
    )
    .is_err());
    let connection = DatabricksConnection::new(
        "https://WORKSPACE.invalid:443/",
        root.path(),
        Arc::new(Opener::default()),
    )
    .unwrap();
    assert_eq!(connection.workspace.as_str(), "https://workspace.invalid");
    assert!(root.path().join("databricks-strict").exists());
}

#[tokio::test]
async fn strict_headless_empty_never_networks_or_browses() {
    let server = Server::start(true, |_, _| panic!("unexpected network")).await;
    let root = tempfile::tempdir().unwrap();
    let opener = Arc::new(Opener::default());
    let c = strict(&server, root.path(), opener.clone());
    assert!(matches!(
        c.discover_models(None).await,
        Err(AgentError::LlmAuth(_))
    ));
    assert!(server.requests.lock().unwrap().is_empty());
    assert!(opener.urls.lock().unwrap().is_empty());
}

#[cfg(unix)]
#[tokio::test]
async fn strict_rejects_discovered_endpoints_before_grants_or_browser() {
    let sink = Server::start(true, |_, _| {
        Reply::Json(200, json!({"access_token":"wrong"}))
    })
    .await;
    // Same host/different port, different host, insecure URL, userinfo, query,
    // fragment and malformed URLs must all fail before either grant or opener.
    for endpoint in [
        format!("{}/token", sink.base),
        "https://other.invalid/token".into(),
        "http://localhost/token".into(),
        "https://user:secret@localhost/token".into(),
        "not a URL".into(),
    ] {
        for field in ["authorization_endpoint", "token_endpoint"] {
            let endpoint = endpoint.clone();
            let server = Server::start(true, move |base, _| {
                let mut body = json!({"authorization_endpoint":format!("{base}/authorize"),"token_endpoint":format!("{base}/token")});
                body[field] = json!(endpoint);
                Reply::Json(200, body)
            }).await;
            for expired in [false, true] {
                let root = tempfile::tempdir().unwrap();
                if expired {
                    seed(root.path(), &server.base, "databricks-strict", "old", true);
                }
                let opener = Arc::new(Opener {
                    fail: true,
                    ..Default::default()
                });
                let c = strict(&server, root.path(), opener.clone());
                assert_eq!(c.connect().await, Err(AuthError::NetworkUnavailable));
                assert_eq!(server.count("/token"), 0);
                assert!(opener.urls.lock().unwrap().is_empty());
            }
        }
    }
    assert!(sink.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn strict_rejects_same_origin_endpoint_credentials_query_fragment() {
    for suffix in [
        "/token?secret=echo",
        "/token#fragment",
        "/token\n",
        "\\token",
    ] {
        for field in ["authorization_endpoint", "token_endpoint"] {
            let server = Server::start(true, move |base, _| {
                let mut body = json!({"authorization_endpoint":format!("{base}/authorize"),"token_endpoint":format!("{base}/token")});
                body[field] = json!(format!("{base}{suffix}"));
                Reply::Json(200, body)
            }).await;
            let root = tempfile::tempdir().unwrap();
            let c = strict(
                &server,
                root.path(),
                Arc::new(Opener {
                    fail: true,
                    ..Default::default()
                }),
            );
            assert_eq!(c.connect().await, Err(AuthError::NetworkUnavailable));
        }
    }
}

#[tokio::test]
async fn strict_connect_code_exchange_catalog_and_headless_401_refresh() {
    let server = Server::start(true, |base, request| match request.path.as_str() {
        "/oidc/.well-known/oauth-authorization-server" => discovery(base),
        "/token" => {
            let form = request.form();
            assert_eq!(form["client_id"], "databricks-cli");
            let token = if form["grant_type"] == "authorization_code" {
                assert_eq!(form["code"], "synthetic-code");
                assert!(form["code_verifier"].len() >= 43);
                "rejected"
            } else {
                assert_eq!(form["refresh_token"], "synthetic-refresh");
                "fresh"
            };
            Reply::Json(
                200,
                json!({"access_token":token,"refresh_token":"synthetic-refresh","expires_in":3600}),
            )
        }
        path if path.starts_with("/api/") => {
            if request.authorization.as_deref() == Some("Bearer rejected") {
                return Reply::Json(401, json!({"secret":"echo"}));
            }
            assert_eq!(request.authorization.as_deref(), Some("Bearer fresh"));
            if path.starts_with("/api/ai-gateway") {
                Reply::Json(200, json!({"endpoints":[{"name":"custom-chat"}]}))
            } else {
                Reply::Json(200, json!({"model_services":[]}))
            }
        }
        _ => panic!("unexpected request {request:?}"),
    })
    .await;
    let root = tempfile::tempdir().unwrap();
    let opener = Arc::new(Opener {
        callback: true,
        ..Default::default()
    });
    let c = strict(&server, root.path(), opener.clone());
    c.connect().await.unwrap();
    assert_eq!(server.count("/api/"), 0, "connect must not discover models");
    let models = c.discover_models(None).await.unwrap();
    assert_eq!(
        models,
        vec![ModelEntry {
            id: "custom-chat".into(),
            name: "custom-chat".into()
        }]
    );
    assert_eq!(server.count("/token"), 2);
    assert_eq!(opener.urls.lock().unwrap().len(), 1);
    c.connect().await.unwrap();
    assert_eq!(
        opener.urls.lock().unwrap().len(),
        1,
        "fresh cache must not reopen browser"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn strict_never_follows_discovery_token_or_catalog_redirects() {
    let sink = Server::start(true, |_, _| {
        Reply::Json(200, json!({"access_token":"escaped"}))
    })
    .await;
    for status in [301, 302, 303, 307, 308] {
        for stage in ["discovery", "refresh", "exchange", "catalog"] {
            let location = format!("{}/sink", sink.base);
            let server = Server::start(true, move |base, r| {
                let redirect = match stage {
                    "discovery" => true,
                    "refresh" | "exchange" => r.path == "/token",
                    _ => r.path.starts_with("/api/"),
                };
                if redirect { return Reply::Raw(format!("HTTP/1.1 {status} Redirect\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")); }
                discovery(base)
            }).await;
            let root = tempfile::tempdir().unwrap();
            if stage == "refresh" || stage == "catalog" {
                seed(
                    root.path(),
                    &server.base,
                    "databricks-strict",
                    "synthetic-access",
                    stage == "refresh",
                );
            }
            let opener = Arc::new(Opener {
                callback: true,
                ..Default::default()
            });
            // Trust the second server too: lack of trust cannot falsely prove
            // redirect containment. The no-redirect policy must do the work.
            let c = DatabricksConnection::build(
                &server.base,
                root.path(),
                opener.clone(),
                server
                    .builder()
                    .add_root_certificate(sink.cert.clone().unwrap()),
            )
            .unwrap();
            if stage == "catalog" {
                assert!(matches!(
                    c.discover_models(None).await,
                    Err(AgentError::Llm(_))
                ));
            } else {
                assert_eq!(c.connect().await, Err(AuthError::NetworkUnavailable));
            }
            assert_eq!(
                opener.urls.lock().unwrap().len(),
                usize::from(stage == "exchange")
            );
            assert_eq!(sink.count("/sink"), 0, "redirect escaped: {stage}/{status}");
        }
    }
    // Positive control: the sink is reachable with the same trust settings.
    sink.builder()
        .build()
        .unwrap()
        .get(format!("{}/control", sink.base))
        .send()
        .await
        .unwrap();
    assert_eq!(sink.count("/control"), 1);
}

#[cfg(unix)]
#[tokio::test]
async fn strict_catalog_preserves_empty_filter_partial_and_paging_semantics() {
    for scenario in ["empty", "filtered-empty", "partial", "pages"] {
        let server = Server::start(true, move |_, r| {
            if scenario == "partial" && r.path.starts_with("/api/2.1/") { return Reply::Json(403, json!({"message":"unavailable"})); }
            if r.path.starts_with("/api/2.1/") { return Reply::Json(200, json!({"model_services":[]})); }
            assert_eq!(r.authorization.as_deref(), Some("Bearer synthetic-access"));
            match scenario {
                "empty" | "filtered-empty" => Reply::Json(200, json!({"endpoints":[]})),
                "pages" if !r.path.contains("page_token=") => Reply::Json(200, json!({"endpoints":[{"name":"one"}],"next_page_token":"https://other.invalid/?secret"})),
                _ => Reply::Json(200, json!({"endpoints":[{"name":"two"}]})),
            }
        }).await;
        let root = tempfile::tempdir().unwrap();
        seed(
            root.path(),
            &server.base,
            "databricks-strict",
            "synthetic-access",
            false,
        );
        let c = strict(&server, root.path(), Arc::new(Opener::default()));
        let filter = if scenario == "filtered-empty" {
            DatabricksModelFilter::parse(Some("no-match")).unwrap()
        } else {
            None
        };
        let models = c.discover_models(filter).await.unwrap();
        match scenario {
            "empty" => assert!(
                models.len() > 1
                    && models
                        .iter()
                        .all(|m| m.name.ends_with(" (default catalog)"))
            ),
            "filtered-empty" => assert!(models.is_empty()),
            "partial" => assert_eq!(
                models.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
                ["two"]
            ),
            _ => {
                assert_eq!(models.len(), 2);
                assert_eq!(server.count("/api/ai-gateway"), 2);
            }
        }
    }
}

#[cfg(unix)]
#[tokio::test]
async fn strict_cancellation_drops_real_http_work_and_allows_retry() {
    for stage in ["discovery", "refresh", "exchange", "catalog"] {
        let stalled = Arc::new(std::sync::atomic::AtomicBool::new(true));
        let stalled2 = stalled.clone();
        let server = Server::start(true, move |base, r| {
            let target = match stage { "discovery" => r.path.contains("well-known"), "refresh"|"exchange" => r.path == "/token", _ => r.path.starts_with("/api/") };
            if target && stalled2.load(Ordering::SeqCst) { return Reply::Stall; }
            if r.path.contains("well-known") { discovery(base) }
            else if r.path == "/token" { Reply::Json(200, json!({"access_token":"fresh", "refresh_token":"synthetic-refresh","expires_in":3600})) }
            else if r.path.starts_with("/api/ai-gateway") { Reply::Json(200, json!({"endpoints":[]})) }
            else { Reply::Json(200, json!({"model_services":[]})) }
        }).await;
        let root = tempfile::tempdir().unwrap();
        if stage == "refresh" || stage == "catalog" {
            seed(
                root.path(),
                &server.base,
                "databricks-strict",
                "old",
                stage == "refresh",
            );
        }
        let opener = Arc::new(Opener {
            callback: true,
            ..Default::default()
        });
        let c = Arc::new(strict(&server, root.path(), opener.clone()));
        let c2 = c.clone();
        let task = tokio::spawn(async move {
            if stage == "catalog" {
                c2.discover_models(None).await.map(|_| ())
            } else {
                c2.connect().await.map_err(AgentError::from)
            }
        });
        server
            .wait_for(match stage {
                "discovery" => "/oidc/",
                "refresh" | "exchange" => "/token",
                _ => "/api/",
            })
            .await;
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        tokio::time::timeout(Duration::from_secs(3), async {
            while server.active.load(Ordering::SeqCst) != 0 {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        stalled.store(false, Ordering::SeqCst);
        tokio::time::timeout(Duration::from_secs(3), async {
            if stage == "catalog" {
                c.discover_models(None).await.unwrap();
            } else {
                c.connect().await.unwrap();
            }
        })
        .await
        .unwrap();
    }
}

#[tokio::test]
async fn strict_browser_wait_cancellation_releases_callback_listener() {
    let server = Server::start(true, |base, _| discovery(base)).await;
    let root = tempfile::tempdir().unwrap();
    let opener = Arc::new(Opener::default());
    let c = Arc::new(strict(&server, root.path(), opener.clone()));
    let c2 = c.clone();
    let task = tokio::spawn(async move { c2.connect().await });
    tokio::time::timeout(Duration::from_secs(3), async {
        while opener.urls.lock().unwrap().is_empty() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    let url = Url::parse(&opener.urls.lock().unwrap()[0]).unwrap();
    let callback = url
        .query_pairs()
        .find(|(k, _)| k == "redirect_uri")
        .unwrap()
        .1
        .into_owned();
    let client = Client::builder().no_proxy().build().unwrap();
    // Connect-only positive control leaves the one-shot callback unconsumed.
    let port = Url::parse(&callback).unwrap().port().unwrap();
    let socket = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .unwrap();
    drop(socket);
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if client.get(&callback).send().await.is_err() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn legacy_different_origin_endpoints_and_redirects_remain_supported() {
    let sink = Server::start(false, |_, r| {
        assert_eq!(r.form()["refresh_token"], "synthetic-refresh");
        Reply::Json(
            200,
            json!({"access_token":"legacy-fresh","expires_in":3600}),
        )
    })
    .await;
    for redirected in [false, true] {
        let endpoint = format!("{}/token", sink.base);
        let server = Server::start(false, move |base, r| {
            if r.path == "/token" { Reply::Raw(format!("HTTP/1.1 307 Redirect\r\nLocation: {endpoint}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")) }
            else { Reply::Json(200, json!({"authorization_endpoint":format!("{base}/authorize"),"token_endpoint":if redirected {format!("{base}/token")} else {endpoint.clone()}})) }
        }).await;
        let root = tempfile::tempdir().unwrap();
        seed(root.path(), &server.base, "databricks", "old", true);
        assert_eq!(
            legacy(&server, root.path(), Arc::new(Opener::default()))
                .bearer_no_browser()
                .await
                .unwrap(),
            "legacy-fresh"
        );
    }
    assert_eq!(sink.count("/token"), 2);
}

#[cfg(unix)]
#[tokio::test]
async fn strict_host_root_and_legacy_namespace_isolation() {
    let server = Server::start(true, |_, _| panic!("should not reach network")).await;
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    seed(a.path(), &server.base, "databricks", "legacy", false);
    let c = strict(&server, a.path(), Arc::new(Opener::default()));
    assert!(matches!(
        c.discover_models(None).await,
        Err(AgentError::LlmAuth(_))
    ));
    seed(
        a.path(),
        &server.base,
        "databricks-strict",
        "strict-a",
        false,
    );
    let c = strict(&server, b.path(), Arc::new(Opener::default()));
    assert_eq!(
        c.source.bearer_no_browser().await.unwrap_err().to_string(),
        AgentError::from(AuthError::NoCredential).to_string()
    );
    let c = DatabricksConnection::new(
        "https://different.invalid",
        a.path(),
        Arc::new(Opener::default()),
    )
    .unwrap();
    assert!(matches!(
        c.source.bearer_no_browser().await,
        Err(AgentError::LlmAuth(_))
    ));
    let c = strict(&server, a.path(), Arc::new(Opener::default()));
    assert_eq!(c.source.bearer_no_browser().await.unwrap(), "strict-a");
}

#[cfg(unix)]
#[tokio::test]
async fn shared_oauth_limits_bound_chunked_and_declared_bodies_in_both_paths() {
    use crate::auth_http::{MAX_OAUTH_ERROR_BYTES, MAX_OAUTH_RESPONSE_BYTES};
    for strict_path in [false, true] {
        for stage in ["discovery", "refresh", "exchange"] {
            for (status, limit) in [
                (200, MAX_OAUTH_RESPONSE_BYTES),
                (400, MAX_OAUTH_ERROR_BYTES),
            ] {
                for declared in [false, true] {
                    let server = Server::start(strict_path, move |base, request| {
                        let target = if stage == "discovery" { request.path.contains("well-known") } else { request.path == "/token" };
                        if !target { return discovery(base); }
                        if declared {
                            // Headers alone exceed the bound; never wait on the body.
                            Reply::Raw(format!("HTTP/1.1 {status} Test\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", limit+1))
                        } else {
                            // Valid JSON + padding means a decoder without the cap
                            // would succeed (or misclassify an oversized invalid_grant).
                            let value = if status == 400 { json!({"error":"invalid_grant"}) }
                                else if stage == "discovery" { json!({"authorization_endpoint":format!("{base}/authorize"),"token_endpoint":format!("{base}/token")}) }
                                else { json!({"access_token":"must-not-cache","expires_in":3600}) };
                            let mut body = value.to_string();
                            body.push_str(&" ".repeat(limit+1-body.len()));
                            chunked(status, &body)
                        }
                    }).await;
                    let root = tempfile::tempdir().unwrap();
                    if stage == "refresh" {
                        seed(
                            root.path(),
                            &server.base,
                            if strict_path {
                                "databricks-strict"
                            } else {
                                "databricks"
                            },
                            "old",
                            true,
                        );
                    }
                    let opener = Arc::new(Opener {
                        callback: true,
                        ..Default::default()
                    });
                    let result = tokio::time::timeout(Duration::from_secs(3), async {
                        if strict_path {
                            strict(&server, root.path(), opener.clone()).connect().await
                        } else {
                            legacy(&server, root.path(), opener.clone())
                                .acquire_with_intent(AuthIntent::UserInitiated, None)
                                .await
                                .map(|_| ())
                        }
                    })
                    .await
                    .unwrap();
                    assert_eq!(
                        result,
                        Err(AuthError::NetworkUnavailable),
                        "strict={strict_path} {stage} {status} declared={declared}"
                    );
                    assert_eq!(
                        opener.urls.lock().unwrap().len(),
                        usize::from(stage == "exchange")
                    );
                }
            }
        }
    }
}

#[cfg(unix)]
#[tokio::test]
async fn shared_exact_limit_responses_and_grant_classifications_are_preserved() {
    use crate::auth_http::{MAX_OAUTH_ERROR_BYTES, MAX_OAUTH_RESPONSE_BYTES};
    for strict_path in [false, true] {
        for stage in ["refresh", "exchange"] {
            for (status, error, expected) in [
                (200, "", Ok(())),
                (
                    400,
                    "invalid_grant",
                    Err(if stage == "refresh" {
                        AuthError::RefreshRejected
                    } else {
                        AuthError::ExchangeFailed
                    }),
                ),
                (400, "invalid_client", Err(AuthError::NetworkUnavailable)),
                (429, "invalid_request", Err(AuthError::NetworkUnavailable)),
                (503, "invalid_grant", Err(AuthError::NetworkUnavailable)),
            ] {
                let server = Server::start(strict_path, move |base, request| {
                    let mut body = if request.path.contains("well-known") { json!({"authorization_endpoint":format!("{base}/authorize"),"token_endpoint":format!("{base}/token")}).to_string() }
                        else if status == 200 { json!({"access_token":"valid","refresh_token":"synthetic-refresh","expires_in":3600}).to_string() }
                        else { json!({"error":error, "error_description":"secret-echo"}).to_string() };
                    let status = if request.path.contains("well-known") { 200 } else { status };
                    let limit = if status == 200 { MAX_OAUTH_RESPONSE_BYTES } else { MAX_OAUTH_ERROR_BYTES };
                    body.push_str(&" ".repeat(limit-body.len()));
                    chunked(status, &body)
                }).await;
                let root = tempfile::tempdir().unwrap();
                if stage == "refresh" {
                    seed(
                        root.path(),
                        &server.base,
                        if strict_path {
                            "databricks-strict"
                        } else {
                            "databricks"
                        },
                        "old",
                        true,
                    );
                }
                let opener = Arc::new(Opener {
                    callback: true,
                    ..Default::default()
                });
                let c;
                let source = if strict_path {
                    c = strict(&server, root.path(), opener.clone());
                    c.source.clone()
                } else {
                    legacy(&server, root.path(), opener.clone())
                };
                let intent = if stage == "refresh" {
                    AuthIntent::Headless
                } else {
                    AuthIntent::UserInitiated
                };
                assert_eq!(
                    source.acquire_with_intent(intent, None).await.map(|_| ()),
                    expected
                );
                assert_eq!(
                    opener.urls.lock().unwrap().len(),
                    usize::from(stage == "exchange")
                );
            }
        }
    }
}

#[cfg(unix)]
#[tokio::test]
async fn shared_diagnostics_never_log_oauth_bodies_urls_opener_or_callback_details() {
    use std::{io::Write, sync::Mutex};
    // Isolate tracing's global callsite interest cache from concurrently running
    // lib tests. The child runs ONLY this test, never an agent or real browser.
    const MARKER: &str = "BUZZ_TEST_OAUTH_LOG_CAPTURE_CHILD";
    if std::env::var_os(MARKER).is_none() {
        let output = tokio::time::timeout(Duration::from_secs(15),
            tokio::process::Command::new(std::env::current_exe().unwrap())
                .arg("--exact")
                .arg("databricks::tests::shared_diagnostics_never_log_oauth_bodies_urls_opener_or_callback_details")
                .arg("--nocapture")
                .env(MARKER, "1")
                .kill_on_drop(true)
                .output()).await.unwrap().unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    #[derive(Clone, Default)]
    struct Capture(Arc<Mutex<Vec<u8>>>);
    impl Write for Capture {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Capture {
        type Writer = Self;
        fn make_writer(&'a self) -> Self {
            self.clone()
        }
    }
    let logs = Capture::default();
    tracing::subscriber::set_global_default(
        tracing_subscriber::fmt()
            .with_ansi(false)
            .without_time()
            .with_writer(logs.clone())
            .finish(),
    )
    .unwrap();
    for strict_path in [false, true] {
        for scenario in [
            "refresh-rejected",
            "refresh-network",
            "exchange-rejected",
            "exchange-network",
            "opener",
            "denial",
        ] {
            logs.0.lock().unwrap().clear();
            let server = Server::start(strict_path, move |base, request| {
                if request.path.contains("well-known") { return discovery(base); }
                let rejected = scenario.ends_with("rejected");
                Reply::Json(if rejected {400} else {503}, json!({"error":if rejected {"invalid_grant"} else {"server_error"},"error_description":format!("secret-provider-body {} {base}", request.body)}))
            }).await;
            let root = tempfile::tempdir().unwrap();
            if scenario.starts_with("refresh") {
                seed(
                    root.path(),
                    &server.base,
                    if strict_path {
                        "databricks-strict"
                    } else {
                        "databricks"
                    },
                    "old",
                    true,
                );
            }
            let opener = Arc::new(Opener {
                callback: true,
                fail: scenario == "opener",
                denial: scenario == "denial",
                ..Default::default()
            });
            let source = if strict_path {
                strict(&server, root.path(), opener).source
            } else {
                legacy(&server, root.path(), opener)
            };
            let intent = if scenario.starts_with("refresh") {
                AuthIntent::Headless
            } else {
                AuthIntent::UserInitiated
            };
            assert!(source.acquire_with_intent(intent, None).await.is_err());
            let output = String::from_utf8(logs.0.lock().unwrap().clone()).unwrap();
            assert!(
                output.contains("oauth"),
                "capture must contain positive log evidence: strict={strict_path} scenario={scenario}"
            );
            for secret in [
                "secret-provider-body",
                "secret-opener-error",
                "secret-denial",
                "synthetic-refresh",
                "synthetic-code",
                "code_verifier",
                "code_challenge",
                &server.base,
            ] {
                assert!(
                    !output.contains(secret),
                    "diagnostic leaked {secret}: {output}"
                );
            }
        }
    }
}

#[tokio::test]
async fn strict_malformed_token_expiry_is_a_safe_infrastructure_failure() {
    let server = Server::start(true, |base, r| {
        if r.path.contains("well-known") {
            discovery(base)
        } else {
            Reply::Json(
                200,
                json!({"access_token":"synthetic", "expires_in":u64::MAX}),
            )
        }
    })
    .await;
    let root = tempfile::tempdir().unwrap();
    let opener = Arc::new(Opener {
        callback: true,
        ..Default::default()
    });
    assert_eq!(
        strict(&server, root.path(), opener).connect().await,
        Err(AuthError::NetworkUnavailable)
    );
}

#[cfg(unix)]
#[tokio::test]
async fn strict_catalog_errors_redact_provider_text_and_urls() {
    let server = Server::start(true, |_, _| {
        Reply::Json(
            403,
            json!({"message":"secret-provider-diagnostic https://private.invalid/?token=secret"}),
        )
    })
    .await;
    let root = tempfile::tempdir().unwrap();
    seed(
        root.path(),
        &server.base,
        "databricks-strict",
        "synthetic-access",
        false,
    );
    let c = strict(&server, root.path(), Arc::new(Opener::default()));
    assert_eq!(
        c.discover_models(None).await.unwrap_err().to_string(),
        AgentError::Llm("Databricks model discovery unavailable".into()).to_string()
    );
    assert_eq!(server.count("/api/"), 2);
}
