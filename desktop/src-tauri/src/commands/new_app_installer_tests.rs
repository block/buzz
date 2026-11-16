use std::io;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::get;
use axum::Router;
use bytes::Bytes;
use futures_util::stream::{self, StreamExt};
use tokio_util::sync::CancellationToken;
use url::Url;

use super::*;

const RELEASE: &str = "https://github.com/block/buzz-app/releases/download/v1.0.0";
const FILE_NAME: &str = "Buzz_1.0.0_aarch64.dmg";
const BODY: &[u8] = b"not really a disk image, but enough bytes to count";

fn test_limits() -> TransferLimits {
    TransferLimits {
        max_bytes: 1024,
        response_timeout: Duration::from_secs(5),
        stall_timeout: Duration::from_millis(200),
        total_timeout: Duration::from_secs(5),
    }
}

/// A body that sends `first` and then never sends anything else.
fn stalled_body(first: &'static [u8]) -> Body {
    Body::from_stream(
        stream::once(async move { Ok::<_, io::Error>(Bytes::from_static(first)) })
            .chain(stream::pending()),
    )
}

/// A body with no announced length, sent as `chunks` chunks of 100 bytes.
fn unsized_body(chunks: usize) -> Body {
    Body::from_stream(stream::iter(
        (0..chunks).map(|_| Ok::<_, io::Error>(Bytes::from(vec![7u8; 100]))),
    ))
}

async fn serve() -> SocketAddr {
    let app = Router::new()
        .route("/ok", get(|| async { BODY }))
        .route("/missing", get(|| async { StatusCode::NOT_FOUND }))
        .route("/empty", get(|| async { "" }))
        .route("/announced-too-large", get(|| async { vec![0u8; 2048] }))
        .route("/unsized-ok", get(|| async { unsized_body(3) }))
        .route("/unsized-too-large", get(|| async { unsized_body(20) }))
        .route("/stalls", get(|| async { stalled_body(b"first") }))
        .route(
            "/cut-short",
            get(|| async {
                Response::builder()
                    .header(header::CONTENT_LENGTH, 1000)
                    .body(Body::from_stream(stream::iter([
                        Ok(Bytes::from_static(b"only this much")),
                        Err(io::Error::other("connection dropped")),
                    ])))
                    .map_err(|error| error.to_string())
                    .into_response()
            }),
        )
        .route("/redirects", get(|| async { Redirect::temporary("/ok") }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test server");
    let addr = listener.local_addr().expect("test server address");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve test server");
    });
    addr
}

fn source_at(addr: SocketAddr, path: &str) -> InstallerSource {
    InstallerSource {
        url: Url::parse(&format!("http://{addr}{path}")).expect("test url"),
        file_name: FILE_NAME.to_string(),
        extension: ".dmg",
    }
}

