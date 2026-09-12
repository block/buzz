//! No Keychain, real browser, retained HOME, or production provider access.
use axum::{
    extract::Form,
    routing::{get, post},
    Json, Router,
};
use buzz_agent::auth::{
    AuthIntent, BrowserOpener, OAuthTokenCustody, PkceOAuthConfig, PkceOAuthTokenSource,
};
use buzz_agent::types::AgentError;
use serde_json::json;
use std::sync::{Arc, Mutex};
#[derive(Default)]
struct Store {
    value: Mutex<Option<String>>,
    writes: Mutex<usize>,
    fail: bool,
}
impl OAuthTokenCustody for Store {
    fn load(&self) -> Result<Option<String>, AgentError> {
        Ok(self.value.lock().unwrap().clone())
    }
    fn store(&self, value: &str) -> Result<(), AgentError> {
        if self.fail {
            return Err(AgentError::Llm("synthetic store denial".into()));
        }
        *self.value.lock().unwrap() = Some(value.to_owned());
        assert_eq!(self.load()?.as_deref(), Some(value));
        *self.writes.lock().unwrap() += 1;
        Ok(())
    }
}
struct Browser {
    bad_state: bool,
}
impl BrowserOpener for Browser {
    fn open(&self, value: &str) -> Result<(), String> {
        let url = url::Url::parse(value).unwrap();
        let q: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
        assert_eq!(q["code_challenge_method"], "S256");
        assert_eq!(q["response_type"], "code");
        assert!(q["code_challenge"].len() >= 43);
        let callback = format!(
            "{}?code=synthetic-code&state={}",
            q["redirect_uri"],
            if self.bad_state { "wrong" } else { &q["state"] }
        );
        tokio::spawn(async move {
            reqwest::get(callback).await.unwrap();
        });
        Ok(())
    }
}
#[tokio::test]
async fn browser_refresh_custody_and_failure_never_touch_token_files() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let origin = base.clone();
    let app = Router::new().route("/discovery",get(move || { let b=origin.clone(); async move { Json(json!({"authorization_endpoint":format!("{b}/authorize"),"token_endpoint":format!("{b}/token")})) } }))
        .route("/token",post(|Form(f):Form<std::collections::HashMap<String,String>>| async move {
            let refresh = f["grant_type"] == "refresh_token";
            if !refresh { assert_eq!(f["code"],"synthetic-code"); assert!(f["code_verifier"].len() >= 43); }
            Json(json!({"access_token":if refresh {"refreshed-secret"} else {"login-secret"},"refresh_token":"refresh-secret","expires_in":3600}))
        }));
    let app = app
        .route("/api/ai-gateway/v2/endpoints", get(|headers: axum::http::HeaderMap| async move {
            assert_eq!(headers["authorization"], "Bearer refreshed-secret");
            Json(json!({"endpoints":[{"name":"allowed-text"},{"name":"text-embedding-family"}],"next_page_token":null}))
        }))
        .route("/api/2.1/unity-catalog/model-services", get(|headers: axum::http::HeaderMap| async move {
            assert_eq!(headers["authorization"], "Bearer refreshed-secret");
            Json(json!({"model_services":[{"name":"model-services/catalog.schema.text"}],"next_page_token":null}))
        }));
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let temp = tempfile::tempdir().unwrap();
    let config = |suffix: &str| PkceOAuthConfig {
        discovery_url: format!("{base}/discovery"),
        client_id: "fixture".into(),
        scopes: vec!["all-apis".into()],
        cache_namespace: suffix.into(),
        cache_dir_override: Some(temp.path().to_owned()),
    };
    let store = Arc::new(Store::default());
    let source = PkceOAuthTokenSource::new_with_custody(
        config("good"),
        Arc::new(Browser { bad_state: false }),
        store.clone(),
    )
    .unwrap();
    assert_eq!(
        source
            .acquire_with_intent(AuthIntent::UserInitiated, None)
            .await
            .unwrap(),
        "login-secret"
    );
    assert_eq!(*store.writes.lock().unwrap(), 1);
    assert_eq!(
        source
            .acquire_with_intent(AuthIntent::Headless, Some("login-secret"))
            .await
            .unwrap(),
        "refreshed-secret"
    );
    assert_eq!(*store.writes.lock().unwrap(), 3);
    let reopened = PkceOAuthTokenSource::new_with_custody(
        config("reopen"),
        Arc::new(Browser { bad_state: true }),
        store.clone(),
    )
    .unwrap();
    assert_eq!(
        reopened
            .acquire_with_intent(AuthIntent::Headless, None)
            .await
            .unwrap(),
        "refreshed-secret"
    );
    // Explicit local configuration; never inspect a real DATABRICKS_HOST.
    std::env::set_var("BUZZ_AGENT_PROVIDER", "databricks_v2");
    std::env::set_var("DATABRICKS_HOST", &base);
    std::env::set_var("BUZZ_AGENT_MODEL", "catalog-only");
    std::env::remove_var("DATABRICKS_TOKEN");
    std::env::remove_var("DATABRICKS_MODEL_FILTER");
    let cfg = buzz_agent::config::Config::from_env().unwrap();
    let models = buzz_agent::catalog::discover_databricks_models_with_token_source(&cfg, reopened)
        .await
        .unwrap();
    let ids: Vec<_> = models.iter().map(|m| m.id.as_str()).collect();
    assert!(ids.contains(&"allowed-text"));
    assert!(ids.contains(&"catalog.schema.text"));
    assert!(!ids.contains(&"text-embedding-family"));
    for (namespace, bad_state, deny) in [("state", true, false), ("denied", false, true)] {
        let store = Arc::new(Store {
            fail: deny,
            ..Default::default()
        });
        let source = PkceOAuthTokenSource::new_with_custody(
            config(namespace),
            Arc::new(Browser { bad_state }),
            store.clone(),
        )
        .unwrap();
        assert!(source
            .acquire_with_intent(AuthIntent::UserInitiated, None)
            .await
            .is_err());
        assert!(store.load().unwrap().is_none());
    }
    fn walk(path: &std::path::Path) {
        for entry in std::fs::read_dir(path).unwrap() {
            let p = entry.unwrap().path();
            if p.is_dir() {
                walk(&p)
            } else {
                let bytes = std::fs::read(p).unwrap();
                for secret in ["login-secret", "refreshed-secret", "refresh-secret"] {
                    assert!(!String::from_utf8_lossy(&bytes).contains(secret));
                }
            }
        }
    }
    walk(temp.path());
    server.abort();
    let _ = server.await;
}
