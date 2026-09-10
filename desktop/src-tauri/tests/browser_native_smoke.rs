//! Harness-free (`harness = false`) native smoke test for the plugin browser
//! adapter: proves the security properties of the real native webview by
//! actually running it, not by asserting against a mock.
//!
//! libtest spawns each `#[test]` on a worker thread, but `tao`'s `EventLoop`
//! must be created on the process main thread on macOS
//! (`tao-0.35.3/src/platform_impl/macos/event_loop.rs:167`). With no harness,
//! this `fn main()` runs directly on the process main thread, so it can host
//! a real window, actually pump the WebKit main run loop, and drive
//! `browser_webview::build_child` — the same function production `open`
//! calls — against a local fixture server.
//!
//! Every stage below waits on a real observed signal (a `PageLoad` callback,
//! a fixture-server request, an in-page report, an evaluated page-state
//! value) with a bounded per-stage deadline, never a fixed sleep whose
//! length is chosen to make an assertion pass. `event_loop.run_return`
//! returns control to this file after processing pending native events, so
//! `pump_once` below repeatedly re-enters the real WebKit/AppKit run loop
//! between condition checks instead of blocking it.
//!
//! Everything here is test-only; a downstream reader should not infer any of
//! it as guidance for production code.

#[cfg(not(target_os = "macos"))]
fn main() {
    println!("browser_native_smoke: skipped, macOS only");
}

#[cfg(target_os = "macos")]
fn main() {
    macos::run();
}

#[cfg(target_os = "macos")]
mod macos {
    use std::path::{Path, PathBuf};
    use std::sync::atomic::AtomicBool;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    use axum::extract::State;
    use axum::http::header;
    use axum::response::{Html, IntoResponse, Response};
    use axum::routing::{get, post};
    use axum::Router;
    use tao::event_loop::{ControlFlow, EventLoop};
    use tao::platform::run_return::EventLoopExtRunReturn;
    use tao::window::WindowBuilder;

    use buzz_lib::plugin_host::browser_webview;
    use buzz_lib::plugin_host::types::{Bounds, PageLoad};
    use buzz_lib::plugin_host::{
        is_admitted, PluginError, PluginHost, PluginHostConfig, ResolveKind,
    };

    const PLUGIN_ID: &str = "dev.example.smoketest";
    const CONTRIBUTION_ID: &str = "web";
    const DENIED_MARKER: &str = "denied";
    const ISOLATION_MARKER: &str = "isolation";
    const DOWNLOAD_MARKER: &str = "download";
    const PAGE_A_MARKER: &str = "page-a";
    const PAGE_B_MARKER: &str = "page-b";

    /// Overall wall-clock budget for the whole run, independent of any
    /// single stage's own deadline, so a stuck stage cannot hang the process
    /// forever.
    const OVERALL_DEADLINE: Duration = Duration::from_secs(60);
    const PAGE_LOAD_DEADLINE: Duration = Duration::from_secs(10);
    const REPORT_DEADLINE: Duration = Duration::from_secs(8);
    const SETTLE_DEADLINE: Duration = Duration::from_secs(3);

    #[derive(Default)]
    struct FixtureLog {
        requests: Vec<String>,
        report: Option<String>,
        download_filename: String,
    }

    type SharedLog = Arc<Mutex<FixtureLog>>;

    fn record(log: &SharedLog, entry: &str) {
        log.lock().unwrap().requests.push(entry.to_string());
    }

    fn request_count(log: &SharedLog, needle: &str) -> usize {
        log.lock()
            .unwrap()
            .requests
            .iter()
            .filter(|entry| entry.contains(needle))
            .count()
    }

    fn take_report(log: &SharedLog) -> Option<String> {
        log.lock().unwrap().report.take()
    }