fn file_names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .expect("read download dir")
        .map(|entry| {
            entry
                .expect("dir entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    names.sort();
    names
}

/// Set in the copy of a test that [`rerun_with_open_umask`] starts.
#[cfg(unix)]
const OPEN_UMASK_ENV: &str = "BUZZ_TEST_INSTALLER_OPEN_UMASK";

/// Runs `test` of this module again in its own process under a umask of 000,
/// and fails if that run fails. A umask belongs to the whole process, so
/// changing it here would loosen the files other tests create meanwhile.
#[cfg(unix)]
fn rerun_with_open_umask(test: &str) {
    // libtest names a test by its module path without the crate name.
    let module = module_path!().split_once("::").map_or("", |(_, rest)| rest);
    let output = std::process::Command::new("sh")
        .args(["-c", r#"umask 000 && exec "$0" "$@""#])
        .arg(std::env::current_exe().expect("test binary"))
        .args(["--exact", &format!("{module}::{test}"), "--test-threads=1"])
        .env(OPEN_UMASK_ENV, "1")
        .output()
        .expect("rerun the test under an open umask");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        // "1 passed" proves the name matched a test, not nothing.
        output.status.success() && stdout.contains("1 passed"),
        "{test} under umask 000:\n{stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

struct Run {
    result: Result<PathBuf, TransferError>,
    events: Vec<InstallerDownloadEvent>,
    marked: Vec<PathBuf>,
}

/// Runs one download with a plain client and a mark that records its target.
async fn run(addr: SocketAddr, path: &str, dir: &Path, cancel: &CancellationToken) -> Run {
    let events = Mutex::new(Vec::new());
    let marked = Mutex::new(Vec::new());
    let result = download_installer(
        &reqwest::Client::new(),
        &source_at(addr, path),
        dir,
        test_limits(),
        cancel,
        |_| {},
        |partial, _| {
            marked.lock().expect("marked").push(partial.to_path_buf());
            Ok(())
        },
        |event| events.lock().expect("events").push(event),
    )
    .await;
    Run {
        result,
        events: events.into_inner().expect("events"),
        marked: marked.into_inner().expect("marked"),
    }
}

#[test]
fn accepts_the_installer_for_each_shipped_target() {
    let cases = [
        ("macos-arm64", "Buzz_1.0.0_aarch64.dmg", ".dmg"),
        ("macos-x64", "Buzz_1.0.0_x64.dmg", ".dmg"),
        ("windows-x64", "Buzz_1.0.0_x64_unsigned.exe", ".exe"),
        ("linux-appimage", "Buzz_1.0.0_x86_64.AppImage", ".AppImage"),
        ("linux-deb", "Buzz_1.0.0_amd64.deb", ".deb"),
    ];
    for (target, file_name, extension) in cases {
        let url = format!("{RELEASE}/{file_name}");
        let source = validate_installer_url(&url, target).expect(target);
        assert_eq!(source.url.as_str(), url, "{target}");
        assert_eq!(source.file_name, file_name, "{target}");
        assert_eq!(source.extension, extension, "{target}");
    }
    // A tag may itself hold slashes; the file name is still the last segment.
    let nested = validate_installer_url(
        "https://github.com/block/buzz-app/releases/download/desktop/v1.0.0/Buzz.dmg",
        "macos-arm64",
    )
    .expect("nested tag");
    assert_eq!(nested.file_name, "Buzz.dmg");
    // Every character a tag may hold.
    assert!(validate_installer_url(
        "https://github.com/block/buzz-app/releases/download/desktop/v1.0.0-rc.1+build_2/Buzz.dmg",
        "macos-arm64",
    )
    .is_some());
}

#[test]
fn failure_reasons_reach_the_webview_under_their_kebab_names() {
    let cases = [
        (InstallerFailure::Busy, "busy"),
        (InstallerFailure::Rejected, "rejected"),
        (InstallerFailure::Network, "network"),
        (InstallerFailure::RedirectBlocked, "redirect-blocked"),
        (InstallerFailure::TooLarge, "too-large"),
        (InstallerFailure::Storage, "storage"),
        (InstallerFailure::PermissionDenied, "permission-denied"),
        (InstallerFailure::Protection, "protection"),
    ];
    for (reason, name) in cases {
        assert_eq!(
            serde_json::to_value(reason).expect("serialize"),
            serde_json::Value::from(name)
        );
    }
}

#[test]
fn rejects_every_url_that_is_not_this_machines_installer() {
    let long_name = format!("{}.dmg", "a".repeat(MAX_FILE_NAME_LEN));
    let cases: Vec<(&str, String, &str)> = vec![
        ("not a url", "Buzz.dmg".into(), "macos-arm64"),
        (
            "plain http",
            format!("{RELEASE}/Buzz.dmg").replace("https:", "http:"),
            "macos-arm64",
        ),
        (
            "another host",
            "https://example.org/block/buzz-app/releases/download/v1/Buzz.dmg".into(),
            "macos-arm64",
        ),
        (
            "host hidden behind userinfo",
            "https://github.com@example.org/block/buzz-app/releases/download/v1/Buzz.dmg".into(),
            "macos-arm64",
        ),
        (
            "credentials",
            "https://user:pass@github.com/block/buzz-app/releases/download/v1/Buzz.dmg".into(),
            "macos-arm64",
        ),
        (
            "non-default port",
            "https://github.com:8443/block/buzz-app/releases/download/v1/Buzz.dmg".into(),
            "macos-arm64",
        ),
        (
            "another repository",
            "https://github.com/example-org/buzz-app/releases/download/v1/Buzz.dmg".into(),
            "macos-arm64",
        ),
        (
            "dot segments that leave the repository",
            format!("{RELEASE}/../../../../../example-org/other/releases/download/v1/Buzz.dmg"),
            "macos-arm64",
        ),
        (
            "encoded dot segments that leave the repository",
            "https://github.com/block/buzz-app/releases/download/%2e%2e/%2e%2e/%2e%2e/%2e%2e/example-org/other/Buzz.dmg".into(),
            "macos-arm64",
        ),
        ("no tag segment", "https://github.com/block/buzz-app/releases/download/Buzz.dmg".into(), "macos-arm64"),
        ("encoded separator in the tag", format!("{RELEASE}%2F..%2F..%2Fother/Buzz.dmg"), "macos-arm64"),
        ("encoded backslash in the tag", format!("{RELEASE}%5Cother/Buzz.dmg"), "macos-arm64"),
        ("encoded NUL in the tag", format!("{RELEASE}%00/Buzz.dmg"), "macos-arm64"),
        ("semicolon in the tag", format!("{RELEASE};other/Buzz.dmg"), "macos-arm64"),
        ("space in the tag", format!("{RELEASE}%20beta/Buzz.dmg"), "macos-arm64"),
        ("at sign in the tag", format!("{RELEASE}@other/Buzz.dmg"), "macos-arm64"),
        (
            "wrong file type for the target",
            format!("{RELEASE}/Buzz_1.0.0_x64_unsigned.exe"),
            "macos-arm64",
        ),
        (
            "extension only in the query",
            format!("{RELEASE}/Buzz.exe?name=Buzz.dmg"),
            "macos-arm64",
        ),
        ("query", format!("{RELEASE}/Buzz.dmg?download=1"), "macos-arm64"),
        ("fragment", format!("{RELEASE}/Buzz.dmg#top"), "macos-arm64"),
        ("extension with nothing before it", format!("{RELEASE}/.dmg"), "macos-arm64"),
        ("hidden file", format!("{RELEASE}/.Buzz.dmg"), "macos-arm64"),
        ("encoded separator in the name", format!("{RELEASE}/..%2FBuzz.dmg"), "macos-arm64"),
        ("encoded backslash in the name", format!("{RELEASE}/..%5CBuzz.dmg"), "macos-arm64"),
        ("space in the name", format!("{RELEASE}/Buzz%20App.dmg"), "macos-arm64"),
        ("name too long", format!("{RELEASE}/{long_name}"), "macos-arm64"),
        ("windows device name", format!("{RELEASE}/CON.exe"), "windows-x64"),
        ("windows device name, any case", format!("{RELEASE}/com1.setup.exe"), "windows-x64"),
        ("target with no installer", format!("{RELEASE}/Buzz.dmg"), "linux-arm64"),
    ];
    for (name, url, target) in cases {
        assert_eq!(validate_installer_url(&url, target), None, "{name}");
    }
    // Near misses of the device-name rule are ordinary names.
    for name in ["COM0.exe", "CONSOLE.exe", "LPT10.exe"] {
        assert!(
            validate_installer_url(&format!("{RELEASE}/{name}"), "windows-x64").is_some(),
            "{name}"
        );
    }
}

#[test]
fn redirects_stay_on_https_release_asset_hosts() {
    let cases = [
        ("https://github.com/block/buzz-app/releases/x", true),
        ("https://objects.githubusercontent.com/asset", true),
        (
            "https://release-assets.githubusercontent.com/asset?sig=1",
            true,
        ),
        ("http://release-assets.githubusercontent.com/asset", false),
        (
            "https://release-assets.githubusercontent.com:8443/asset",
            false,
        ),
        (
            "https://user@release-assets.githubusercontent.com/asset",
            false,
        ),
        ("https://evil.githubusercontent.com/asset", false),
        (
            "https://release-assets.githubusercontent.com.example.org/asset",
            false,
        ),
        ("https://example.org/asset", false),
        ("https://127.0.0.1/asset", false),
    ];
    for (url, expected) in cases {
        let parsed = Url::parse(url).expect(url);
        assert_eq!(redirect_allowed(&parsed), expected, "{url}");
    }
}

#[tokio::test]
async fn installer_client_refuses_other_hosts_and_plain_http() {
    let addr = serve().await;

    // The plain client follows this redirect, so the server side works.
    let followed = reqwest::Client::new()
        .get(format!("http://{addr}/redirects"))
        .send()
        .await
        .expect("plain client follows the redirect");
    assert_eq!(followed.status(), StatusCode::OK);

    // The installer client's policy stops it: 127.0.0.1 is not an asset host.
    let policy_only = installer_client_builder().build().expect("client");
    let error = policy_only
        .get(format!("http://{addr}/redirects"))
        .send()
        .await
        .expect_err("redirect to a host outside the allowlist");
    assert!(error.is_redirect(), "{error:?}");

    // The production client does not speak plain http at all.
    let production = build_installer_client().expect("production client");
    let error = production
        .get(format!("http://{addr}/ok"))
        .send()
        .await
        .expect_err("plain http");
    assert!(error.is_builder(), "{error:?}");
}

#[tokio::test]
async fn saves_the_whole_body_under_the_installers_name() {
    let addr = serve().await;
    let dir = tempfile::tempdir().expect("download dir");
    let partial_seen = Mutex::new(None);
    let events = Mutex::new(Vec::new());

    let saved = download_installer(
        &reqwest::Client::new(),
        &source_at(addr, "/ok"),
        dir.path(),
        test_limits(),
        &CancellationToken::new(),
        |_| {},
        |partial, origin| {
            // The mark is applied to the complete partial file, before any
            // file exists under the installer's name.
            assert_eq!(std::fs::read(partial).expect("partial"), BODY);
            assert_eq!(origin.path(), "/ok");
            assert!(!dir.path().join(FILE_NAME).exists());
            *partial_seen.lock().expect("partial") = Some(partial.to_path_buf());
            Ok(())
        },
        |event| events.lock().expect("events").push(event),
    )
    .await
    .expect("download");

    assert_eq!(saved, dir.path().join(FILE_NAME));
    assert_eq!(std::fs::read(&saved).expect("saved"), BODY);
    assert_eq!(file_names(dir.path()), [FILE_NAME]);

    let partial = partial_seen.into_inner().expect("partial").expect("marked");
    let partial_name = partial
        .file_name()
        .expect("partial name")
        .to_string_lossy()
        .into_owned();
    assert!(
        partial_name.starts_with("Buzz_1.0.0_aarch64.dmg.") && partial_name.ends_with(".part"),
        "{partial_name}"
    );

    let total = BODY.len() as u64;
    let events = events.into_inner().expect("events");
    assert_eq!(
        events.first(),
        Some(&InstallerDownloadEvent::Progress {
            received_bytes: 0,
            total_bytes: Some(total),
        })
    );
    assert_eq!(
        events.last(),
        Some(&InstallerDownloadEvent::Progress {
            received_bytes: total,
            total_bytes: Some(total),
        })
    );

    // The saved file is requested as 0644 and the umask applies on top. It is
    // not the private mode of a temporary file. Whatever the umask, nobody but
    // the owner can write it and nobody can execute it.
    #[cfg(unix)]
    {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        let mode_of = |path: &Path| {
            std::fs::metadata(path)
                .expect("metadata")
                .permissions()
                .mode()
                & 0o777
        };
        let ordinary = dir.path().join("ordinary-new-file");
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o644)
            .open(&ordinary)
            .expect("ordinary file");
        let mode = mode_of(&saved);
        // A stricter umask is honoured: the same bits come off as for any file.
        assert_eq!(mode, mode_of(&ordinary), "{mode:o}");
        assert_eq!(mode & 0o133, 0, "{mode:o}");
        if std::env::var_os(OPEN_UMASK_ENV).is_some() {
            // No umask at all: the mode is exactly the one requested.
            assert_eq!(mode, 0o644, "{mode:o}");
        } else {
            // The default umask hides a looser request, so check without one.
            rerun_with_open_umask("saves_the_whole_body_under_the_installers_name");
        }
    }
}

#[tokio::test]
async fn a_redirect_the_policy_refuses_is_its_own_failure() {
    let addr = serve().await;
    let dir = tempfile::tempdir().expect("download dir");

    // 127.0.0.1 is not a release asset host, so the policy stops the redirect.
    let result = download_installer(
        &installer_client_builder().build().expect("client"),
        &source_at(addr, "/redirects"),
        dir.path(),
        test_limits(),
        &CancellationToken::new(),
        |_| {},
        |_, _| Ok(()),
        |_| {},
    )
    .await;

    assert_eq!(
        result,
        Err(TransferError::Failed(InstallerFailure::RedirectBlocked))
    );
    assert_eq!(file_names(dir.path()), Vec::<String>::new());
}

#[tokio::test]
async fn a_folder_that_cannot_be_written_names_its_cause() {
    let addr = serve().await;
    let dir = tempfile::tempdir().expect("parent dir");

    // Something that is not a folder sits where the folder should be.
    let not_a_folder = dir.path().join("Downloads");
    std::fs::write(&not_a_folder, b"a file").expect("file");
    let run_in = run(addr, "/ok", &not_a_folder, &CancellationToken::new()).await;
    assert_eq!(
        run_in.result,
        Err(TransferError::Failed(InstallerFailure::Storage))
    );

    // The system refuses the write: the folder is read-only for this user.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let locked = dir.path().join("locked");
        std::fs::create_dir(&locked).expect("locked dir");
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o555))
            .expect("lock dir");
        // A superuser ignores the mode, and then there is nothing to observe.
        if std::fs::write(locked.join("probe"), b"").is_err() {
            let run_in = run(addr, "/ok", &locked, &CancellationToken::new()).await;
            assert_eq!(
                run_in.result,
                Err(TransferError::Failed(InstallerFailure::PermissionDenied))
            );
            assert_eq!(file_names(&locked), Vec::<String>::new());
        }
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755))
            .expect("unlock dir");
    }
}

