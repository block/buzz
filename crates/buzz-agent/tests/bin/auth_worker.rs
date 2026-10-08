//! Test-only helper: a real second process that runs the PUBLIC auth
//! coordinator (`PkceOAuthTokenSource::acquire_with_intent`) against a shared
//! temp cache, so the auth tests can prove the *cross-process* single-flight
//! contract end-to-end rather than with two in-process handles.
//!
//! The in-process `INFLIGHT` registry coalesces same-key callers within one
//! process before they ever reach the file lock, so two `PkceOAuthTokenSource`
//! instances in one test do NOT exercise the cross-process protocol (the OS
//! advisory lock and the on-disk cache re-read). This binary is a genuine
//! second process: it contends on the same `flock`/`LockFileEx` and reads/writes
//! the same private cache file the parent coordinator does.
//!
//! The browser step is scripted (no real window): the opener drives the
//! loopback callback exactly as a real browser would, and its launch count is
//! reported back so a test can assert "exactly one browser across processes".
//!
//! Env contract (all required unless noted):
//!   AUTH_WORKER_DISCOVERY_URL   — OIDC discovery URL (the parent stub).
//!   AUTH_WORKER_CACHE_DIR       — shared cache dir (`cache_dir_override`).
//!   AUTH_WORKER_NAMESPACE       — cache namespace.
//!   AUTH_WORKER_CLIENT_ID       — OAuth client id.
//!   AUTH_WORKER_SCOPES          — comma-separated scopes.
//!   AUTH_WORKER_INTENT          — auto | userinitiated | headless.
//!   AUTH_WORKER_SCRIPT          — approve | deny | failopen.
//!   AUTH_WORKER_RESULT          — path to write the JSON outcome to.
//!   AUTH_WORKER_REJECTED        — (optional) rejected token bytes passed to
//!                                 `acquire_with_intent`; absent means no rejection.
//!   AUTH_WORKER_READY_MARKER    — (optional) written once the source is built,
//!                                 before acquisition, so the parent can release
//!                                 several workers into a genuine lock race.
//!   AUTH_WORKER_START_MARKER    — (optional) acquisition blocks until this file
//!                                 exists, so multiple workers begin together.
//!   AUTH_WORKER_LAUNCHED_MARKER — (optional) written when the browser opener
//!                                 fires (i.e. this process holds the lock and is
//!                                 mid-flow), so the parent can queue behind it.
//!   AUTH_WORKER_PROCEED_MARKER  — (optional) the scripted callback is withheld
//!                                 until this file exists, so the parent can
//!                                 confirm another process is already waiting on
//!                                 the lock before this one resolves.
//!   AUTH_WORKER_SNAPSHOT_MARKER — (optional) a file path; when set, a tracing
//!                                 layer intercepts the `acquire_leader_snapshot`
//!                                 event emitted by `auth.rs` after the attempt-
//!                                 generation snapshot is taken (and before the
//!                                 cross-process lock is acquired) and writes this
//!                                 file once. Lets the parent observe that this
//!                                 process has committed its snapshot-gen and is
//!                                 about to queue on the lock.
//!
//! Result JSON: `{ "result": "ok"|"<error_code>", "bearer": <string|null>,
//! "launches": <u64> }`.

use std::fs;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use buzz_agent::auth::{AuthIntent, BrowserOpener, PkceOAuthConfig, PkceOAuthTokenSource};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

/// Tracing layer that writes a file once when it sees the
/// `buzz_agent::auth::acquire_leader_snapshot` event emitted by
/// `acquire_leader` immediately after the attempt-generation snapshot is fixed
/// and before the cross-process lock is acquired.  Installed only when
/// `AUTH_WORKER_SNAPSHOT_MARKER` is set, so normal test runs incur no overhead.
struct SnapshotMarkerLayer {
    path: PathBuf,
    written: AtomicBool,
}

impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for SnapshotMarkerLayer {
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        if event.metadata().target() == "buzz_agent::auth::acquire_leader_snapshot"
            && !self.written.swap(true, Ordering::SeqCst)
        {
            let _ = fs::write(&self.path, b"snapshotted");
        }
    }
}

/// What the scripted "user" does when the coordinator opens a browser.
#[derive(Clone, Copy)]
enum Script {
    Approve,
    Deny,
    FailToOpen,
}

/// A [`BrowserOpener`] that counts launches and drives the loopback callback on
/// a background thread — the same technique as the in-crate test opener, but
/// with two optional cross-process barriers so the parent can order events:
/// `launched_marker` announces that this process holds the lock and has opened
/// the browser, and `proceed_marker` withholds the callback until the parent
/// signals it has queued another process behind the lock.
struct WorkerOpener {
    script: Script,
    calls: Arc<AtomicU64>,
    launched_marker: Option<PathBuf>,
    proceed_marker: Option<PathBuf>,
}

