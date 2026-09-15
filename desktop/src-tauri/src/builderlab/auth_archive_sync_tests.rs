//! Workspace authority is separate from authentication and never survives reauth.
use super::*;

#[tokio::test]
async fn workspace_activation_requires_current_generation_and_relay() {
    let api = MockApi::new().await;
    let session = BuilderlabSession::default();
    api.login(&session, "first").await.unwrap();
    let signer = session.active_signer().unwrap();
    let relay = "wss://relay.example";
    assert!(!session.workspace_active().unwrap());
    assert!(session
        .workspace_signer(signer.generation(), relay)
        .is_err());
    session.activate_workspace(&signer, relay).unwrap();
    assert!(session.workspace_active().unwrap());
    session
        .workspace_signer(signer.generation(), relay)
        .unwrap();
    assert!(session.workspace_signer(None, relay).is_err());
    assert!(session
        .workspace_signer(signer.generation(), "wss://other.example")
        .is_err());
    session.clear().unwrap();
    assert!(!session.workspace_active().unwrap());
    assert!(session.check_signer(&signer).is_err());
    api.login(&session, "replacement").await.unwrap();
    assert!(session.activate_workspace(&signer, relay).is_err());
    assert!(session
        .workspace_signer(signer.generation(), relay)
        .is_err());
    let current = session.active_signer().unwrap();
    assert_eq!(current.public_key(), signer.public_key());
    assert_ne!(current.generation(), signer.generation());
    session.activate_workspace(&current, relay).unwrap();
    session
        .workspace_signer(current.generation(), relay)
        .unwrap();
}
