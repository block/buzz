//! Regression tests for the explicit mesh JWKS transport.

use super::*;

fn minimal_jwks_json(kid: &str) -> String {
    format!(
        r#"{{"keys":[{{"kty":"EC","crv":"P-256","x":"f83OJ3D2xF1Bg8vub9tLe1gHMzV76e8Tus9uPHvRVEU","y":"x_FEzRu9m36HLN_tue659LNpXW6pCyStikYjKIWI5a0","use":"sig","alg":"ES256","kid":"{kid}"}}]}}"#
    )
}

fn mesh_contract(uri: &str) -> JwksSourceContract {
    JwksSourceContract::new_mesh(uri.to_owned(), 300, 3600).expect("valid mesh contract")
}

#[test]
fn mesh_contract_rejects_invalid_uri_and_timing() {
    for uri in [
        "https://mesh.example.test/jwks.json",
        "http://127.0.0.1/jwks.json",
        "http://[::1]/jwks.json",
        "http://2130706433/jwks.json",
        "http://0177.0.0.1/jwks.json",
        "http://0x7f000001/jwks.json",
        "http://user:password@mesh.example.test/jwks.json",
        "http://@mesh.example.test/jwks.json",
        "http://mesh.example.test/jwks.json?token=private",
        "http://mesh.example.test/jwks.json?",
        "http://mesh.example.test/jwks.json#fragment",
        "http://mesh.example.test/jwks.json#",
        "http:\\\\mesh.example.test\\jwks.json",
    ] {
        assert!(
            JwksSourceContract::new_mesh(uri.to_owned(), 300, 3600).is_none(),
            "mesh contract must reject {uri}"
        );
    }

    let uri = "http://mesh.example.test/jwks.json".to_owned();
    for (refresh, deadline) in [
        (0, 3600),
        (300, 0),
        (3600, 3600),
        (300, 299),
        (1, MAX_JWKS_TIMING_SECONDS + 1),
    ] {
        assert!(
            JwksSourceContract::new_mesh(uri.clone(), refresh, deadline).is_none(),
            "mesh contract must reject timing ({refresh}, {deadline})"
        );
    }

    assert!(
        JwksSourceContract::new_mesh(uri.clone(), 300, 301).is_some(),
        "a deadline just above the refresh interval must be accepted"
    );

    assert!(
        JwksSourceContract::new(uri, 300, 3600).is_none(),
        "the public constructor must keep rejecting HTTP"
    );
}

#[test]
fn mesh_contract_canonicalizes_and_transport_participates_in_equality() {
    let canonical = mesh_contract("http://mesh.example.test/a/jwks.json");
    let equivalent = mesh_contract("HTTP://MESH.EXAMPLE.TEST:80/a/./jwks.json");
    assert_eq!(canonical.jwks_uri(), "http://mesh.example.test/a/jwks.json");
    assert_eq!(canonical, equivalent);

    let public = JwksSourceContract::new(
        "https://mesh.example.test/a/jwks.json".to_owned(),
        300,
        3600,
    )
    .expect("valid public contract");
    let mut transport_mismatch = public.clone();
    transport_mismatch.transport = JwksTransport::Mesh;
    assert_ne!(
        public, transport_mismatch,
        "the sealed contract equality must include transport"
    );
}

#[test]
fn mesh_address_allow_list_covers_range_boundaries_and_embedded_ipv6() {
    let cases = [
        ("127.0.0.0", true),
        ("127.255.255.255", true),
        ("126.255.255.255", false),
        ("128.0.0.0", false),
        ("10.0.0.0", true),
        ("10.255.255.255", true),
        ("9.255.255.255", false),
        ("11.0.0.0", false),
        ("172.16.0.0", true),
        ("172.31.255.255", true),
        ("172.15.255.255", false),
        ("172.32.0.0", false),
        ("192.168.0.0", true),
        ("192.168.255.255", true),
        ("192.167.255.255", false),
        ("192.169.0.0", false),
        ("100.64.0.0", true),
        ("100.127.255.255", true),
        ("100.63.255.255", false),
        ("100.128.0.0", false),
        ("240.0.0.0", true),
        ("255.255.255.254", true),
        ("239.255.255.255", false),
        ("255.255.255.255", false),
        ("169.254.0.1", false),
        ("0.0.0.0", false),
        ("224.0.0.1", false),
        ("198.18.0.1", false),
        ("192.0.2.1", false),
        ("198.51.100.1", false),
        ("203.0.113.1", false),
        ("192.88.99.1", false),
        ("fc00::", true),
        ("fdff:ffff:ffff:ffff:ffff:ffff:ffff:ffff", true),
        ("fbff:ffff:ffff:ffff:ffff:ffff:ffff:ffff", false),
        ("fe00::", false),
        ("::1", true),
        ("::", false),
        ("fe80::1", false),
        ("fec0::1", false),
        ("fe00::1", false),
        ("ff02::1", false),
        ("2001:db8::1", false),
        ("2001:2::1", false),
        ("2002:a00:1::", false),
        ("::a00:1", false),
        ("::ffff:10.0.0.1", false),
        ("::ffff:0:0:a00:1", false),
        ("64:ff9b::10.0.0.1", false),
        ("64:ff9b:1::10.0.0.1", false),
    ];

    for (address, expected) in cases {
        let ip = address.parse().expect("valid test IP address");
        assert_eq!(
            is_mesh_address_allowed(&ip),
            expected,
            "mesh address policy for {address}"
        );
    }
}