#[test]
fn the_download_folder_falls_back_to_home_downloads_on_linux_only() {
    let configured = Some(PathBuf::from("/home/sam/Transfers"));
    let home = Some(PathBuf::from("/home/sam"));
    // (configured, home, is_linux, expected)
    let cases = [
        (configured.clone(), home.clone(), true, configured.clone()),
        (configured.clone(), None, false, configured.clone()),
        (
            None,
            home.clone(),
            true,
            Some(PathBuf::from("/home/sam/Downloads")),
        ),
        (None, home.clone(), false, None),
        (None, None, true, None),
    ];
    for (configured, home, is_linux, expected) in cases {
        assert_eq!(
            resolve_download_dir(configured.clone(), home.clone(), is_linux),
            expected,
            "{configured:?} {home:?} {is_linux}"
        );
    }
}

#[tokio::test]
async fn never_overwrites_an_existing_file() {
    let addr = serve().await;
    let dir = tempfile::tempdir().expect("download dir");
    std::fs::write(dir.path().join(FILE_NAME), b"mine").expect("existing");
    std::fs::write(dir.path().join("Buzz_1.0.0_aarch64 (1).dmg"), b"also mine").expect("existing");

    let run = run(addr, "/ok", dir.path(), &CancellationToken::new()).await;

    let saved = run.result.expect("download");
    assert_eq!(saved, dir.path().join("Buzz_1.0.0_aarch64 (2).dmg"));
    assert_eq!(std::fs::read(&saved).expect("saved"), BODY);
    assert_eq!(
        std::fs::read(dir.path().join(FILE_NAME)).expect("first"),
        b"mine"
    );
    assert_eq!(
        std::fs::read(dir.path().join("Buzz_1.0.0_aarch64 (1).dmg")).expect("second"),
        b"also mine"
    );
    assert_eq!(file_names(dir.path()).len(), 3);
}

