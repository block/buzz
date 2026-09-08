use super::*;

fn config(dir: &Path) -> PkceOAuthConfig {
    PkceOAuthConfig {
        discovery_url: "https://example.com/.well-known".into(),
        client_id: "client".into(),
        scopes: vec!["read".into(), "write".into()],
        cache_namespace: "test".into(),
        cache_dir_override: Some(dir.to_owned()),
    }
}

fn legacy_identity(cfg: &PkceOAuthConfig) -> String {
    format!(
        "{}|{}|{}",
        cfg.discovery_url,
        cfg.client_id,
        cfg.scopes.join(",")
    )
}

#[test]
fn cache_keys_preserve_field_and_scope_boundaries() {
    let dir = tempfile::tempdir().unwrap();
    let first = config(dir.path());
    let mut scope_alias = first.clone();
    scope_alias.scopes = vec!["read,write".into()];
    let mut url_alias = first.clone();
    url_alias.discovery_url.push_str("|client");
    url_alias.client_id = "other".into();
    let mut client_alias = first.clone();
    client_alias.client_id = "client|other".into();
    let mut empty_array = first.clone();
    empty_array.scopes.clear();
    let mut empty_element = empty_array.clone();
    empty_element.scopes.push(String::new());
    let mut client_scope = first.clone();
    client_scope.client_id.push_str("|read");
    client_scope.scopes = vec!["write".into()];
    let mut scope_client = first.clone();
    scope_client.scopes = vec!["read|write".into()];

    for (a, b) in [
        (first, scope_alias),
        (url_alias, client_alias),
        (empty_array, empty_element),
        (client_scope, scope_client),
    ] {
        assert_eq!(legacy_identity(&a), legacy_identity(&b));
        assert_ne!(cache_path_for(&a).unwrap(), cache_path_for(&b).unwrap());
    }
}

#[tokio::test]
async fn colliding_configs_do_not_share_tokens_or_coordination_files() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = config(dir.path());
    let source = PkceOAuthTokenSource::new(cfg.clone()).unwrap();
    source
        .save(
            &mut *source.state.lock().await,
            CachedToken {
                access_token: "only-for-two-scopes".into(),
                refresh_token: Some("only-refresh-two-scopes".into()),
                expires_at: None,
            },
        )
        .unwrap();
    let same = PkceOAuthTokenSource::new(cfg.clone()).unwrap();
    assert_eq!(
        same.state.lock().await.as_ref().unwrap().access_token,
        "only-for-two-scopes"
    );
    let mut other = cfg.clone();
    other.scopes = vec!["read,write".into()];
    assert_eq!(legacy_identity(&cfg), legacy_identity(&other));
    let alias = PkceOAuthTokenSource::new(other).unwrap();
    assert!(alias.state.lock().await.is_none());
    for suffix in ["lock", "cooldown", "attempt"] {
        assert_ne!(
            append_ext(&source.cache_path, suffix),
            append_ext(&alias.cache_path, suffix)
        );
    }
}

#[tokio::test]
async fn legacy_cache_is_not_imported() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = config(dir.path());
    let namespace = dir.path().join(&cfg.cache_namespace);
    fs::create_dir_all(&namespace).unwrap();
    let hash = hex::encode(sha2::Sha256::digest(legacy_identity(&cfg)));
    let old = namespace.join(format!("{hash}.json"));
    fs::write(
        &old,
        br#"{"access_token":"ambiguous","refresh_token":"ambiguous","expires_at":null}"#,
    )
    .unwrap();
    let source = PkceOAuthTokenSource::new(cfg).unwrap();
    assert!(source.state.lock().await.is_none());
    assert!(!source.cache_path.exists());
    assert!(old.exists());
}
