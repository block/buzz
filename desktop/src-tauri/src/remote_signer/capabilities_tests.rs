use super::*;
use crate::builderlab::session::SessionValidity;
use base64::{engine::general_purpose::STANDARD, Engine};
use nostr::nips::nip44;
use tokio_util::sync::CancellationToken;

fn peer() -> Keys {
    Keys::parse("000000000000000000000000000000000000000000000000000000000000000b").unwrap()
}
fn cipher(plaintext: &str) -> String {
    nip44::encrypt(
        keys().secret_key(),
        &peer().public_key(),
        plaintext,
        nip44::Version::V2,
    )
    .unwrap()
}
fn auth_tag(owner: &Keys, agent: &PublicKey, conditions: &str) -> Value {
    // Independent fixture signing preimage, not the SDK compute/verify pair.
    use nostr::hashes::{sha256, Hash};
    let preimage = format!("nostr:agent-auth:{}:{conditions}", agent.to_hex());
    let digest = sha256::Hash::hash(preimage.as_bytes()).to_byte_array();
    let sig = owner.sign_schnorr(&nostr::secp256k1::Message::from_digest(digest));
    json!([
        "auth",
        owner.public_key().to_hex(),
        conditions,
        sig.to_string()
    ])
}

async fn exchange_server() -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let (tx, request) = oneshot::channel();
    let task = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let request = read_request(&mut stream).await;
        let body = &request.body;
        let value = if request
            .headers
            .starts_with("POST /api/goose/v1/buzz/identity/encrypt ")
        {
            let peer = PublicKey::from_hex(body["peer_pubkey"].as_str().unwrap()).unwrap();
            json!({"ciphertext":nip44::encrypt(keys().secret_key(), &peer, body["plaintext"].as_str().unwrap(), nip44::Version::V2).unwrap()})
        } else if request
            .headers
            .starts_with("POST /api/goose/v1/buzz/identity/decrypt ")
        {
            let peer = PublicKey::from_hex(body["peer_pubkey"].as_str().unwrap()).unwrap();
            json!({"plaintext":nip44::decrypt(keys().secret_key(), &peer, body["ciphertext"].as_str().unwrap()).unwrap()})
        } else {
            assert!(request
                .headers
                .starts_with("POST /api/goose/v1/buzz/identity/authorize-agent "));
            json!({"auth_tag":auth_tag(&keys(), &PublicKey::from_hex(body["agent_pubkey"].as_str().unwrap()).unwrap(), body["conditions"].as_str().unwrap())})
        };
        let _ = tx.send(request);
        stream
            .write_all(response(200, value.to_string()).as_bytes())
            .await
            .unwrap();
    });
    Server {
        base,
        request,
        task,
    }
}
fn assert_wire(request: Request, path: &str, body: Value) {
    assert!(request.headers.starts_with(&format!(
        "POST /api/goose/v1/buzz/identity/{path} HTTP/1.1\r\n"
    )));
    let headers = request.headers.to_ascii_lowercase();
    assert!(headers.contains(&format!("x-bb-session-credential: {CREDENTIAL}\r\n")));
    assert!(!headers.contains("authorization:"));
    assert!(!headers.contains("cookie:"));
    assert_eq!(request.body, body);
}

#[tokio::test]
async fn real_nip44_peer_self_unicode_and_local_library_limit_exchange() {
    for receiver in [keys(), peer()] {
        for text in [
            " \0\t\n\r é e\u{301} 漢 🐝 \u{fffd} ".to_owned(),
            "x".to_owned(),
            "x".repeat(33),
            "x".repeat(257),
            "x".repeat(65_408),
        ] {
            let pk = receiver.public_key();
            let mut api = exchange_server().await;
            let remote =
                ActiveUserSigner::new(Arc::new(signer(&format!("{}/api/goose/", api.base))))
                    .await
                    .unwrap();
            let encrypted = remote.signer().nip44_encrypt(&pk, &text).await.unwrap();
            assert_eq!(
                nip44::decrypt(receiver.secret_key(), &keys().public_key(), &encrypted).unwrap(),
                text
            );
            assert_wire(
                (&mut api.request).await.unwrap(),
                "encrypt",
                json!({"peer_pubkey":pk.to_hex(), "plaintext":text}),
            );
            let mut api = exchange_server().await;
            let remote = signer(&format!("{}/api/goose", api.base));
            let decrypted = remote.nip44_decrypt(&pk, &encrypted).await.unwrap();
            assert_eq!(decrypted.as_bytes(), text.as_bytes());
            assert_wire(
                (&mut api.request).await.unwrap(),
                "decrypt",
                json!({"peer_pubkey":pk.to_hex(), "ciphertext":encrypted}),
            );
        }
    }
}

