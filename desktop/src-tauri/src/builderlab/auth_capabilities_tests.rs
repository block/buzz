//! Authenticated production owner cancellation, same-key replacement, and exact
//! credential reuse across the newly enabled adapters. Loopback only.
use super::*;
use crate::remote_signer::{RemoteSigner, RemoteSignerSession};

fn captured_remote(api: &MockApi, owner: &BuilderlabSession) -> RemoteSigner {
    let captured = owner.snapshot().unwrap().unwrap();
    RemoteSigner::new(
        api.base.as_str(),
        RemoteSignerSession::new(&captured.credential, captured.signer.unwrap().public_key())
            .unwrap(),
    )
    .unwrap()
    .with_validity(captured.validity)
}

async fn call(
    remote: &RemoteSigner,
    op: &str,
    peer: &Keys,
    owner: nostr::PublicKey,
) -> Result<(), String> {
    use nostr::NostrSigner;
    match op {
        "encrypt" => remote
            .nip44_encrypt(&peer.public_key(), "hello �")
            .await
            .map(|_| ())
            .map_err(|e| e.to_string()),
        "decrypt" => {
            let ciphertext = nostr::nips::nip44::encrypt(
                peer.secret_key(),
                &owner,
                "hello �",
                nostr::nips::nip44::Version::V2,
            )
            .unwrap();
            remote
                .nip44_decrypt(&peer.public_key(), &ciphertext)
                .await
                .map(|_| ())
                .map_err(|e| e.to_string())
        }
        _ => remote
            .authorize_agent(&peer.public_key(), "")
            .await
            .map(|_| ())
            .map_err(|e| e.to_string()),
    }
}

#[tokio::test]
async fn native_owner_all_capabilities_logout_and_same_key_reauth_drop_old_result() {
    for op in ["encrypt", "decrypt", "authorize-agent"] {
        for replace in [false, true] {
            let api = MockApi::new().await;
            let owner = BuilderlabSession::default();
            api.login(&owner, "A").await.unwrap();
            let remote = Arc::new(captured_remote(&api, &owner));
            let path = format!("v1/buzz/identity/{op}");
            api.hold(&path);
            let held = remote.clone();
            let pk = api.state.keys.public_key();
            let task = tokio::spawn(async move { call(&held, op, &Keys::generate(), pk).await });
            api.arrived().await;
            if replace {
                api.login(&owner, "B").await.unwrap();
                assert_eq!(owner.active_signer().unwrap().public_key(), pk);
            } else {
                owner.clear().unwrap();
            }
            assert!(tokio::time::timeout(Duration::from_secs(2), task)
                .await
                .unwrap()
                .unwrap()
                .is_err());
            api.release();
            let count = api.state.requests.lock().unwrap().len();
            assert!(call(&remote, op, &Keys::generate(), pk).await.is_err());
            assert_eq!(api.state.requests.lock().unwrap().len(), count);
            if replace {
                call(&captured_remote(&api, &owner), op, &Keys::generate(), pk)
                    .await
                    .unwrap();
            }
            let requests = api.state.requests.lock().unwrap();
            let credentials: Vec<_> = requests
                .iter()
                .filter(|r| r.0 == path)
                .map(|r| r.1.as_deref().unwrap())
                .collect();
            assert_eq!(
                credentials,
                if replace {
                    vec!["credential-A", "credential-B"]
                } else {
                    vec!["credential-A"]
                }
            );
        }
    }
}

#[tokio::test]
async fn authenticated_active_signer_nip44_uses_discovered_owner_session() {
    let api = MockApi::new().await;
    let owner = BuilderlabSession::default();
    api.login(&owner, "A").await.unwrap();
    let captured = owner.active_signer().unwrap();
    let peer = Keys::generate();
    let text = " a\0 e\u{301} � 🐝 ";
    let encrypted = captured
        .signer()
        .nip44_encrypt(&peer.public_key(), text)
        .await
        .unwrap();
    assert_eq!(
        nostr::nips::nip44::decrypt(peer.secret_key(), &captured.public_key(), &encrypted).unwrap(),
        text
    );
    assert_eq!(
        captured
            .signer()
            .nip44_decrypt(&peer.public_key(), &encrypted)
            .await
            .unwrap(),
        text
    );
    owner.clear().unwrap();
    assert!(captured
        .signer()
        .nip44_encrypt(&peer.public_key(), text)
        .await
        .is_err());
    let requests = api.state.requests.lock().unwrap();
    for request in requests
        .iter()
        .filter(|r| r.0.ends_with("/encrypt") || r.0.ends_with("/decrypt"))
    {
        assert_eq!(request.1.as_deref(), Some("credential-A"));
    }
}