#[tokio::test]
async fn mesh_resolver_validates_ip_literal_fast_path() {
    use std::net::IpAddr;

    for (host, expected) in [
        ("8.8.8.8", Err(JwksFetchError::InvalidUri)),
        ("2606:4700::1", Err(JwksFetchError::InvalidUri)),
        (
            "10.0.0.1",
            Ok("10.0.0.1".parse::<IpAddr>().expect("valid test IP")),
        ),
    ] {
        assert_eq!(
            resolve_for_transport(host, 443, JwksTransport::Mesh).await,
            expected,
            "mesh resolver result for {host}"
        );
    }
}

#[tokio::test]
async fn transport_resolver_checks_every_answer_and_preserves_empty_result_error() {
    use std::net::IpAddr;

    let allowed = "10.0.0.1".parse::<IpAddr>().expect("valid test IP");
    let also_allowed = "192.168.0.1".parse::<IpAddr>().expect("valid test IP");
    let public = "8.8.8.8".parse::<IpAddr>().expect("valid test IP");
    let public_v6 = "2606:4700::1".parse::<IpAddr>().expect("valid test IP");

    assert_eq!(
        TEST_DNS_ANSWERS
            .scope(
                vec![allowed, also_allowed],
                resolve_for_transport("mesh.example.test", 443, JwksTransport::Mesh),
            )
            .await,
        Ok(allowed),
        "the first address is pinned only after the complete answer set passes"
    );

    for answers in [
        vec![allowed, public],
        vec![public, allowed],
        vec![also_allowed, public_v6],
        vec![public_v6, also_allowed],
    ] {
        assert_eq!(
            TEST_DNS_ANSWERS
                .scope(
                    answers,
                    resolve_for_transport("mesh.example.test", 443, JwksTransport::Mesh),
                )
                .await,
            Err(JwksFetchError::InvalidUri),
            "mesh rejects a mixed answer regardless of address ordering"
        );
    }

    for answers in [vec![allowed, public], vec![public, allowed]] {
        assert_eq!(
            TEST_DNS_ANSWERS
                .scope(
                    answers,
                    resolve_for_transport("public.example.test", 443, JwksTransport::PublicHttps),
                )
                .await,
            Err(JwksFetchError::InvalidUri),
            "public HTTPS retains its strict all-answer check"
        );
    }

    assert_eq!(
        TEST_DNS_ANSWERS
            .scope(
                Vec::new(),
                resolve_for_transport("mesh.example.test", 443, JwksTransport::Mesh),
            )
            .await,
        Err(JwksFetchError::NetworkError),
        "an empty resolver result remains a network error"
    );
}

async fn bind_localhost_listener() -> tokio::net::TcpListener {
    use std::net::ToSocketAddrs;

    let listener = tokio::task::spawn_blocking(|| {
        let address = ("localhost", 0)
            .to_socket_addrs()?
            .next()
            .ok_or_else(|| std::io::Error::other("localhost has no addresses"))?;
        let listener = std::net::TcpListener::bind(address)?;
        listener.set_nonblocking(true)?;
        Ok::<_, std::io::Error>(listener)
    })
    .await
    .expect("localhost listener task")
    .expect("bind first localhost address");
    tokio::net::TcpListener::from_std(listener).expect("register listener with Tokio")
}