#[test]
fn candidate_names_count_up_before_the_extension() {
    assert_eq!(candidate_file_name("Buzz.dmg", ".dmg", 0), "Buzz.dmg");
    assert_eq!(candidate_file_name("Buzz.dmg", ".dmg", 1), "Buzz (1).dmg");
    assert_eq!(
        candidate_file_name("Buzz_1.0.0_x86_64.AppImage", ".AppImage", 12),
        "Buzz_1.0.0_x86_64 (12).AppImage"
    );
}

#[tokio::test]
async fn failed_transfers_leave_nothing_in_the_folder() {
    let addr = serve().await;
    let cases = [
        ("/missing", InstallerFailure::Network),
        ("/empty", InstallerFailure::Network),
        ("/cut-short", InstallerFailure::Network),
        ("/stalls", InstallerFailure::Network),
        ("/announced-too-large", InstallerFailure::TooLarge),
        ("/unsized-too-large", InstallerFailure::TooLarge),
    ];
    for (path, reason) in cases {
        let dir = tempfile::tempdir().expect("download dir");
        let run = run(addr, path, dir.path(), &CancellationToken::new()).await;
        assert_eq!(run.result, Err(TransferError::Failed(reason)), "{path}");
        assert_eq!(file_names(dir.path()), Vec::<String>::new(), "{path}");
        assert_eq!(run.marked, Vec::<PathBuf>::new(), "{path}");
    }
}

