//! What the host serves a canvas frame and what it refuses: declared-script
//! bootstrap, capability gating, path containment, the document security
//! policy, and the native navigation and update-socket seams.

use std::fs;

use tempfile::TempDir;

use super::super::{
    allow_webview_navigation, ipc, protocol,
    storage::{prepare_snapshot, ProjectBinding},
    ProjectCanvasRuntime,
};
use super::{request, source_root, write_package, OWNER};

#[test]
fn active_load_serves_its_validated_bytes_after_disk_mutation() {
    let temp = TempDir::new().unwrap();
    let binding = ProjectBinding::parse(request()).unwrap();
    let source = source_root(&temp, &binding);
    write_package(&source, "immutable");
    let snapshot = prepare_snapshot(&temp.path().join("CANVASES"), &binding, None).unwrap();
    let revision = snapshot.revision.clone();
    let runtime = ProjectCanvasRuntime::with_root(temp.path().join("CANVASES"));
    let descriptor = runtime.issue_load(binding.clone(), snapshot).unwrap();

    let disk_entry = binding
        .runtime_root_for_test(&temp.path().join("CANVASES"))
        .join("revisions")
        .join(revision)
        .join("canvas.js");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&disk_entry, fs::Permissions::from_mode(0o644)).unwrap();
    }
    #[cfg(windows)]
    {
        let mut permissions = fs::metadata(&disk_entry).unwrap().permissions();
        permissions.set_readonly(false);
        fs::set_permissions(&disk_entry, permissions).unwrap();
    }
    fs::write(&disk_entry, "globalThis.canvasMarker = 'tampered';").unwrap();

    let path = format!("/{}/package/canvas.js", descriptor.load_id);
    let (_, body) = protocol::route(&runtime, &path).unwrap();
    assert_eq!(
        String::from_utf8(body).unwrap(),
        "globalThis.canvasMarker = \"immutable\";"
    );

    runtime.release(&descriptor.load_id).unwrap();
    assert!(protocol::route(&runtime, &path).is_err());
}

#[test]
fn bootstrap_is_host_owned_and_loads_only_declared_scripts_after_connect() {
    let temp = TempDir::new().unwrap();
    let binding = ProjectBinding::parse(request()).unwrap();
    let source = source_root(&temp, &binding);
    write_package(&source, "bootstrap");
    let snapshot = prepare_snapshot(&temp.path().join("CANVASES"), &binding, None).unwrap();
    let runtime = ProjectCanvasRuntime::with_root(temp.path().join("CANVASES"));
    let descriptor = runtime.issue_load(binding, snapshot).unwrap();

    let (_, shell) = protocol::route(&runtime, &format!("/{}/", descriptor.load_id)).unwrap();
    let shell = String::from_utf8(shell).unwrap();
    assert!(shell.contains("id=\"canvas-root\""));
    assert!(!shell.contains("canvasMarker"));

    let (_, bootstrap) = protocol::route(
        &runtime,
        &format!("/{}/__buzz/bootstrap.js", descriptor.load_id),
    )
    .unwrap();
    let bootstrap = String::from_utf8(bootstrap).unwrap();
    assert!(bootstrap.contains(&descriptor.nonce));
    assert!(bootstrap.contains("message.type !== \"host.connect\""));
    assert!(bootstrap.contains("widgets/chore%2Dboard%2Ejs"));
    assert!(bootstrap.contains("canvas%2Ejs"));
    assert!(bootstrap.contains("window, \"buzzCanvas\""));
    assert!(bootstrap.contains("packageBaseUrl"));
    assert!(bootstrap.contains("new URL(\"./package/\", location.href).href"));
    assert!(bootstrap.contains("sdk: {}"));
    assert!(!protocol::DOCUMENT_CSP.contains("'unsafe-inline'"));

    // The host SDK loads before any package resource so packages can use
    // window.buzzCanvas.sdk from their first statement.
    let scripts_list = bootstrap
        .split("const scripts = [")
        .nth(1)
        .and_then(|rest| rest.split(']').next())
        .unwrap();
    assert!(scripts_list.starts_with("\"./__buzz/sdk.js\","));
    let styles_list = bootstrap
        .split("const styles = [")
        .nth(1)
        .and_then(|rest| rest.split(']').next())
        .unwrap();
    assert!(styles_list.starts_with("\"./__buzz/sdk.css\","));
}