#[tokio::test]
async fn oa_exact_path_required_empty_conditions_and_real_signature() {
    for conditions in ["", "kind=1&created_at>0&created_at<1"] {
        let mut api = exchange_server().await;
        let remote = signer(&format!("{}/api/goose", api.base));
        let tag = remote
            .authorize_agent(&peer().public_key(), conditions)
            .await
            .unwrap();
        assert_eq!(
            buzz_sdk_pkg::nip_oa::verify_auth_tag(
                &serde_json::to_string(&tag).unwrap(),
                &peer().public_key()
            )
            .unwrap(),
            keys().public_key()
        );
        assert_wire(
            (&mut api.request).await.unwrap(),
            "authorize-agent",
            json!({"agent_pubkey":peer().public_key().to_hex(), "conditions":conditions}),
        );
    }
}

#[tokio::test]
async fn oa_rejects_wrong_bindings_grammar_signature_and_shapes() {
    let valid = auth_tag(&keys(), &peer().public_key(), "kind=1");
    let mut cases = vec![
        json!({"auth_tag":auth_tag(&peer(), &keys().public_key(), "kind=1")}),
        json!({"auth_tag":auth_tag(&keys(), &Keys::generate().public_key(), "kind=1")}),
        json!({"auth_tag":auth_tag(&keys(), &peer().public_key(), "kind=2")}),
        json!({"auth_tag":["auth"]}),
        json!({"auth_tag":valid.to_string()}),
        json!({"auth_tag":valid, "extra":1}),
        json!({}),
    ];
    for (index, value) in [
        (0, json!("other")),
        (1, json!(keys().public_key().to_hex().to_uppercase())),
        (2, json!(null)),
        (3, json!("00".repeat(64))),
        (3, json!("BAD_SIGNATURE")),
    ] {
        let mut tag = valid.clone();
        tag[index] = value;
        cases.push(json!({"auth_tag":tag}));
    }
    for value in cases {
        let api = server(response(200, value.to_string())).await;
        assert!(signer(&api.base)
            .authorize_agent(&peer().public_key(), "kind=1")
            .await
            .is_err());
    }
    for conditions in [
        "kind=01",
        "kind=65536",
        "created_at<4294967296",
        "kind=1&&kind=2",
        " secret ",
    ] {
        let api = server(response(
            200,
            json!({"auth_tag":auth_tag(&keys(), &peer().public_key(), conditions)}).to_string(),
        ))
        .await;
        let error = signer(&api.base)
            .authorize_agent(&peer().public_key(), conditions)
            .await
            .unwrap_err();
        assert_eq!(error, RemoteSignerError::InvalidSignature);
        assert!(!format!("{error:?} {error}").contains(conditions));
    }
}