async fn read_http_request(stream: &mut tokio::net::TcpStream) -> String {
    use tokio::io::AsyncReadExt as _;

    let mut bytes = Vec::new();
    let mut chunk = [0; 1024];
    while !bytes.windows(4).any(|window| window == b"\r\n\r\n") {
        let read = stream.read(&mut chunk).await.expect("read request");
        if read == 0 {
            break;
        }
        bytes.extend_from_slice(&chunk[..read]);
    }
    String::from_utf8(bytes).expect("HTTP request headers are ASCII")
}

async fn write_http_response(
    stream: &mut tokio::net::TcpStream,
    status: &str,
    headers: &str,
    body: &str,
) {
    use tokio::io::AsyncWriteExt as _;

    let response = format!(
        "HTTP/1.1 {status}\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream
        .write_all(response.as_bytes())
        .await
        .expect("write response");
}

#[tokio::test]
async fn production_source_fetches_mesh_contract_and_raw_fetch_stays_public_only() {
    let listener = bind_localhost_listener().await;
    let port = listener.local_addr().expect("listener address").port();
    let uri = format!("http://localhost:{port}/.well-known/jwks.json");
    let contract = mesh_contract(&uri);
    let issuer = "https://issuer.example";
    let public_issuer = "https://public-issuer.example";
    let public_contract = JwksSourceContract::new(
        format!("https://localhost:{port}/public-jwks.json"),
        300,
        3600,
    )
    .expect("public HTTPS hostname contract");
    let source = ProductionJwksSource::new(
        vec![
            IssuerJwksConfig {
                issuer: issuer.to_owned(),
                contract: contract.clone(),
            },
            IssuerJwksConfig {
                issuer: public_issuer.to_owned(),
                contract: public_contract,
            },
        ],
        HttpJwksFetcher::new(),
    )
    .expect("valid mixed-transport source");
    let body = minimal_jwks_json("mesh-key");
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("accept JWKS request");
        let request = read_http_request(&mut stream).await;
        write_http_response(
            &mut stream,
            "200 OK",
            "Content-Type: application/json\r\n",
            &body,
        )
        .await;
        (listener, request)
    });

    let snapshot = source
        .get_snapshot(issuer)
        .await
        .expect("production source warms its snapshot through the mesh contract");
    assert_eq!(snapshot.issuer(), issuer);
    let (listener, request) = server.await.expect("local JWKS server task");
    assert!(
        request.starts_with("GET /.well-known/jwks.json HTTP/1.1\r\n"),
        "the fetcher uses GET at the configured path: {request:?}"
    );

    assert_eq!(
        HttpJwksFetcher::new().fetch_jwks(&uri).await.unwrap_err(),
        JwksFetchError::InvalidUri,
        "raw fetches cannot inherit the contract's HTTP permission"
    );
    assert!(
        JwksSourceContract::new(uri, 300, 3600).is_none(),
        "the public constructor cannot create an HTTP contract"
    );

    tokio::select! {
        snapshot = source.get_snapshot(public_issuer) => {
            assert!(snapshot.is_none(), "public HTTPS must reject localhost DNS");
        }
        accepted = listener.accept() => {
            panic!("public HTTPS contract attempted to connect to its non-global DNS result: {accepted:?}");
        }
    }
}

#[tokio::test]
async fn production_mesh_fetcher_checks_all_dns_answers_before_connecting() {
    use std::net::IpAddr;

    let listener = bind_localhost_listener().await;
    let address = listener.local_addr().expect("listener address");
    let uri = format!("http://mesh.example.test:{}/jwks.json", address.port());
    let contract = mesh_contract(&uri);
    let body = minimal_jwks_json("mixed-answer-key");
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("accept fetcher request");
        let request = read_http_request(&mut stream).await;
        write_http_response(&mut stream, "200 OK", "", &body).await;
        request
    });

    let result = TEST_DNS_ANSWERS
        .scope(
            vec![
                address.ip(),
                IpAddr::V4(std::net::Ipv4Addr::new(8, 8, 8, 8)),
            ],
            HttpJwksFetcher::new().fetch_for_contract(&contract),
        )
        .await;
    assert_eq!(
        result,
        Err(JwksFetchError::InvalidUri),
        "the production fetcher rejects the mixed DNS answer before connecting"
    );

    server.abort();
    assert!(server.await.unwrap_err().is_cancelled());
}