#[test]
fn host_sdk_routes_serve_the_bundled_sources() {
    let temp = TempDir::new().unwrap();
    let binding = ProjectBinding::parse(request()).unwrap();
    let source = source_root(&temp, &binding);
    write_package(&source, "sdk");
    let snapshot = prepare_snapshot(&temp.path().join("CANVASES"), &binding, None).unwrap();
    let runtime = ProjectCanvasRuntime::with_root(temp.path().join("CANVASES"));
    let descriptor = runtime.issue_load(binding, snapshot).unwrap();

    let (content_type, sdk_js) =
        protocol::route(&runtime, &format!("/{}/__buzz/sdk.js", descriptor.load_id)).unwrap();
    assert_eq!(content_type, "text/javascript; charset=utf-8");
    let sdk_js = String::from_utf8(sdk_js).unwrap();
    assert!(sdk_js.contains("canvas.subscribe"));
    assert!(sdk_js.contains("host.subscriptionUpdate"));
    // The SDK must not start the port: the package entry owns port.start(),
    // and starting it early would drop host.init for later listeners.
    assert!(!sdk_js.contains("port.start()"));

    let (content_type, sdk_css) =
        protocol::route(&runtime, &format!("/{}/__buzz/sdk.css", descriptor.load_id)).unwrap();
    assert_eq!(content_type, "text/css; charset=utf-8");
    assert!(String::from_utf8(sdk_css)
        .unwrap()
        .contains("--buzz-background"));
}

#[test]
fn manifest_accepts_the_full_capability_set_and_rejects_unknown_ones() {
    let temp = TempDir::new().unwrap();
    let binding = ProjectBinding::parse(request()).unwrap();
    let source = source_root(&temp, &binding);
    write_package(&source, "capabilities");
    let manifest_path = source.join("manifest.json");
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
    // Must accept every capability the desktop protocol schema recognizes.
    manifest["capabilities"] = serde_json::json!([
        "project.metadata.read",
        "project.channels.read",
        "project.reviews.read",
        "project.tasks.read",
        "project.people.read",
        "project.tasks.write",
        "app.open",
        "app.dm.send"
    ]);
    fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    assert!(prepare_snapshot(&temp.path().join("CANVASES"), &binding, None).is_ok());

    manifest["capabilities"] = serde_json::json!(["network"]);
    fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    let error = prepare_snapshot(&temp.path().join("CANVASES"), &binding, None).unwrap_err();
    assert!(error.contains("unsupported project canvas capability"));
}

#[test]
fn invalid_or_undeclared_package_files_fail_closed() {
    let temp = TempDir::new().unwrap();
    let binding = ProjectBinding::parse(request()).unwrap();
    let source = source_root(&temp, &binding);
    write_package(&source, "bad");
    fs::write(source.join("index.html"), "<script>bad()</script>").unwrap();

    let error = prepare_snapshot(&temp.path().join("CANVASES"), &binding, None).unwrap_err();
    assert!(error.contains("unsupported project canvas file type"));
}

#[test]
fn finder_metadata_does_not_break_package_reload() {
    let temp = TempDir::new().unwrap();
    let binding = ProjectBinding::parse(request()).unwrap();
    let source = source_root(&temp, &binding);
    write_package(&source, "finder");
    fs::write(source.join(".DS_Store"), b"finder metadata").unwrap();

    assert!(prepare_snapshot(&temp.path().join("CANVASES"), &binding, None).is_ok());
}