    /// Epoch milliseconds, the same clock JS's `Date.now()` uses, so a
    /// Rust-side event and a JS-reported timestamp can be merged into one
    /// ordered timeline without a separate correlation step.
    fn now_ms() -> u128 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before epoch")
            .as_millis()
    }

    /// Independent event timeline for `on_page_load` callbacks, kept
    /// separate from the HTTP fixture's own request log so a page-load
    /// assertion and a fixture-request assertion can never be confused for
    /// each other even though both are implemented as tagged string logs.
    /// Each entry carries the `now_ms()` timestamp it was recorded at, for
    /// diagnosing the order/timing of native decisions vs. what the JS side
    /// observed, without changing what any existing check matches on.
    #[derive(Default)]
    struct EventLog {
        entries: Vec<(u128, String)>,
    }

    type SharedEventLog = Arc<Mutex<EventLog>>;

    fn record_event(log: &SharedEventLog, entry: &str) {
        log.lock()
            .unwrap()
            .entries
            .push((now_ms(), entry.to_string()));
    }

    fn event_count(log: &SharedEventLog, needle: &str) -> usize {
        log.lock()
            .unwrap()
            .entries
            .iter()
            .filter(|(_, entry)| entry.contains(needle))
            .count()
    }

    async fn handle_home(State(log): State<SharedLog>) -> Response {
        record(&log, "GET /home");
        Html("<html><head><title>home</title></head><body>home</body></html>").into_response()
    }

    /// One page fixture per named route (rather than a `Path<String>`
    /// extractor on routes with no capturing segment — an earlier revision
    /// of this file did that, which axum rejects at request time since a
    /// literal route declares no path parameter to extract). Each page
    /// embeds a script that stamps a fresh `window.__pageInstanceId` only
    /// when the script actually executes: a full reload runs the script
    /// again and produces a new id, while a WebKit back/forward cache
    /// restore resumes the frozen JS realm without rerunning it, so the id
    /// observed after `go_back`/`go_forward` proves cache restoration rather
    /// than a fresh navigation, independent of whatever the fixture server
    /// did or didn't see.
    fn page_html(name: &str) -> Html<String> {
        Html(format!(
            r#"<html><head><title>{name}</title></head><body>{name}
<script>window.__pageInstanceId = "{name}-" + Math.random().toString(36).slice(2);</script>
</body></html>"#
        ))
    }

    async fn handle_page_a(State(log): State<SharedLog>) -> Response {
        record(&log, "GET /page-a");
        page_html("page-a").into_response()
    }

    async fn handle_page_b(State(log): State<SharedLog>) -> Response {
        record(&log, "GET /page-b");
        page_html("page-b").into_response()
    }

    async fn handle_isolation(State(log): State<SharedLog>) -> Response {
        record(&log, "GET /isolation");
        // Reports every isolation property this design claims: no Tauri
        // internals, no `ipc.localhost`/`buzz-media://` route, no
        // `window.open`, no camera/microphone grant (checked separately, no
        // real device opened because the adapter denies before any device
        // access begins), and no in-page `tauri://` navigation.
        Html(
            r#"<html><head><title>isolation</title></head><body>
<script>
(async () => {
  const checkpoint = (stage) => fetch(`/checkpoint/${stage}`).catch(() => {});
  const result = { internals: typeof window.__TAURI_INTERNALS__ };
  checkpoint("internals-read");
  try { await fetch("http://ipc.localhost/main"); result.ipcFetch = "ok"; }
  catch (e) { result.ipcFetch = "failed"; }
  checkpoint("ipc-fetch-done");
  try { await fetch("buzz-media://x"); result.mediaFetch = "ok"; }
  catch (e) { result.mediaFetch = "failed"; }
  checkpoint("media-fetch-done");
  result.windowOpen = window.open("https://example.com", "_blank") === null ? "blocked" : "opened";
  checkpoint("window-open-done");
  // Diagnostic for a "timeout" getUserMedia outcome: WebKit can defer or
  // silently ignore a capture request while the window/document isn't the
  // active foreground surface, independent of whatever
  // `with_permission_handler` decided. Recorded once up front, right
  // before the probes fire, alongside the real callback-decision observer
  // on the Rust side.
  result.visibilityState = document.visibilityState;
  result.hasFocus = document.hasFocus();
  // Date.now() is epoch ms, the same clock the Rust side's `now_ms()`
  // uses for page_events timestamps, so these can be merged into one
  // ordered timeline without a separate correlation step — this is what
  // shows whether a native permission decision arrives before or after
  // this probe's own internal 2s timeout, or not at all.
  result.scriptStartAt = Date.now();
  const probeGetUserMedia = (constraints) => Promise.race([
    navigator.mediaDevices.getUserMedia(constraints).then(
      (stream) => { stream.getTracks().forEach((track) => track.stop()); return "granted"; },
      () => "denied",
    ),
    new Promise((resolve) => setTimeout(() => resolve("timeout"), 2000)),
  ]);
  result.audioRequestAt = Date.now();
  result.getUserMediaAudio = await probeGetUserMedia({ audio: true });
  result.audioOutcomeAt = Date.now();
  checkpoint("getusermedia-audio-done");
  result.videoRequestAt = Date.now();
  result.getUserMediaVideo = await probeGetUserMedia({ video: true });
  result.videoOutcomeAt = Date.now();
  checkpoint("getusermedia-video-done");
  // Whether the navigation attempt lands is decided on the Rust side by
  // watching the real admission-decision log for this exact candidate
  // (see the wrapped `admit` closure), not by guessing how long WebKit
  // takes to process it and then diffing `location.href`.
  try { location.href = "tauri://localhost"; } catch (e) {}
  checkpoint("tauri-nav-done");
  await fetch("/report", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(result) });
  checkpoint("report-sent");
})();
</script>
</body></html>"#,
        )
        .into_response()
    }

    async fn handle_checkpoint(
        State(log): State<SharedLog>,
        axum::extract::Path(stage): axum::extract::Path<String>,
    ) -> Response {
        record(&log, &format!("CHECKPOINT {stage}"));
        axum::http::StatusCode::OK.into_response()
    }

    async fn handle_download(State(log): State<SharedLog>) -> Response {
        record(&log, "GET /download");
        let filename = log.lock().unwrap().download_filename.clone();
        let mut response = "download-payload".into_response();
        // `text/plain` is a MIME type WKWebView can render itself, so
        // WebKit never classifies the response as a download at all
        // (wry's wkwebview navigation policy only takes the download
        // branch when `!response.canShowMIMEType()`) — the fixture
        // rendered this inline as an ordinary page regardless of
        // `with_download_started_handler`'s decision, so the "no file
        // saved" check below exercised no real denial. `octet-stream`
        // forces WebKit into the actual download path so the production
        // callback is the thing being tested.
        response.headers_mut().insert(
            header::CONTENT_TYPE,
            "application/octet-stream".parse().unwrap(),
        );
        response.headers_mut().insert(
            header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"{filename}\"")
                .parse()
                .unwrap(),
        );
        response
    }

    async fn handle_report(State(log): State<SharedLog>, body: String) -> Response {
        log.lock().unwrap().report = Some(body);
        axum::http::StatusCode::OK.into_response()
    }

    /// Writes a fixture plugin package whose one MCP method, `browser.resolve`,
    /// answers deterministically from the requested address: the fixture
    /// server's own fixed routes for each marker, and a deliberately
    /// disallowed `file:///etc/passwd` for `denied` — the value this test
    /// uses to prove the host refuses it before the fixture ever sees a
    /// request. Mirrors the shell-script fixture style the host's own
    /// `mcp_stdio_tests.rs` uses.
    fn write_fixture_package(dir: &Path, base_url: &str) -> PathBuf {
        std::fs::create_dir_all(dir).expect("create fixture package dir");
        let script_path = dir.join("plugin.sh");
        let script = format!(
            r#"#!/usr/bin/env bash
set -u
BASE="{base_url}"
while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9]*\).*/\1/p')
  method=$(printf '%s' "$line" | sed -n 's/.*"method":"\([^"]*\)".*/\1/p')
  case "$method" in
    server/discover)
      printf '{{"jsonrpc":"2.0","id":%s,"result":{{"resultType":"complete","supportedVersions":["2026-07-28"],"capabilities":{{"tools":{{"listChanged":false}}}},"cacheScope":"private","ttlMs":0,"_meta":{{"io.modelcontextprotocol/serverInfo":{{"name":"dev.example.smoketest","version":"0.1.0"}}}}}}}}\n' "$id"
      ;;
    tools/list)
      printf '{{"jsonrpc":"2.0","id":%s,"result":{{"resultType":"complete","cacheScope":"private","ttlMs":0,"tools":[{{"name":"browser.resolve","title":"Resolve","description":"Resolve an address.","inputSchema":{{"type":"object"}},"outputSchema":{{"type":"object"}}}}]}}}}\n' "$id"
      ;;
    tools/call)
      kind=$(printf '%s' "$line" | sed -n 's/.*"kind":"\([^"]*\)".*/\1/p')
      input=$(printf '%s' "$line" | sed -n 's/.*"input":"\([^"]*\)".*/\1/p')
      if [ "$kind" = "home" ]; then
        url="$BASE/home"
      elif [ "$input" = "{DENIED_MARKER}" ]; then
        url="file:///etc/passwd"
      elif [ "$input" = "{PAGE_A_MARKER}" ]; then
        url="$BASE/page-a"
      elif [ "$input" = "{PAGE_B_MARKER}" ]; then
        url="$BASE/page-b"
      elif [ "$input" = "{ISOLATION_MARKER}" ]; then
        url="$BASE/isolation"
      elif [ "$input" = "{DOWNLOAD_MARKER}" ]; then
        url="$BASE/download"
      else
        url="$BASE/home"
      fi
      printf '{{"jsonrpc":"2.0","id":%s,"result":{{"resultType":"complete","content":[{{"type":"text","text":"%s"}}],"structuredContent":{{"outcome":"resolved","url":"%s"}}}}}}\n' "$id" "$url" "$url"
      ;;
  esac