#[tokio::test]
async fn production_mesh_fetcher_denies_redirects_and_non_success_statuses() {
    let listener = bind_localhost_listener().await;
    let port = listener.local_addr().expect("listener address").port();
    let uri = format!("http://localhost:{port}/jwks.json");
    let contract = mesh_contract(&uri);
    let target_body = minimal_jwks_json("redirect-target");
    let server = tokio::spawn(async move {
        let (mut first, _) = listener.accept().await.expect("accept first request");
        let first_request = read_http_request(&mut first).await;
        write_http_response(
            &mut first,
            "302 Found",
            &format!("Location: http://localhost:{port}/target\r\n"),
            "",
        )
        .await;

        // If redirects are followed, this second response makes that behavior
        // observable as a successful fetch instead of a network failure.
        let (mut second, _) = listener.accept().await.expect("accept redirect target");
        let second_request = read_http_request(&mut second).await;
        write_http_response(&mut second, "200 OK", "", &target_body).await;
        (first_request, second_request)
    });

    assert_eq!(
        HttpJwksFetcher::new()
            .fetch_for_contract(&contract)
            .await
            .unwrap_err(),
        JwksFetchError::NetworkError,
        "3xx responses are rejected without following Location"
    );
    server.abort();
    assert!(server.await.unwrap_err().is_cancelled());

    let listener = bind_localhost_listener().await;
    let port = listener.local_addr().expect("listener address").port();
    let uri = format!("http://localhost:{port}/jwks.json");
    let contract = mesh_contract(&uri);
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("accept non-2xx request");
        let request = read_http_request(&mut stream).await;
        write_http_response(&mut stream, "503 Service Unavailable", "", "").await;
        request
    });
    assert_eq!(
        HttpJwksFetcher::new()
            .fetch_for_contract(&contract)
            .await
            .unwrap_err(),
        JwksFetchError::NetworkError
    );
    let request = server.await.expect("non-2xx server task");
    assert!(request.starts_with("GET /jwks.json HTTP/1.1\r\n"));
}

#[tokio::test(flavor = "current_thread")]
async fn production_fetch_wrapper_deadline_covers_waiting_for_response() {
    use std::time::Duration;

    let listener = bind_localhost_listener().await;
    let port = listener.local_addr().expect("listener address").port();
    let uri = format!("http://localhost:{port}/jwks.json");
    let contract = mesh_contract(&uri);
    let fetch =
        tokio::spawn(async move { HttpJwksFetcher::new().fetch_for_contract(&contract).await });

    let (mut stream, _) = tokio::time::timeout(Duration::from_secs(2), listener.accept())
        .await
        .expect("fetcher should connect before its whole-request deadline")
        .expect("accept request");
    let request = tokio::time::timeout(Duration::from_secs(2), read_http_request(&mut stream))
        .await
        .expect("fetcher should send request headers before waiting for response");
    assert!(request.starts_with("GET /jwks.json HTTP/1.1\r\n"));

    // Establish real socket readiness before freezing Tokio time. Starting
    // paused lets timer auto-advance race the OS accept/read readiness.
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(JWKS_REQUEST_TIMEOUT_SECS)).await;
    tokio::time::resume();
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(1), fetch)
            .await
            .expect("fetch wrapper must finish at its whole-request deadline")
            .expect("fetch task"),
        Err(JwksFetchError::NetworkError)
    );
    // Keep the peer open until the bounded assertion above. Without the
    // production deadline the fetch remains stalled instead of seeing EOF.
    drop(stream);
}

#[tokio::test]
async fn production_mesh_fetcher_keeps_response_cap() {
    let listener = bind_localhost_listener().await;
    let port = listener.local_addr().expect("listener address").port();
    let uri = format!("http://localhost:{port}/jwks.json");
    let contract = mesh_contract(&uri);
    let oversized_body = vec![b'x'; MAX_JWKS_RESPONSE_BYTES + 1];
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("accept oversized request");
        let request = read_http_request(&mut stream).await;
        use tokio::io::AsyncWriteExt as _;
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n")
            .await
            .expect("write response headers");
        stream
            .write_all(&oversized_body)
            .await
            .expect("write oversized streamed body");
        request
    });
    assert_eq!(
        HttpJwksFetcher::new()
            .fetch_for_contract(&contract)
            .await
            .unwrap_err(),
        JwksFetchError::ResponseTooLarge
    );
    let request = server.await.expect("oversized response server task");
    assert!(request.starts_with("GET /jwks.json HTTP/1.1\r\n"));
}