impl BrowserOpener for WorkerOpener {
    fn open(&self, url: &str) -> Result<(), String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if let Some(marker) = &self.launched_marker {
            fs::write(marker, b"launched").expect("write launched marker");
        }
        let query = match self.script {
            Script::FailToOpen => return Err("no browser available".into()),
            Script::Approve => "code=scripted-code",
            Script::Deny => "error=access_denied",
        };
        let parsed = url::Url::parse(url).expect("authorize URL must parse");
        let redirect = parsed
            .query_pairs()
            .find(|(k, _)| k == "redirect_uri")
            .map(|(_, v)| v.into_owned())
            .expect("authorize URL carries redirect_uri");
        let state = parsed
            .query_pairs()
            .find(|(k, _)| k == "state")
            .map(|(_, v)| v.into_owned())
            .expect("authorize URL carries state");
        let redirect = url::Url::parse(&redirect).expect("redirect_uri must parse");
        let port = redirect.port().expect("loopback redirect carries a port");
        let request = format!(
            "GET /?{query}&state={state} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
        );
        let proceed = self.proceed_marker.clone();
        std::thread::spawn(move || {
            // Hold the callback until the parent has confirmed another process
            // is already queued behind the lock (bounded so a missing signal
            // can't wedge the test past the browser timeout).
            if let Some(marker) = proceed {
                for _ in 0..6000 {
                    if marker.exists() {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
            }
            if let Ok(mut sock) = TcpStream::connect(("127.0.0.1", port)) {
                let _ = sock.write_all(request.as_bytes());
                let _ = sock.flush();
                let mut discard = Vec::new();
                let _ = sock.read_to_end(&mut discard);
            }
        });
        Ok(())
    }
}

fn env(key: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| panic!("{key} set"))
}

#[tokio::main]
async fn main() {
    // If the parent test set AUTH_WORKER_SNAPSHOT_MARKER, install a tracing
    // subscriber layer that fires when the coordinator emits its pre-lock
    // snapshot event and writes the marker file.
    if let Ok(marker_path) = std::env::var("AUTH_WORKER_SNAPSHOT_MARKER") {
        tracing_subscriber::registry()
            .with(SnapshotMarkerLayer {
                path: PathBuf::from(marker_path),
                written: AtomicBool::new(false),
            })
            .init();
    }

    let intent = match env("AUTH_WORKER_INTENT").as_str() {
        "auto" => AuthIntent::Auto,
        "userinitiated" => AuthIntent::UserInitiated,
        "headless" => AuthIntent::Headless,
        other => panic!("unknown AUTH_WORKER_INTENT: {other}"),
    };
    let script = match env("AUTH_WORKER_SCRIPT").as_str() {
        "approve" => Script::Approve,
        "deny" => Script::Deny,
        "failopen" => Script::FailToOpen,
        other => panic!("unknown AUTH_WORKER_SCRIPT: {other}"),
    };
    let result_path = PathBuf::from(env("AUTH_WORKER_RESULT"));
    let start_marker = std::env::var("AUTH_WORKER_START_MARKER")
        .ok()
        .map(PathBuf::from);
    let ready_marker = std::env::var("AUTH_WORKER_READY_MARKER")
        .ok()
        .map(PathBuf::from);

    let calls = Arc::new(AtomicU64::new(0));
    let opener = WorkerOpener {
        script,
        calls: calls.clone(),
        launched_marker: std::env::var("AUTH_WORKER_LAUNCHED_MARKER")
            .ok()
            .map(PathBuf::from),
        proceed_marker: std::env::var("AUTH_WORKER_PROCEED_MARKER")
            .ok()
            .map(PathBuf::from),
    };

    let cfg = PkceOAuthConfig {
        discovery_url: env("AUTH_WORKER_DISCOVERY_URL"),
        client_id: env("AUTH_WORKER_CLIENT_ID"),
        scopes: env("AUTH_WORKER_SCOPES")
            .split(',')
            .map(str::to_owned)
            .collect(),
        cache_namespace: env("AUTH_WORKER_NAMESPACE"),
        cache_dir_override: Some(PathBuf::from(env("AUTH_WORKER_CACHE_DIR"))),
    };
    let src = PkceOAuthTokenSource::new_with(cfg, Arc::new(opener)).expect("build token source");

    // Announce readiness, then wait for the parent's release so several workers
    // hit the lock together — a genuine race rather than staggered spawns.
    if let Some(marker) = &ready_marker {
        fs::write(marker, b"ready").expect("write ready marker");
    }
    if let Some(marker) = start_marker {
        for _ in 0..6000 {
            if marker.exists() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    let (result, bearer) = match src
        .acquire_with_intent(
            intent,
            std::env::var("AUTH_WORKER_REJECTED").ok().as_deref(),
        )
        .await
    {
        Ok(token) => ("ok".to_owned(), Some(token)),
        Err(e) => (e.code().to_owned(), None),
    };
    let body = serde_json::json!({
        "result": result,
        "bearer": bearer,
        "launches": calls.load(Ordering::SeqCst),
    });
    fs::write(&result_path, serde_json::to_vec(&body).unwrap()).expect("write result file");
}