#[tokio::test]
async fn nip44_rejects_inputs_before_network_and_noncanonical_envelopes() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let remote = signer(&format!("http://{}", listener.local_addr().unwrap()));
    for text in [String::new(), "x".repeat(65_536), "漢".repeat(21_846)] {
        assert_eq!(
            remote.encrypt(&peer().public_key(), &text).await,
            Err(RemoteSignerError::InvalidInput)
        );
    }
    let valid = cipher(&"x".repeat(33));
    let mut bad_version = STANDARD.decode(&valid).unwrap();
    bad_version[0] = 1;
    let mut impossible = vec![0; 100];
    impossible[0] = 2;
    for text in [
        "secret".into(),
        format!(" {valid}"),
        format!("{valid}\n"),
        valid.trim_end_matches('=').to_owned(),
        format!("{valid}="),
        STANDARD.encode(bad_version),
        STANDARD.encode(impossible),
        "A".repeat(87_476),
    ] {
        assert_eq!(
            remote.decrypt(&peer().public_key(), &text).await,
            Err(RemoteSignerError::InvalidInput)
        );
    }
    assert!(
        tokio::time::timeout(Duration::from_millis(50), listener.accept())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn nip44_strict_responses_and_exact_plaintext_bounds() {
    for value in [
        json!({}),
        json!({"ciphertext":null}),
        json!({"ciphertext":"invalid"}),
        json!({"ciphertext":cipher("x"),"extra":true}),
        json!({"ciphertext":cipher(&"x".repeat(33))}),
    ] {
        let api = server(response(200, value.to_string())).await;
        assert!(signer(&api.base)
            .encrypt(&peer().public_key(), "x")
            .await
            .is_err());
    }
    let input = cipher("x");
    for value in [
        json!({}),
        json!({"plaintext":null}),
        json!({"plaintext":""}),
        json!({"plaintext":"x".repeat(65_536)}),
        json!({"plaintext":"x","extra":true}),
        json!({"plaintext":"x".repeat(33)}),
    ] {
        let api = server(response(200, value.to_string())).await;
        assert!(signer(&api.base)
            .decrypt(&peer().public_key(), &input)
            .await
            .is_err());
    }
    for body in [
        r#"{"plaintext":"\ud800"}"#,
        r#"{"plaintext":"x","plaintext":"y"}"#,
    ] {
        let api = server(response(200, body)).await;
        assert_eq!(
            signer(&api.base)
                .decrypt(&peer().public_key(), &input)
                .await,
            Err(RemoteSignerError::MalformedResponse)
        );
    }
}

fn validity() -> SessionValidity {
    SessionValidity {
        generation: 7,
        cancel: CancellationToken::new(),
        expires: chrono::Utc::now() + chrono::Duration::hours(1),
    }
}
async fn operation(remote: &RemoteSigner, op: &str) -> Result<(), RemoteSignerError> {
    match op {
        "encrypt" => remote.encrypt(&peer().public_key(), "x").await.map(|_| ()),
        "decrypt" => remote
            .decrypt(&peer().public_key(), &cipher("x"))
            .await
            .map(|_| ()),
        "authorize-agent" => remote
            .authorize_agent(&peer().public_key(), "")
            .await
            .map(|_| ()),
        _ => unreachable!(),
    }
}
fn success(op: &str) -> Value {
    match op {
        "encrypt" => json!({"ciphertext":cipher("x")}),
        "decrypt" => json!({"plaintext":"x"}),
        _ => json!({"auth_tag":auth_tag(&keys(), &peer().public_key(), "")}),
    }
}

#[tokio::test]
async fn all_capabilities_share_status_redaction_redirect_and_body_bounds() {
    for op in ["encrypt", "decrypt", "authorize-agent"] {
        for (status, error) in [
            (401, RemoteSignerError::Authentication(401)),
            (403, RemoteSignerError::Authentication(403)),
            (429, RemoteSignerError::TransientStatus(429)),
            (503, RemoteSignerError::TransientStatus(503)),
            (400, RemoteSignerError::HttpStatus(400)),
            (302, RemoteSignerError::HttpStatus(302)),
        ] {
            let target = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let wire = response(status, "DO_NOT_LOG_SECRET").replace(
                "Content-Type:",
                &format!(
                    "Location: http://{}/leak\r\nContent-Type:",
                    target.local_addr().unwrap()
                ),
            );
            let api = server(wire).await;
            let v = validity();
            let remote = signer(&api.base).with_validity(v.clone());
            assert_eq!(operation(&remote, op).await, Err(error));
            assert_eq!(v.cancel.is_cancelled(), status == 401 || status == 403);
            assert!(!format!("{error:?} {error} {remote:?}").contains("DO_NOT_LOG_SECRET"));
            assert!(
                tokio::time::timeout(Duration::from_millis(10), target.accept())
                    .await
                    .is_err()
            );
        }
        for wire in [
            response(200, "{"),
            "HTTP/1.1 200 OK\r\nContent-Length: 9999999\r\n\r\n".into(),
            response(200, " ".repeat(600_000)),
            "HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\nx".into(),
        ] {
            let api = server(wire).await;
            let v = validity();
            assert!(operation(&signer(&api.base).with_validity(v.clone()), op)
                .await
                .is_err());
            assert!(!v.cancel.is_cancelled());
        }
    }
}

#[tokio::test]
async fn all_capabilities_cancel_expire_and_reject_late_responses() {
    for op in ["encrypt", "decrypt", "authorize-agent"] {
        for expire in [false, true] {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let mut v = validity();
            if expire {
                v.expires = chrono::Utc::now() + chrono::Duration::seconds(1);
            }
            let remote = Arc::new(
                signer(&format!("http://{}", listener.local_addr().unwrap()))
                    .with_validity(v.clone()),
            );
            let held = remote.clone();
            let task = tokio::spawn(async move { operation(&held, op).await });
            let (mut stream, _) = listener.accept().await.unwrap();
            let request = read_request(&mut stream).await;
            assert!(request.headers.contains(CREDENTIAL));
            if !expire {
                v.cancel.cancel();
            }
            assert_eq!(
                tokio::time::timeout(Duration::from_secs(2), task)
                    .await
                    .unwrap()
                    .unwrap(),
                Err(RemoteSignerError::Authentication(401))
            );
            let _ = stream
                .write_all(response(200, success(op).to_string()).as_bytes())
                .await;
            assert_eq!(
                operation(&remote, op).await,
                Err(RemoteSignerError::Authentication(401))
            );
            assert!(
                tokio::time::timeout(Duration::from_millis(20), listener.accept())
                    .await
                    .is_err()
            );
        }
    }
}

#[tokio::test]
async fn server_generated_65535_fixture_is_not_shrunk_to_local_encrypt_limit() {
    // Authored cross-language probe (not the official corpus): Kotlin server
    // encoder, public fixture scalars 1 -> 2, plaintext 65535 ASCII x bytes.
    // Source: /tmp/buzz-identity-nip44-checkpoint/interop/kotlin-to-rust.json.
    let owner = Keys::parse(&format!("{:064x}", 1)).unwrap();
    let receiver = Keys::parse(&format!("{:064x}", 2)).unwrap();
    let encrypted = STANDARD.encode(include_bytes!("fixtures/server-max-v2.bin"));
    let text = "x".repeat(65_535);
    assert_eq!(
        nip44::decrypt(receiver.secret_key(), &owner.public_key(), &encrypted).unwrap(),
        text
    );
    let mut api = server(response(200, json!({"ciphertext":encrypted}).to_string())).await;
    let remote = RemoteSigner::new(
        &api.base,
        RemoteSignerSession::new(CREDENTIAL, owner.public_key()).unwrap(),
    )
    .unwrap();
    assert_eq!(
        remote.encrypt(&receiver.public_key(), &text).await.unwrap(),
        encrypted
    );
    assert_eq!((&mut api.request).await.unwrap().body["plaintext"], text);
    let api = server(response(200, json!({"plaintext":text}).to_string())).await;
    let remote = RemoteSigner::new(
        &api.base,
        RemoteSignerSession::new(CREDENTIAL, owner.public_key()).unwrap(),
    )
    .unwrap();
    assert_eq!(
        remote
            .decrypt(&receiver.public_key(), &encrypted)
            .await
            .unwrap(),
        text
    );
}

#[tokio::test]
async fn all_capabilities_share_deadline_and_streamed_limit() {
    for op in ["encrypt", "decrypt", "authorize-agent"] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let v = validity();
        let mut remote =
            signer(&format!("http://{}", listener.local_addr().unwrap())).with_validity(v.clone());
        // Shorten only the test's deadline, retaining the production client path.
        remote.client = Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_millis(100))
            .build()
            .unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            read_request(&mut stream).await;
            tokio::time::sleep(Duration::from_secs(2)).await;
        });
        assert_eq!(
            operation(&remote, op).await,
            Err(RemoteSignerError::Timeout)
        );
        assert!(!v.cancel.is_cancelled());
        server.abort();
        let body = " ".repeat(600_000);
        let api = server_chunked(&body).await;
        assert_eq!(
            operation(&signer(&api.base), op).await,
            Err(RemoteSignerError::ResponseTooLarge)
        );
    }
}

async fn server_chunked(body: &str) -> Server {
    server(format!("HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n{:x}\r\n{body}\r\n0\r\n\r\n", body.len())).await
}

#[tokio::test]
async fn invalid_utf8_response_is_rejected_not_lossily_replaced() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let remote = signer(&format!("http://{}", listener.local_addr().unwrap()));
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        read_request(&mut stream).await;
        let body = b"{\"plaintext\":\"\xff\"}";
        stream
            .write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                )
                .as_bytes(),
            )
            .await
            .unwrap();
        stream.write_all(body).await.unwrap();
    });
    assert_eq!(
        remote.decrypt(&peer().public_key(), &cipher("x")).await,
        Err(RemoteSignerError::MalformedResponse)
    );
    server.await.unwrap();
}