done
"#
        );
        std::fs::write(&script_path, script).expect("write fixture script");
        let mut perms = std::fs::metadata(&script_path).unwrap().permissions();
        std::os::unix::fs::PermissionsExt::set_mode(&mut perms, 0o755);
        std::fs::set_permissions(&script_path, perms).unwrap();

        let bytes = std::fs::read(&script_path).unwrap();
        let sha256 = {
            use sha2::{Digest, Sha256};
            let mut hasher = Sha256::new();
            hasher.update(&bytes);
            hex::encode(hasher.finalize())
        };
        let triple = env!("BUZZ_PLUGIN_TARGET_TRIPLE");
        let manifest = serde_json::json!({
            "packageFormatVersion": "0.1.0-alpha",
            "contractVersion": "0.1.0-alpha",
            "id": PLUGIN_ID,
            "name": "Smoke Test Plugin",
            "version": "0.1.0",
            "publisher": "Buzz",
            "license": "MIT",
            "runtime": {
                "type": "stdio",
                "targets": {
                    triple: { "path": "plugin.sh", "sha256": sha256, "bytes": bytes.len() }
                }
            },
            "grants": ["browser.browse"],
            "contributions": [
                { "kind": "browser", "id": CONTRIBUTION_ID, "title": "Web" }
            ]
        });
        std::fs::write(
            dir.join("manifest.json"),
            serde_json::to_vec_pretty(&manifest).unwrap(),
        )
        .expect("write fixture manifest");
        dir.to_path_buf()
    }

    /// Runs one slice of the real native event loop and returns control here
    /// (never blocks on `sleep`): `run_return` processes whatever WebKit/AppKit
    /// events are currently pending — including `didCommitNavigation`/
    /// `didFinishNavigation` delegate callbacks, which is what actually
    /// advances a load — then exits as soon as at least one loop iteration
    /// completed, so the caller can re-check its condition against fresh
    /// state before pumping again.
    fn pump_once(event_loop: &mut EventLoop<()>) {
        let mut ticked = false;
        event_loop.run_return(|_event, _, control_flow| {
            if ticked {
                *control_flow = ControlFlow::Exit;
            } else {
                ticked = true;
                *control_flow = ControlFlow::Poll;
            }
        });
    }

    /// Pumps the real event loop until `condition` observes the awaited
    /// signal or `deadline` elapses. This is the only form of waiting used
    /// below: every call site names the actual signal it is waiting for
    /// (a page-load event, a fixture request count, a report, an evaluated
    /// page-state value), never a fixed sleep chosen to make the assertion
    /// pass.
    fn wait_for(
        event_loop: &mut EventLoop<()>,
        deadline: Duration,
        mut condition: impl FnMut() -> bool,
    ) -> bool {
        let start = Instant::now();
        loop {
            if condition() {
                return true;
            }
            if start.elapsed() >= deadline {
                return false;
            }
            pump_once(event_loop);
        }
    }

    /// Evaluates `js` in the live page and pumps the event loop until the
    /// WKWebView completion handler delivers a result or `deadline` elapses.
    /// Page-state assertions only; never called from production code, which
    /// this file's `browser_webview_tests.rs` sibling separately enforces by
    /// banning `evaluate_script` from `browser_webview.rs` itself.
    fn eval(
        event_loop: &mut EventLoop<()>,
        view: &wry::WebView,
        js: &str,
        deadline: Duration,
    ) -> Option<String> {
        let result: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        let result_for_callback = Arc::clone(&result);
        if view
            .evaluate_script_with_callback(js, move |value| {
                *result_for_callback.lock().unwrap() = Some(value);
            })
            .is_err()
        {
            return None;
        }
        if !wait_for(event_loop, deadline, || result.lock().unwrap().is_some()) {
            return None;
        }
        let value = result.lock().unwrap().take();
        value
    }

    /// Unwraps the JSON string `evaluate_script_with_callback` delivers
    /// (a quoted JS string value) into its plain contents.
    fn unquote(raw: &str) -> String {
        serde_json::from_str::<String>(raw).unwrap_or_else(|_| raw.to_string())
    }

    /// Re-evaluates `js` until it equals `expected` or `deadline` elapses.
    /// A back/forward-cache page swap commits the new URL before it finishes
    /// replacing the visible DOM, so a single `eval` right after the URL
    /// changes can observe the outgoing page's state for a moment; polling
    /// the real DOM instead of trusting one read makes the wait bounded and
    /// event-driven rather than timing-sensitive.
    fn eval_until(
        event_loop: &mut EventLoop<()>,
        view: &wry::WebView,
        js: &str,
        expected: &str,
        deadline: Duration,
    ) -> Option<String> {
        let start = Instant::now();
        loop {
            let value =
                eval(event_loop, view, js, Duration::from_millis(200)).map(|raw| unquote(&raw));
            if value.as_deref() == Some(expected) || start.elapsed() >= deadline {
                return value;
            }
        }
    }

    pub fn run() {
        let overall_start = Instant::now();
        let mut failures: Vec<String> = Vec::new();
        let mut check = |label: &str, ok: bool| {
            if !ok {
                failures.push(label.to_string());
                eprintln!("FAIL: {label}");
            } else {
                eprintln!("ok: {label}");
            }
        };

        let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");
        let download_filename = format!("buzz-smoke-{}.txt", std::process::id());
        let log: SharedLog = Arc::new(Mutex::new(FixtureLog {
            download_filename: download_filename.clone(),
            ..FixtureLog::default()
        }));

        let listener = runtime.block_on(async {
            tokio::net::TcpListener::bind("127.0.0.1:0")
                .await
                .expect("bind fixture listener")
        });
        let addr = listener.local_addr().expect("fixture addr");
        let base_url = format!("http://{addr}");

        {
            let app = Router::new()
                .route("/home", get(handle_home))
                .route("/isolation", get(handle_isolation))
                .route("/checkpoint/{stage}", get(handle_checkpoint))
                .route("/download", get(handle_download))
                .route("/page-a", get(handle_page_a))
                .route("/page-b", get(handle_page_b))
                .route("/report", post(handle_report))
                .with_state(Arc::clone(&log));
            runtime.spawn(async move {
                axum::serve(listener, app).await.ok();
            });
        }

        let temp = tempfile::tempdir().expect("temp dir");
        let fixture_dir = write_fixture_package(&temp.path().join("pkg"), &base_url);
        let host = PluginHost::new(PluginHostConfig {
            root: temp.path().join("host-root"),
            resolve_deadline: Duration::from_secs(2),
            discover_deadline: Duration::from_secs(5),
        });

        // Evidence gathered before any conditional early return, so a
        // failure partway through still prints every check already run.
        let mut event_loop_holder: Option<EventLoop<()>> = None;
        let mut view_holder: Option<wry::WebView> = None;

        'stages: {
            let installed = host.install(&fixture_dir);
            if let Err(error) = &installed {
                eprintln!("install error: {error:?}");
            }
            check("install succeeds", installed.is_ok());
            let Ok(installed) = installed else {
                break 'stages;
            };
            check(
                "installed id matches manifest",
                installed.plugin_id == PLUGIN_ID,
            );

            // 1. Real install and real plugin process: `start_session` spawns
            //    `plugin.sh` and speaks real MCP `server/discover`,
            //    `tools/list`, then `tools/call` over its stdio.
            let session = runtime.block_on(host.start_session(PLUGIN_ID, CONTRIBUTION_ID));
            check("start_session succeeds", session.is_ok());
            let Ok(session) = session else { break 'stages };

            let home_url = format!("{base_url}/home");
            check(
                "home resolve (via start_session) returns fixture /home",
                session.home_url == home_url,
            );

            let mut event_loop: EventLoop<()> = EventLoop::new();
            let window = WindowBuilder::new()
                .with_title("buzz browser_native_smoke")
                .with_visible(true)
                .build(&event_loop)
                .expect("create smoke window");
            // `with_visible(true)` only maps the window; it does not make
            // this process or window the foreground/active surface. If
            // WebKit defers or drops a capture request while inactive,
            // that alone (not `with_permission_handler`'s decision, not
            // Info.plist) could explain a "timeout" getUserMedia outcome,
            // so explicitly focus the synthetic window before anything
            // that requests media runs.
            window.set_focus();

            let page_events: SharedEventLog = Arc::new(Mutex::new(EventLog::default()));
            let page_events_for_admit = Arc::clone(&page_events);
            // Wraps the real admission decision (not a test double) so the
            // isolation page's denied in-page `tauri://` navigation can be
            // observed as a real event on the production decision path,
            // instead of guessing when WebKit has finished processing the
            // navigation via a fixed sleep.
            let admit: Arc<dyn Fn(&str) -> bool + Send + Sync> =
                Arc::new(move |candidate: &str| {
                    let allowed = is_admitted(candidate);
                    record_event(
                        &page_events_for_admit,
                        &format!("admit {candidate} -> {allowed}"),
                    );
                    allowed
                });
            let page_events_for_callback = Arc::clone(&page_events);
            let on_page_load: Arc<dyn Fn(PageLoad, String) + Send + Sync> =
                Arc::new(move |kind, url| {
                    let tag = match kind {
                        PageLoad::Started => "started",
                        PageLoad::Finished => "finished",
                    };
                    record_event(&page_events_for_callback, &format!("{tag} {url}"));
                });

            let page_events_for_permission = Arc::clone(&page_events);
            // Observer only: invoked with the decision the production
            // `with_permission_handler` closure already made, after it is
            // fixed. Proves the adapter itself decided `Deny`, independent
            // of whether the page's JS promise ever observes that decision
            // (the two have been seen to disagree; the cause is unproven).
            let on_permission_decision: Arc<
                dyn Fn(wry::PermissionKind, wry::PermissionResponse) + Send + Sync,
            > = Arc::new(move |kind, response| {
                record_event(
                    &page_events_for_permission,
                    &format!("permission {kind:?} -> {response:?}"),
                );
            });

            let page_events_for_download = Arc::clone(&page_events);
            // Observer only: invoked with the decision the production
            // `with_download_started_handler` closure already made, after
            // it is fixed. Proves the callback itself fired and returned
            // `false`, independent of whether the response ever committed
            // as a navigation or whether a file happened to appear.
            let on_download_decision: Arc<dyn Fn(String, bool) + Send + Sync> =
                Arc::new(move |url, allowed| {
                    record_event(
                        &page_events_for_download,
                        &format!("download {url} -> allowed={allowed}"),
                    );
                });

            let view = browser_webview::build_child(
                &window,
                &session.home_url,
                Bounds {
                    x: 0.0,
                    y: 0.0,
                    width: 800.0,
                    height: 600.0,
                },
                Arc::clone(&admit),
                Arc::clone(&on_page_load),
                Arc::clone(&on_permission_decision),
                Arc::clone(&on_download_decision),
                Arc::new(AtomicBool::new(true)),
            );
            check("build_child succeeds", view.is_ok());
            let Ok(view) = view else {
                event_loop_holder = Some(event_loop);
                break 'stages;
            };

            // 2. Local home fetch and `Finished`: wait for the real initial
            //    navigation the builder started (`with_url`) to actually
            //    commit and finish, driven only by pumping the real run loop.
            let home_loaded = wait_for(&mut event_loop, PAGE_LOAD_DEADLINE, || {
                request_count(&log, "GET /home") >= 1
                    && event_count(&page_events, &format!("finished {home_url}")) >= 1
            });
            check("home page loaded (real GET + real Finished)", home_loaded);
            check(
                "committed URL is the fixture home page",
                view.url().ok().as_deref() == Some(home_url.as_str()),
            );

            // 3. Deliberately malicious plugin answer: the host refuses it
            //    before the webview is ever told to navigate, so the fixture
            //    sees no request and the committed URL is unchanged.
            let denied = runtime.block_on(host.resolve(
                session.id.clone(),
                ResolveKind::Address,
                Some(DENIED_MARKER),
            ));
            check(
                "denied address rejected as NavigationDenied",
                matches!(denied, Err(PluginError::NavigationDenied { .. })),
            );
            check(
                "no request reached the fixture for the denied address",
                request_count(&log, "passwd") == 0,
            );
            check(
                "committed URL unchanged after host-level refusal",
                view.url().ok().as_deref() == Some(home_url.as_str()),
            );

            // 4. Isolation page: resolved through the real plugin process,
            //    loaded through the real adapter, reported back by real
            //    in-page script execution over the fixture's `/report`.
            let isolation_url = format!("{base_url}/isolation");
            let isolation_admitted = runtime.block_on(host.resolve(
                session.id.clone(),
                ResolveKind::Address,
                Some(ISOLATION_MARKER),
            ));
            check("isolation address admitted", isolation_admitted.is_ok());
            if let Ok(admitted) = &isolation_admitted {
                // Re-assert focus immediately before the page that calls
                // getUserMedia loads: something between window creation
                // and here (fixture startup, a permission prompt from an
                // earlier run) could have stolen it.
                window.set_focus();
                let _ = view.load_url(&admitted.url);
            }
            let isolation_loaded = wait_for(&mut event_loop, PAGE_LOAD_DEADLINE, || {
                request_count(&log, "GET /isolation") >= 1
                    && event_count(&page_events, &format!("finished {isolation_url}")) >= 1
            });
            check("isolation page loaded", isolation_loaded);

            let report_seen = wait_for(&mut event_loop, REPORT_DEADLINE, || {
                log.lock().unwrap().report.is_some()
            });
            if !report_seen {
                eprintln!(
                    "isolation report not observed; fixture log so far: {:?}",
                    log.lock().unwrap().requests
                );
            }
            check("isolation page reported back over /report", report_seen);
            // Date.now() epoch-ms timestamps from the JS side, captured
            // here (label, key-in-report) so they can be merged below with
            // the Rust-side page_events timestamps (same clock, `now_ms()`)
            // into one ordered timeline — without this, "the callback
            // decided Deny" and "the JS probe timed out" are two separate
            // facts with no way to tell which happened first.
            let mut js_timeline: Vec<(i64, String)> = Vec::new();
            if let Some(report) = take_report(&log) {
                let parsed: serde_json::Value = serde_json::from_str(&report).unwrap_or_default();
                for (label, key) in [
                    ("JS script-start", "scriptStartAt"),
                    ("JS audio-request", "audioRequestAt"),
                    ("JS audio-outcome", "audioOutcomeAt"),
                    ("JS video-request", "videoRequestAt"),
                    ("JS video-outcome", "videoOutcomeAt"),
                ] {
                    if let Some(ms) = parsed[key].as_i64() {
                        js_timeline.push((ms, label.to_string()));
                    }
                }
                check("no Tauri internals", parsed["internals"] == "undefined");
                check("ipc.localhost fetch failed", parsed["ipcFetch"] == "failed");
                check("buzz-media fetch failed", parsed["mediaFetch"] == "failed");
                check("window.open blocked", parsed["windowOpen"] == "blocked");
                eprintln!(
                    "info: document.visibilityState={}, document.hasFocus()={} at getUserMedia probe time",
                    parsed["visibilityState"], parsed["hasFocus"],
                );
                // The page-side probe races the real call against a 2s
                // timeout so one hung permission callback can never hang
                // this test. A timeout is not evidence of denial — it only
                // means the promise never resolved either way in 2s — so
                // only "denied" satisfies the property; a "timeout" outcome
                // fails this check rather than passing on absent evidence.
                check(
                    "getUserMedia(audio) denied, no device accessed",
                    parsed["getUserMediaAudio"] == "denied",
                );
                eprintln!(
                    "info: getUserMedia(audio) outcome: {}",
                    parsed["getUserMediaAudio"]
                );
                check(
                    "getUserMedia(video) denied, no device accessed",
                    parsed["getUserMediaVideo"] == "denied",
                );
                eprintln!(
                    "info: getUserMedia(video) outcome: {}",
                    parsed["getUserMediaVideo"]
                );
            }
            // Independent of whatever the JS promise observed above: the
            // `on_permission_decision` observer fires with the production
            // `with_permission_handler` closure's own decision, decoupling
            // "did the adapter decide Deny" from "did the JS promise ever
            // observe that decision" — the two have been seen to disagree
            // (a JS-side "timeout" outcome alongside a native Deny), and
            // the cause of that disagreement is unproven. The native
            // decision may still be in flight when a JS-side "timeout"
            // already caused `/report` to fire, so this waits (bounded)
            // rather than checking the log immediately.
            let microphone_denied = wait_for(&mut event_loop, SETTLE_DEADLINE, || {
                event_count(&page_events, "permission Microphone -> Deny") >= 1
            });
            check(
                "permission callback decided Deny for microphone",
                microphone_denied,
            );
            let camera_denied = wait_for(&mut event_loop, SETTLE_DEADLINE, || {
                event_count(&page_events, "permission Camera -> Deny") >= 1
            });
            check("permission callback decided Deny for camera", camera_denied);
            // Merged, time-ordered view of the JS-reported timestamps
            // above and every native page_events entry — shows whether
            // e.g. the video permission callback's Deny decision landed
            // before or after the JS probe's own internal 2s timeout, or
            // not at all in this window. Diagnostic only; changes no
            // check's pass/fail behavior.
            let mut timeline = js_timeline;
            for (ts, entry) in page_events.lock().unwrap().entries.iter() {
                timeline.push((*ts as i64, format!("RUST {entry}")));
            }
            timeline.sort_by_key(|(ts, _)| *ts);
            eprintln!("info: merged timing diagnostic (epoch ms, JS Date.now() == Rust now_ms()):");
            for (ts, entry) in &timeline {
                eprintln!("  {ts} {entry}");
            }
            // Real signal from the production admission decision itself,
            // not a fixed sleep followed by a `location.href` diff: the
            // wrapped `admit` closure records every candidate it decides on,
            // so wait for the exact denial entry the isolation page's
            // in-page `tauri://` navigation attempt must have produced.
            let tauri_nav_denied = wait_for(&mut event_loop, SETTLE_DEADLINE, || {
                page_events
                    .lock()
                    .unwrap()
                    .entries
                    .iter()
                    .any(|(_, entry)| {
                        entry.starts_with("admit tauri://") && entry.ends_with("-> false")
                    })
            });
            check("in-page tauri:// navigation refused", tauri_nav_denied);
            check(
                "committed URL is still the isolation page after its own refused navigations",
                view.url().ok().as_deref() == Some(isolation_url.as_str()),
            );

            // 5. Download denial through the production callback: the
            //    fixture sees the request (WebKit must fetch headers to
            //    decide this is a download), but `with_download_started_handler`
            //    always denies, so no navigation happens and no file lands
            //    at the destination WebKit would otherwise have used.
            let download_dir_before = std::env::var_os("HOME")
                .map(PathBuf::from)
                .and_then(|home| dirs_download_dir_under(&home));
            let download_target = download_dir_before
                .as_ref()
                .map(|dir| dir.join(&download_filename));
            let download_url = format!("{base_url}/download");
            let download_admitted = runtime.block_on(host.resolve(
                session.id.clone(),
                ResolveKind::Address,
                Some(DOWNLOAD_MARKER),
            ));
            check("download address admitted", download_admitted.is_ok());
            if let Ok(admitted) = &download_admitted {
                let _ = view.load_url(&admitted.url);
            }
            let download_requested = wait_for(&mut event_loop, SETTLE_DEADLINE, || {
                request_count(&log, "GET /download") >= 1
            });
            check("download requested from the fixture", download_requested);
            // `application/octet-stream` (the fixture no longer serves
            // `text/plain`) forces WebKit's real download path — wry only
            // takes it when `!response.canShowMIMEType()` — so
            // `with_download_started_handler` is the callback actually
            // being exercised here, not skipped as it was when the
            // response rendered inline. The `on_download_decision` observer
            // fires with that closure's own return value after it is
            // fixed, so wait for the real denial entry rather than
            // inferring the decision from its side effects.
            let download_denied = wait_for(&mut event_loop, SETTLE_DEADLINE, || {
                event_count(
                    &page_events,
                    &format!("download {download_url} -> allowed=false"),
                ) >= 1
            });
            check("download callback decided false (denied)", download_denied);
            // A denied download never commits a navigation, so no
            // `Finished` event fires for `download_url`; the file and URL
            // checks below corroborate the callback's decision with its
            // real-world effects rather than asserting on an event this
            // path cannot produce.
            if let Some(target) = &download_target {
                check(
                    "no file was saved at the destination a download would have used",
                    !target.exists(),
                );
            }
            check(
                "committed URL unchanged by the denied download",
                view.url().ok().as_deref() == Some(isolation_url.as_str()),
            );

            // 6. Native back/forward: two real navigations, then traversal.
            //    Each page stamps `window.__pageInstanceId` only when its
            //    script actually executes, so comparing the id (and the
            //    fixture's own GET count) before and after traversal shows
            //    whether WebKit served the restore from its back/forward
            //    cache or replayed a real navigation — both are valid
            //    browser behavior, so this only asserts the committed URL
            //    and page state are correct, and records which path
            //    actually happened rather than requiring one of them.
            let page_a_url = format!("{base_url}/page-a");
            let page_b_url = format!("{base_url}/page-b");

            let page_a_admitted = runtime.block_on(host.resolve(
                session.id.clone(),
                ResolveKind::Address,
                Some(PAGE_A_MARKER),
            ));
            check("page-a address admitted", page_a_admitted.is_ok());
            if let Ok(admitted) = &page_a_admitted {
                let _ = view.load_url(&admitted.url);
            }
            let page_a_loaded = wait_for(&mut event_loop, PAGE_LOAD_DEADLINE, || {
                event_count(&page_events, &format!("finished {page_a_url}")) >= 1
            });
            check("page-a loaded", page_a_loaded);
            let page_a_gets_after_first_load = request_count(&log, "GET /page-a");
            let page_a_instance = eval(
                &mut event_loop,
                &view,
                "window.__pageInstanceId",
                PAGE_LOAD_DEADLINE,
            )
            .map(|raw| unquote(&raw));
            check("page-a instance id captured", page_a_instance.is_some());

            let page_b_admitted = runtime.block_on(host.resolve(
                session.id.clone(),
                ResolveKind::Address,
                Some(PAGE_B_MARKER),
            ));
            check("page-b address admitted", page_b_admitted.is_ok());
            if let Ok(admitted) = &page_b_admitted {
                let _ = view.load_url(&admitted.url);
            }
            let page_b_loaded = wait_for(&mut event_loop, PAGE_LOAD_DEADLINE, || {
                event_count(&page_events, &format!("finished {page_b_url}")) >= 1
            });
            check("page-b loaded", page_b_loaded);
            let page_b_gets_after_first_load = request_count(&log, "GET /page-b");
            let page_b_instance = eval(
                &mut event_loop,
                &view,
                "window.__pageInstanceId",
                PAGE_LOAD_DEADLINE,
            )
            .map(|raw| unquote(&raw));
            check("page-b instance id captured", page_b_instance.is_some());

            check(
                "back is available before going back",
                view.can_go_back().unwrap_or(false),
            );
            let _ = view.go_back();
            let back_restored = wait_for(&mut event_loop, PAGE_LOAD_DEADLINE, || {
                view.url().ok().as_deref() == Some(page_a_url.as_str())
            });
            check("back restores page-a's committed URL", back_restored);
            let back_title = eval_until(
                &mut event_loop,
                &view,
                "document.title",
                "page-a",
                PAGE_LOAD_DEADLINE,
            );
            check(
                "back restored page-a's DOM (title)",
                back_title.as_deref() == Some("page-a"),
            );
            let back_instance = eval(
                &mut event_loop,
                &view,
                "window.__pageInstanceId",
                PAGE_LOAD_DEADLINE,
            )
            .map(|raw| unquote(&raw));
            check("back restored page-a's page state", back_instance.is_some());
            eprintln!(
                "info: back to page-a was a {} (GET count {} -> {}, instance id {})",
                if back_instance.is_some() && back_instance == page_a_instance {
                    "cache restore"
                } else {
                    "fresh reload"
                },
                page_a_gets_after_first_load,
                request_count(&log, "GET /page-a"),
                if back_instance == page_a_instance {
                    "unchanged"
                } else {
                    "changed"
                },
            );

            let _ = view.go_forward();
            let forward_restored = wait_for(&mut event_loop, PAGE_LOAD_DEADLINE, || {
                view.url().ok().as_deref() == Some(page_b_url.as_str())
            });
            check("forward restores page-b's committed URL", forward_restored);
            let forward_title = eval_until(
                &mut event_loop,
                &view,
                "document.title",
                "page-b",
                PAGE_LOAD_DEADLINE,
            );
            check(
                "forward restored page-b's DOM (title)",
                forward_title.as_deref() == Some("page-b"),
            );
            let forward_instance = eval(
                &mut event_loop,
                &view,
                "window.__pageInstanceId",
                PAGE_LOAD_DEADLINE,
            )
            .map(|raw| unquote(&raw));
            check(
                "forward restored page-b's page state",
                forward_instance.is_some(),
            );
            eprintln!(
                "info: forward to page-b was a {} (GET count {} -> {}, instance id {})",
                if forward_instance.is_some() && forward_instance == page_b_instance {
                    "cache restore"
                } else {
                    "fresh reload"
                },
                page_b_gets_after_first_load,
                request_count(&log, "GET /page-b"),
                if forward_instance == page_b_instance {
                    "unchanged"
                } else {
                    "changed"
                },
            );

            // 7. Disable keeps the package and grant but invalidates the
            //    session; uninstall removes the package and registry record.
            let disable = host.set_enabled(PLUGIN_ID, false);
            check("disable succeeds", disable.is_ok());
            check(
                "disable reports this session as terminated",
                disable
                    .as_ref()
                    .map(|outcome| outcome.terminated_sessions.contains(&session.id))
                    .unwrap_or(false),
            );
            let after_disable =
                runtime.block_on(host.resolve(session.id.clone(), ResolveKind::Home, None));
            check(
                "resolve on the disabled session's process no longer succeeds",
                after_disable.is_err(),
            );
            let still_listed = host.list().map(|plugins| {
                plugins
                    .iter()
                    .any(|plugin| plugin.plugin_id == PLUGIN_ID && !plugin.enabled)
            });
            check(
                "package and grant retained, marked disabled (not removed)",
                still_listed == Ok(true),
            );

            let uninstall = host.uninstall(PLUGIN_ID);
            check("uninstall succeeds", uninstall.is_ok());
            check(
                "uninstall removed the package directory",
                uninstall
                    .as_ref()
                    .map(|outcome| outcome.package_removal.is_ok())
                    .unwrap_or(false),
            );
            let removed = host
                .list()
                .map(|plugins| plugins.iter().any(|plugin| plugin.plugin_id == PLUGIN_ID));
            check(
                "package no longer listed after uninstall",
                removed == Ok(false),
            );
            let after_uninstall =
                runtime.block_on(host.resolve(session.id.clone(), ResolveKind::Home, None));
            check(
                "resolve after uninstall is NoSession",
                matches!(after_uninstall, Err(PluginError::NoSession)),
            );

            view_holder = Some(view);
            event_loop_holder = Some(event_loop);
        }

        // Cleanup runs on every path, including a failure partway through:
        // drop the native view first (closes the WKWebView), terminate any
        // remaining plugin process groups, then let the fixture server and
        // its runtime tear down as this function returns.
        drop(view_holder.take());
        if let Some(mut event_loop) = event_loop_holder.take() {
            pump_once(&mut event_loop);
        }
        if let Err(error) = host.shutdown() {
            eprintln!("browser_native_smoke: host shutdown reported {error:?}");
        }

        check(
            "completed within the overall deadline",
            overall_start.elapsed() < OVERALL_DEADLINE,
        );

        print_summary(&failures);
    }

    /// Best-effort real download-directory lookup without adding a new
    /// dependency: mirrors `dirs::download_dir()`'s macOS behavior
    /// (`~/Downloads`) so the download-denial check can look for evidence
    /// of a saved file without ever writing one itself.
    fn dirs_download_dir_under(home: &Path) -> Option<PathBuf> {
        let candidate = home.join("Downloads");
        candidate.is_dir().then_some(candidate)
    }

    fn print_summary(failures: &[String]) {
        use std::io::Write;
        let _ = std::io::stdout().flush();
        if failures.is_empty() {
            println!("browser_native_smoke: all checks passed");
        } else {
            eprintln!("browser_native_smoke: {} check(s) failed:", failures.len());
            for failure in failures {
                eprintln!("  - {failure}");
            }
            std::process::exit(1);
        }
    }
}