#[tokio::test]
async fn a_body_without_a_length_is_saved_with_no_total() {
    let addr = serve().await;
    let dir = tempfile::tempdir().expect("download dir");

    let run = run(addr, "/unsized-ok", dir.path(), &CancellationToken::new()).await;

    let saved = run.result.expect("download");
    assert_eq!(std::fs::read(saved).expect("saved").len(), 300);
    assert_eq!(
        run.events.first(),
        Some(&InstallerDownloadEvent::Progress {
            received_bytes: 0,
            total_bytes: None,
        })
    );
}

#[tokio::test]
async fn a_file_that_cannot_be_marked_is_not_kept() {
    let addr = serve().await;
    let dir = tempfile::tempdir().expect("download dir");

    let result = download_installer(
        &reqwest::Client::new(),
        &source_at(addr, "/ok"),
        dir.path(),
        test_limits(),
        &CancellationToken::new(),
        |_| {},
        |_, _| Err(io::Error::other("volume has no extended attributes")),
        |_| {},
    )
    .await;

    assert_eq!(
        result,
        Err(TransferError::Failed(InstallerFailure::Protection))
    );
    assert_eq!(file_names(dir.path()), Vec::<String>::new());
}

#[tokio::test]
async fn cancelling_mid_transfer_removes_the_partial_file() {
    let addr = serve().await;
    let dir = tempfile::tempdir().expect("download dir");
    let cancel = CancellationToken::new();
    let partial_seen = Mutex::new(None);
    let limits = TransferLimits {
        stall_timeout: Duration::from_secs(30),
        total_timeout: Duration::from_secs(30),
        ..test_limits()
    };
    let source = source_at(addr, "/stalls");
    let client = reqwest::Client::new();

    let download = download_installer(
        &client,
        &source,
        dir.path(),
        limits,
        &cancel,
        |partial| *partial_seen.lock().expect("partial") = Some(partial.to_path_buf()),
        |_, _| Ok(()),
        |_| {},
    );
    // Cancel once the first body bytes are on disk under the partial name.
    let cancel_mid_transfer = async {
        loop {
            let partial = partial_seen.lock().expect("partial").clone();
            let written = partial
                .and_then(|partial| std::fs::metadata(partial).ok())
                .map_or(0, |metadata| metadata.len());
            if written > 0 {
                assert_eq!(file_names(dir.path()).len(), 1, "only the partial file");
                cancel.cancel();
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    };
    let (result, ()) = tokio::join!(download, cancel_mid_transfer);

    assert_eq!(result, Err(TransferError::Cancelled));
    assert_eq!(file_names(dir.path()), Vec::<String>::new());
}

#[tokio::test]
async fn a_cancel_that_lands_before_the_move_leaves_no_installer() {
    let addr = serve().await;
    let dir = tempfile::tempdir().expect("download dir");
    let cancel = CancellationToken::new();

    let result = download_installer(
        &reqwest::Client::new(),
        &source_at(addr, "/ok"),
        dir.path(),
        test_limits(),
        &cancel,
        |_| {},
        |_, _| {
            cancel.cancel();
            Ok(())
        },
        |_| {},
    )
    .await;

    assert_eq!(result, Err(TransferError::Cancelled));
    assert_eq!(file_names(dir.path()), Vec::<String>::new());
}

#[test]
fn only_one_download_holds_the_slot() {
    let downloads = InstallerDownloads::default();
    let first = downloads.begin().expect("first download");
    assert!(
        downloads.begin().is_none(),
        "second download while one runs"
    );

    // Cancel reaches the running download's token.
    assert!(!first.cancel.is_cancelled());
    downloads.cancel();
    assert!(first.cancel.is_cancelled());

    drop(first);
    let next = downloads.begin().expect("slot is free again");
    assert!(
        !next.cancel.is_cancelled(),
        "a new download starts uncancelled"
    );
}

#[test]
fn shutdown_cancels_and_removes_the_partial_file() {
    let dir = tempfile::tempdir().expect("download dir");
    let partial = dir.path().join("Buzz.dmg.abc123.part");
    std::fs::write(&partial, b"half").expect("partial");
    let downloads = InstallerDownloads::default();

    // Nothing running: nothing to do.
    downloads.abort_for_shutdown();
    assert!(partial.exists());

    let record = dir.path().join("app-data").join("partial-record.txt");
    downloads.recover(record.clone(), Some(dir.path()));
    let active = downloads.begin().expect("download");
    downloads.set_partial(&partial);
    assert!(record.exists());
    downloads.abort_for_shutdown();
    assert!(active.cancel.is_cancelled());
    assert!(!partial.exists());
    // Nothing is left for the next launch to remove.
    assert!(!record.exists());
    // The download task cleaning up afterwards finds nothing, and that is fine.
    downloads.abort_for_shutdown();
}

#[test]
fn the_next_launch_removes_exactly_the_partial_a_dead_run_recorded() {
    let dir = tempfile::tempdir().expect("download dir");
    let record = dir.path().join("app-data").join("partial-record.txt");
    let downloads_dir = dir.path().join("Downloads");
    std::fs::create_dir(&downloads_dir).expect("download dir");
    let partial_name = "Buzz_1.0.0_aarch64.dmg.aB3xYz.part";
    let partial = downloads_dir.join(partial_name);
    // Files the cleanup has no business touching, however similar they look.
    let bystanders = [
        "Buzz_1.0.0_aarch64.dmg",
        "Buzz_1.0.0_aarch64.dmg.zzzzzz.part",
        "film.mkv.part",
    ];
    std::fs::write(&partial, b"half").expect("partial");
    for name in bystanders {
        std::fs::write(downloads_dir.join(name), b"mine").expect(name);
    }

    // A download that ends normally leaves no record behind. While it runs
    // the record holds the partial file's name and no folder.
    let downloads = InstallerDownloads::default();
    downloads.recover(record.clone(), Some(&downloads_dir));
    let active = downloads.begin().expect("download");
    downloads.set_partial(&partial);
    assert_eq!(
        std::fs::read_to_string(&record).expect("record"),
        partial_name
    );
    std::fs::remove_file(&partial).expect("the download removes its partial");
    drop(active);
    assert!(!record.exists());
    std::fs::write(&partial, b"half").expect("partial");

    // A run that dies mid-download does: none of its cleanup runs.
    let active = downloads.begin().expect("download");
    downloads.set_partial(&partial);
    std::mem::forget(active);
    assert!(record.exists());

    // Without a Downloads folder to look in, the record waits.
    remove_recorded_partial(&record, None);
    assert!(record.exists());
    assert!(partial.exists());

    // The next launch removes that file, forgets it, and touches nothing else.
    InstallerDownloads::default().recover(record.clone(), Some(&downloads_dir));
    assert!(!record.exists());
    assert_eq!(file_names(&downloads_dir), bystanders);

    // A partial file of exactly this module's shape, in a folder that is not
    // the Downloads folder. No record can reach it.
    let elsewhere = dir.path().join("elsewhere");
    std::fs::create_dir(&elsewhere).expect("elsewhere");
    let outside = elsewhere.join("backup.deb.old1.part");
    std::fs::write(&outside, b"not ours").expect("outside");
    #[cfg(unix)]
    std::os::unix::fs::symlink(
        downloads_dir.join("Buzz_1.0.0_aarch64.dmg"),
        downloads_dir.join("Buzz_1.0.0_aarch64.dmg.linked.part"),
    )
    .expect("symlink");
    std::fs::create_dir(downloads_dir.join("Buzz_1.0.0_aarch64.dmg.folder.part")).expect("folder");
    // A record that names anything but a partial file of this module's shape
    // directly inside the Downloads folder is dropped, and nothing is removed.
    let not_ours = [
        // An edited record that holds a path instead of a name.
        outside.to_str().expect("utf-8 path").to_string(),
        "../elsewhere/backup.deb.old1.part".to_string(),
        partial.to_str().expect("utf-8 path").to_string(),
        // The right shape, but only the other folder has such a file.
        "backup.deb.old1.part".to_string(),
        // Names in the Downloads folder that are not partial files.
        "Buzz_1.0.0_aarch64.dmg".to_string(),
        "film.mkv.part".to_string(),
        // The right shape, but a link and a folder, not a regular file.
        "Buzz_1.0.0_aarch64.dmg.linked.part".to_string(),
        "Buzz_1.0.0_aarch64.dmg.folder.part".to_string(),
        String::new(),
    ];
    std::fs::write(&partial, b"half").expect("partial");
    let before = (file_names(&downloads_dir), file_names(&elsewhere));
    for recorded in not_ours {
        std::fs::write(&record, &recorded).expect("record");
        remove_recorded_partial(&record, Some(&downloads_dir));
        assert!(!record.exists(), "{recorded}");
        assert_eq!(
            (file_names(&downloads_dir), file_names(&elsewhere)),
            before,
            "{recorded}"
        );
    }
    // Neither is a record that is not text at all.
    std::fs::write(&record, [0xff, 0xfe, 0x00]).expect("record");
    remove_recorded_partial(&record, Some(&downloads_dir));
    assert!(!record.exists());
    assert_eq!(file_names(&downloads_dir), before.0);
}

#[test]
fn a_partial_that_cannot_be_removed_stays_recorded() {
    let dir = tempfile::tempdir().expect("download dir");
    let record = dir.path().join("app-data").join("partial-record.txt");
    let downloads_dir = dir.path().join("Downloads");
    std::fs::create_dir(&downloads_dir).expect("download dir");
    let partial_name = "Buzz_1.0.0_aarch64.dmg.aB3xYz.part";
    let partial = downloads_dir.join(partial_name);
    let recorded = || std::fs::read_to_string(&record).ok();

    // A folder stands in for a file the system refuses to remove, such as one
    // a scanner holds open on Windows: removing it as a file fails everywhere.
    std::fs::create_dir(&partial).expect("stand-in");
    let downloads = InstallerDownloads::default();
    downloads.recover(record.clone(), Some(&downloads_dir));

    // A download ends and its own removal of the partial file fails, which
    // nothing reports. Freeing the slot must not forget the file.
    let active = downloads.begin().expect("download");
    downloads.set_partial(&partial);
    drop(active);
    assert_eq!(recorded().as_deref(), Some(partial_name));

    // The same at exit: the removal fails there, and again when the download
    // task frees its slot afterwards.
    let active = downloads.begin().expect("download");
    downloads.set_partial(&partial);
    downloads.abort_for_shutdown();
    assert!(active.cancel.is_cancelled());
    assert_eq!(recorded().as_deref(), Some(partial_name));
    drop(active);
    assert_eq!(recorded().as_deref(), Some(partial_name));

    // The next launch finds the file by that record and removes it.
    std::fs::remove_dir(&partial).expect("stand-in");
    std::fs::write(&partial, b"half").expect("partial");
    InstallerDownloads::default().recover(record.clone(), Some(&downloads_dir));
    assert!(!partial.exists());
    assert_eq!(recorded(), None);
}

#[test]
fn progress_is_reported_at_most_once_per_interval() {
    let start = Instant::now();
    let mut throttle = ProgressThrottle::default();
    let total = Some(1000);

    assert!(throttle.admit(0, total, start), "first report");
    assert!(!throttle.admit(10, total, start + Duration::from_millis(10)));
    assert!(!throttle.admit(20, total, start + Duration::from_millis(99)));
    assert!(throttle.admit(30, total, start + Duration::from_millis(100)));
    assert!(!throttle.admit(40, total, start + Duration::from_millis(150)));
    // The report that completes the total always goes through.
    assert!(throttle.admit(1000, total, start + Duration::from_millis(151)));
    // Without a total there is no such exception.
    assert!(!throttle.admit(1000, None, start + Duration::from_millis(152)));
}

#[test]
fn quarantine_value_matches_a_browser_download() {
    assert_eq!(quarantine_value(0x6916_0a2b), "0081;69160a2b;Buzz;");
    assert_eq!(quarantine_value(1), "0081;00000001;Buzz;");
}

#[test]
fn mark_of_the_web_names_the_internet_zone_and_the_origin() {
    let origin = Url::parse(&format!("{RELEASE}/Buzz_1.0.0_x64_unsigned.exe")).expect("origin");
    assert_eq!(
        zone_identifier_contents(&origin),
        format!("[ZoneTransfer]\r\nZoneId=3\r\nHostUrl={RELEASE}/Buzz_1.0.0_x64_unsigned.exe\r\n")
    );
    assert_eq!(
        zone_identifier_path(Path::new("Downloads/Buzz.exe.abc.part")),
        Path::new("Downloads/Buzz.exe.abc.part:Zone.Identifier")
    );
}

#[test]
fn mark_of_the_web_write_is_verified_by_reading_it_back() {
    let dir = tempfile::tempdir().expect("dir");
    let origin = Url::parse(&format!("{RELEASE}/Buzz.exe")).expect("origin");

    // Off Windows the stream path is an ordinary sibling file, which is enough
    // to exercise the write and the read-back.
    let file = dir.path().join("Buzz.exe.part");
    std::fs::write(&file, b"installer").expect("file");
    write_zone_identifier(&file, &origin).expect("mark");
    assert_eq!(
        std::fs::read_to_string(zone_identifier_path(&file)).expect("stream"),
        zone_identifier_contents(&origin)
    );

    // A stream that cannot be created fails the mark.
    let missing = dir.path().join("no-such-dir").join("Buzz.exe.part");
    assert!(write_zone_identifier(&missing, &origin).is_err());
}

// Listed on every platform, so a runner that selects this file's tests by
// name finds it; it runs where the quarantine attribute exists.
#[test]
#[cfg_attr(
    not(target_os = "macos"),
    ignore = "the quarantine attribute exists only on macOS"
)]
fn macos_download_is_quarantined() {
    #[cfg(target_os = "macos")]
    macos_download_is_quarantined_body();
}