#[test]
fn manifest_paths_cannot_traverse_the_package() {
    let temp = TempDir::new().unwrap();
    let binding = ProjectBinding::parse(request()).unwrap();
    let source = source_root(&temp, &binding);
    write_package(&source, "bad-path");
    let manifest = serde_json::json!({
        "format": "buzz-project-canvas",
        "protocolVersion": 1,
        "scripts": ["widgets/../escape.js", "canvas.js"],
        "styles": ["styles/canvas.css"],
        "data": "data/dashboards.json",
        "capabilities": []
    });
    fs::write(
        source.join("manifest.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();

    assert!(prepare_snapshot(&temp.path().join("CANVASES"), &binding, None).is_err());
}

#[cfg(unix)]
#[test]
fn symlinked_storage_ancestor_is_rejected() {
    use std::os::unix::fs::symlink;

    let temp = TempDir::new().unwrap();
    let root = temp.path().join("CANVASES");
    fs::create_dir(&root).unwrap();
    let root = root.canonicalize().unwrap();
    let binding = ProjectBinding::parse(request()).unwrap();
    let project = binding.project_root_for_test(&root);
    let community = root.join(
        project
            .strip_prefix(&root)
            .unwrap()
            .components()
            .next()
            .unwrap(),
    );
    let outside = temp.path().join("outside");
    fs::create_dir(&outside).unwrap();
    symlink(&outside, &community).unwrap();

    let error = prepare_snapshot(&root, &binding, None).unwrap_err();
    assert!(error.contains("not a real directory"));
}

#[cfg(unix)]
#[test]
fn package_symlinks_are_rejected() {
    use std::os::unix::fs::symlink;

    let temp = TempDir::new().unwrap();
    let binding = ProjectBinding::parse(request()).unwrap();
    let source = source_root(&temp, &binding);
    write_package(&source, "symlink");
    let outside = temp.path().join("outside.png");
    fs::write(&outside, "secret").unwrap();
    symlink(&outside, source.join("assets/leak.png")).unwrap();

    assert!(prepare_snapshot(&temp.path().join("CANVASES"), &binding, None).is_err());
}

#[cfg(unix)]
#[test]
fn package_hard_links_are_rejected() {
    let temp = TempDir::new().unwrap();
    let binding = ProjectBinding::parse(request()).unwrap();
    let source = source_root(&temp, &binding);
    write_package(&source, "hard-link");
    let outside = temp.path().join("outside.png");
    fs::write(&outside, "secret").unwrap();
    fs::hard_link(&outside, source.join("assets/leak.png")).unwrap();

    let error = prepare_snapshot(&temp.path().join("CANVASES"), &binding, None).unwrap_err();
    assert!(error.contains("hard linked"));
}

#[test]
fn project_coordinate_and_community_are_validated_before_path_derivation() {
    let mut invalid = request();
    invalid.community_id = "../other".to_string();
    // Community values are hashed, so punctuation cannot become a path.
    assert!(ProjectBinding::parse(invalid).is_ok());

    let mut invalid = request();
    invalid.project_id = "30621:not-hex:project".to_string();
    assert!(ProjectBinding::parse(invalid).is_err());

    let mut invalid = request();
    invalid.project_id = format!("30621:{OWNER}:");
    assert!(ProjectBinding::parse(invalid).is_err());
}

#[test]
fn protocol_security_policy_has_no_network_or_tauri_ipc_source() {
    assert!(protocol::DOCUMENT_CSP.contains("connect-src 'none'"));
    assert!(protocol::DOCUMENT_CSP.contains("webrtc 'block'"));
    assert!(!protocol::DOCUMENT_CSP.contains(" ipc:"));
    assert!(!protocol::PERMISSIONS_POLICY.contains("camera=(*"));
    assert!(!protocol::PERMISSIONS_POLICY.contains("microphone=(*"));
}

#[test]
fn native_navigation_policy_blocks_external_document_navigation() {
    assert!(allow_webview_navigation(
        &"buzz-canvas://localhost/load/".parse().unwrap(),
        None
    ));
    assert!(allow_webview_navigation(
        &"tauri://localhost/".parse().unwrap(),
        None
    ));
    assert!(allow_webview_navigation(
        &"about:blank".parse().unwrap(),
        None
    ));
    assert!(!allow_webview_navigation(
        &"https://example.com/leak?snapshot=secret".parse().unwrap(),
        None
    ));
    assert!(!allow_webview_navigation(
        &"file:///tmp/secret".parse().unwrap(),
        None
    ));
}

// The dev server load is the webview's *initial* navigation, so a policy that
// does not recognise the configured origin opens a blank window. Every `just`
// desktop recipe derives a per-worktree Vite port, so the origin the app is
// launched on is never the `tauri.conf.json` default.
#[test]
fn native_navigation_policy_allows_the_configured_dev_server() {
    let dev_url: tauri::Url = "http://localhost:30164".parse().unwrap();

    assert!(allow_webview_navigation(
        &"http://localhost:30164/".parse().unwrap(),
        Some(&dev_url)
    ));
    assert!(allow_webview_navigation(
        &"http://localhost:30164/index.html".parse().unwrap(),
        Some(&dev_url)
    ));
}

#[test]
fn native_navigation_policy_blocks_other_localhost_origins() {
    let dev_url: tauri::Url = "http://localhost:30164".parse().unwrap();

    // Some other server on the loopback interface is not the frontend.
    assert!(!allow_webview_navigation(
        &"http://localhost:1420/".parse().unwrap(),
        Some(&dev_url)
    ));
    assert!(!allow_webview_navigation(
        &"http://127.0.0.1:30164/".parse().unwrap(),
        Some(&dev_url)
    ));
    // Release builds have no dev server, so plain http stays blocked.
    assert!(!allow_webview_navigation(
        &"http://localhost:30164/".parse().unwrap(),
        None
    ));
}

// `start` runs on the main thread from Tauri's `setup` hook, with no Tokio
// runtime in context. Registering the socket with the reactor there aborts the
// whole app on launch, so the handoff has to survive a plain sync caller.
#[cfg(unix)]
#[test]
fn agent_update_socket_serves_when_started_outside_the_async_runtime() {
    use std::{
        os::unix::net::{UnixListener as StdUnixListener, UnixStream as StdUnixStream},
        sync::mpsc,
        time::Duration,
    };

    let temp = TempDir::new().unwrap();
    let socket_path = temp.path().join("agent-updates.sock");
    let listener = StdUnixListener::bind(&socket_path).unwrap();
    listener.set_nonblocking(true).unwrap();

    let (accepted, wait) = mpsc::channel();
    ipc::spawn_serving(listener, socket_path.clone(), move |listener| async move {
        if listener.accept().await.is_ok() {
            let _ = accepted.send(());
        }
    });

    let _client = StdUnixStream::connect(&socket_path).unwrap();
    wait.recv_timeout(Duration::from_secs(5))
        .expect("socket bound before the runtime handoff should still accept connections");
}
