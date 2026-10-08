//! Real fire-and-forget trigger, real HTTP bodies, and real roster persistence.
//! Run in a child process so the home directory, Tauri runtime and global nest
//! owner cannot touch the user's profile or race unrelated package tests.
use super::*;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use std::time::Duration;

#[test]
fn real_trigger_coalesces_and_survives_a_stalled_old_workspace() {
    const CHILD: &str = "BUZZ_NEST_REGEN_TEST_HOME";
    if let Some(home) = std::env::var_os(CHILD) {
        let home = PathBuf::from(home);
        assert_eq!(std::env::var_os("HOME").unwrap(), home.as_os_str());
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            tauri::async_runtime::set(tokio::runtime::Handle::current());
            exercise_real_trigger(&home).await;
        });
        return;
    }
    let home = tempfile::tempdir().unwrap();
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "managed_agents::nest::regen_trigger_tests::real_trigger_coalesces_and_survives_a_stalled_old_workspace", "--nocapture"])
        .env(CHILD, home.path()).env("HOME", home.path())
        .env("XDG_DATA_HOME", home.path()).env("APPDATA", home.path())
        .env("LOCALAPPDATA", home.path())
        .env_remove("BUZZ_PRIVATE_KEY").env_remove("BUZZ_AUTH_TAG")
        .env_remove("BUZZ_RELAY_URL").env_remove("BUZZ_NETWORK_TRACE")
        .output().unwrap();
    assert!(
        output.status.success(),
        "child failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

async fn exercise_real_trigger(home: &Path) {
    use axum::{
        body::{Body, Bytes},
        response::Response,
        routing::{get, post},
        Json, Router,
    };
    let old_hits = Arc::new(AtomicUsize::new(0));
    let old_started = Arc::new(tokio::sync::Notify::new());
    let hits = old_hits.clone();
    let started = old_started.clone();
    let old = Router::new().route(
        "/",
        get(move || {
            hits.fetch_add(1, Ordering::SeqCst);
            started.notify_one();
            async {
                // Headers succeed but the NIP-11 body never ends. Only the real
                // nest-owned timeout can release this read; the test never does.
                Response::new(Body::from_stream(futures_util::stream::pending::<
                    Result<Bytes, std::io::Error>,
                >()))
            }
        }),
    );
    let keys = nostr::Keys::generate();
    let relay_self = keys.public_key().to_hex();
    let snapshot = nostr::EventBuilder::new(nostr::Kind::Custom(13535), "")
        .sign_with_keys(&keys)
        .unwrap();
    let query_hits = Arc::new(AtomicUsize::new(0));
    let hits = query_hits.clone();
    let latest = Router::new()
        .route(
            "/",
            get(move || {
                let value = relay_self.clone();
                async move { Json(serde_json::json!({"self": value})) }
            }),
        )
        .route(
            "/query",
            post(move || {
                hits.fetch_add(1, Ordering::SeqCst);
                let value = snapshot.clone();
                async move { Json(serde_json::json!([value])) }
            }),
        );
    async fn serve(router: Router) -> (String, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}", listener.local_addr().unwrap());
        (
            url,
            tokio::spawn(async move {
                axum::serve(listener, router).await.unwrap();
            }),
        )
    }
    let (old_url, old_server) = serve(old).await;
    let (latest_url, latest_server) = serve(latest).await;
    // Inject paths rather than relying on HOME/APPDATA: Windows known-folder
    // APIs ignore those environment overrides. PathResolver joins the mock
    // identifier onto data_dir; an absolute identifier replaces that base.
    let app_data = home.join("app-data");
    let mut context = tauri::test::mock_context(tauri::test::noop_assets());
    context.config_mut().identifier = app_data.to_str().unwrap().to_owned();
    NEST_DIR.set(Some(home.join("nest"))).unwrap();
    let state = crate::app_state::build_app_state();
    *state.relay_url_override.lock().unwrap() = Some(old_url.clone());
    let app = tauri::test::mock_builder()
        .manage(state)
        .build(context)
        .unwrap();
    assert_eq!(app.path().app_data_dir().unwrap(), app_data);
    let nest = nest_dir().unwrap();
    assert_eq!(nest, home.join("nest"));
    fs::create_dir_all(&nest).unwrap();
    let file = nest.join("AGENTS.md");
    fs::write(&file, "# User instructions\n").unwrap();

    try_regenerate_nest(app.handle());
    tokio::time::timeout(Duration::from_secs(3), old_started.notified())
        .await
        .unwrap();
    for _ in 0..288 {
        try_regenerate_nest(app.handle());
    }
    *app.state::<AppState>().relay_url_override.lock().unwrap() = Some(latest_url.clone());
    try_regenerate_nest(app.handle());

    // Wait through the actual operation deadline. Do not release the stalled
    // response or manufacture a fresh worker; its pending latest trigger must
    // progress on its own and finish with exactly one new-workspace query.
    tokio::time::timeout(NEST_ARCHIVE_TIMEOUT + Duration::from_secs(5), async {
        loop {
            let content = fs::read_to_string(&file).unwrap();
            assert!(!content.contains(&old_url), "stale workspace was committed");
            if content.contains(&latest_url) && !NEST_REGEN.state.lock().unwrap().running {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(old_hits.load(Ordering::SeqCst), 1);
    assert_eq!(query_hits.load(Ordering::SeqCst), 1);
    assert!(fs::read_to_string(&file)
        .unwrap()
        .starts_with("# User instructions"));

    // Idle-to-running through the real trigger must still work.
    try_regenerate_nest(app.handle());
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if !NEST_REGEN.state.lock().unwrap().running {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(query_hits.load(Ordering::SeqCst), 2);
    old_server.abort();
    latest_server.abort();
}