#[cfg(target_os = "macos")]
fn macos_download_is_quarantined_body() {
    let dir = tempfile::tempdir().expect("dir");
    let file = dir.path().join("Buzz.dmg.part");
    std::fs::write(&file, b"installer").expect("file");
    let origin = Url::parse(&format!("{RELEASE}/Buzz.dmg")).expect("origin");

    mark_as_downloaded(&file, &origin).expect("mark");

    let value = xattr::get(&file, QUARANTINE_XATTR)
        .expect("read quarantine")
        .expect("quarantine attribute is set");
    let value = String::from_utf8(value).expect("utf8");
    let fields: Vec<&str> = value.split(';').collect();
    assert_eq!(fields.len(), 4, "{value}");
    assert_eq!(fields[0], "0081");
    assert!(u64::from_str_radix(fields[1], 16).expect("hex timestamp") > 0x6000_0000);
    assert_eq!(fields[2], "Buzz");

    // The attribute follows the file through the move into place.
    let moved = dir.path().join("Buzz.dmg");
    std::fs::rename(&file, &moved).expect("rename");
    assert!(xattr::get(&moved, QUARANTINE_XATTR)
        .expect("read")
        .is_some());

    // A file that is gone cannot be marked, and that is an error.
    assert!(mark_as_downloaded(&file, &origin).is_err());
}
