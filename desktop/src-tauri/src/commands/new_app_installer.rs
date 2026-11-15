//! In-app download of the new Buzz app's installer, started from the
//! deprecation banner.
//!
//! The webview only names a URL. Everything else is decided here: whether the
//! URL is a release asset of the new app for this machine, which hosts a
//! redirect may reach, what the file is called and where it goes. The file is
//! written under a partial name in the Downloads folder, marked as downloaded
//! from the internet, and only then moved to a free final name.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::ipc::Channel;
use tauri::{AppHandle, Manager};
use tempfile::TempPath;
use tokio::io::AsyncWriteExt;
use tokio_util::sync::CancellationToken;
use url::Url;

use super::updater::new_app_installer_target;

const INSTALLER_HOST: &str = "github.com";
const INSTALLER_PATH_PREFIX: &str = "/block/buzz-app/releases/download/";
/// Hosts GitHub serves release assets from: the download link itself, and the
/// storage hosts it redirects to.
const REDIRECT_HOSTS: [&str; 3] = [
    "github.com",
    "objects.githubusercontent.com",
    "release-assets.githubusercontent.com",
];
const MAX_REDIRECTS: usize = 5;
const MAX_FILE_NAME_LEN: usize = 128;
/// `name.ext`, then `name (1).ext` up to `name (99).ext`.
const MAX_NAME_ATTEMPTS: u32 = 100;
/// Published installers are under 200 MiB.
const MAX_INSTALLER_BYTES: u64 = 512 * 1024 * 1024;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(30);
const STALL_TIMEOUT: Duration = Duration::from_secs(30);
const TOTAL_TIMEOUT: Duration = Duration::from_secs(60 * 60);
const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);
const PARTIAL_SUFFIX: &str = ".part";
/// File in the app data directory that holds the file name of the running
/// download's partial file, so the launch after a crash can remove exactly
/// that file from the Downloads folder.
const PARTIAL_RECORD_NAME: &str = "new-app-installer-partial.txt";

/// Progress sent to the webview over the command's channel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum InstallerDownloadEvent {
    /// The URL was accepted; `file_name` is the name the file is saved under
    /// unless that name is taken.
    Started { file_name: String },
    /// Bytes written so far. `total_bytes` is absent when the server sent no
    /// length.
    Progress {
        received_bytes: u64,
        total_bytes: Option<u64>,
    },
}

/// Why a download did not produce a file. The webview words each one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum InstallerFailure {
    /// Another installer download is still running.
    Busy,
    /// The URL is not an installer of the new app for this machine.
    Rejected,
    /// The request failed, stalled, or ended early.
    Network,
    /// A redirect pointed outside the hosts release assets are served from, or
    /// there were too many of them. Trying again gets the same answer.
    RedirectBlocked,
    /// The response is larger than any installer should be.
    TooLarge,
    /// The file could not be written or moved into the Downloads folder.
    Storage,
    /// The system refused access to the Downloads folder.
    PermissionDenied,
    /// The file could not be marked as downloaded from the internet.
    Protection,
}

/// How a download ended.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(
    tag = "status",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum InstallerDownloadOutcome {
    /// The installer is at `folder`/`file_name`.
    Saved { file_name: String, folder: String },
    /// The download was cancelled and its partial file removed.
    Cancelled,
    /// The download failed and its partial file was removed.
    Failed { reason: InstallerFailure },
}

fn fail(reason: InstallerFailure, detail: impl std::fmt::Display) -> InstallerFailure {
    eprintln!("buzz-desktop: new app installer download failed ({reason:?}): {detail}");
    reason
}

/// A write the system refused is told apart from every other storage error:
/// freeing space does not fix it, granting access does.
fn storage_failure(error: io::Error) -> InstallerFailure {
    let reason = if error.kind() == io::ErrorKind::PermissionDenied {
        InstallerFailure::PermissionDenied
    } else {
        InstallerFailure::Storage
    };
    fail(reason, error)
}

/// Every extension [`installer_extension`] can return.
const INSTALLER_EXTENSIONS: [&str; 4] = [".dmg", ".exe", ".AppImage", ".deb"];

/// The file extension each installer manifest key must point at.
fn installer_extension(target: &str) -> Option<&'static str> {
    match target {
        "macos-arm64" | "macos-x64" => Some(".dmg"),
        "windows-x64" => Some(".exe"),
        "linux-appimage" => Some(".AppImage"),
        "linux-deb" => Some(".deb"),
        _ => None,
    }
}

/// An installer URL that passed validation, with the file name derived from it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct InstallerSource {
    pub(super) url: Url,
    pub(super) file_name: String,
    pub(super) extension: &'static str,
}

/// Accepts `raw` only when it is an https release asset of block/buzz-app on
/// github.com whose file name is plain and ends in the extension `target`
/// installs. These are the rules the webview's `resolveInstallerUrl` applies;
/// they are repeated here because the webview is not trusted.
pub(super) fn validate_installer_url(raw: &str, target: &str) -> Option<InstallerSource> {
    let extension = installer_extension(target)?;
    // Parsing normalises `..` segments and backslashes before the checks.
    let url = Url::parse(raw).ok()?;
    if url.scheme() != "https"
        || url.host_str() != Some(INSTALLER_HOST)
        || url.port().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return None;
    }
    let (tag, file_name) = url
        .path()
        .strip_prefix(INSTALLER_PATH_PREFIX)?
        .rsplit_once('/')?;
    if !is_safe_tag(tag) || !is_safe_file_name(file_name, extension) {
        return None;
    }
    Some(InstallerSource {
        file_name: file_name.to_string(),
        extension,
        url,
    })
}

/// A release tag is sent to GitHub as written: letters, digits and `._+-/`
/// only. It holds no percent escape, so staying inside the repository's
/// releases does not depend on how GitHub decodes one.
fn is_safe_tag(tag: &str) -> bool {
    !tag.is_empty()
        && tag.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'+' | b'-' | b'/')
        })
}

/// A file name is used as is or not at all: letters, digits and `._+-` only,
/// so it holds no separator, no percent escape and no control character.
fn is_safe_file_name(name: &str, extension: &str) -> bool {
    name.len() <= MAX_FILE_NAME_LEN
        && name
            .strip_suffix(extension)
            .is_some_and(|stem| !stem.is_empty())
        && name.starts_with(|first: char| first.is_ascii_alphanumeric())
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'+' | b'-'))
        && !is_windows_device_name(name)
}

/// `CON.exe`, `nul.dmg` and friends name devices on Windows, whatever follows
/// the first dot.
fn is_windows_device_name(name: &str) -> bool {
    let base = name.split('.').next().unwrap_or(name).to_ascii_uppercase();
    match base.as_str() {
        "CON" | "PRN" | "AUX" | "NUL" => true,
        _ => {
            base.len() == 4
                && (base.starts_with("COM") || base.starts_with("LPT"))
                && base.ends_with(|digit: char| ('1'..='9').contains(&digit))
        }
    }
}

/// The name tried on the given attempt: the installer's own name first, then
/// `stem (1).ext`, `stem (2).ext` and so on.
pub(super) fn candidate_file_name(file_name: &str, extension: &str, attempt: u32) -> String {
    if attempt == 0 {
        return file_name.to_string();
    }
    let stem = file_name.strip_suffix(extension).unwrap_or(file_name);
    format!("{stem} ({attempt}){extension}")
}

/// Whether a redirect may be followed: https, default port, no credentials,
/// and one of the hosts GitHub serves release assets from.
pub(super) fn redirect_allowed(url: &Url) -> bool {
    url.scheme() == "https"
        && url.port().is_none()
        && url.username().is_empty()
        && url.password().is_none()
        && url
            .host_str()
            .is_some_and(|host| REDIRECT_HOSTS.contains(&host))
}

/// Everything about the installer client except the https-only switch, which
/// tests against a local http server have to leave off.
pub(super) fn installer_client_builder() -> reqwest::ClientBuilder {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() > MAX_REDIRECTS {
                attempt.error("too many redirects")
            } else if redirect_allowed(attempt.url()) {
                attempt.follow()
            } else {
                attempt.error("redirect left the release asset hosts")
            }
        }))
        .connect_timeout(CONNECT_TIMEOUT)
        .read_timeout(STALL_TIMEOUT)
        .user_agent(concat!("buzz-desktop/", env!("CARGO_PKG_VERSION")))
}

/// Builds the client installer downloads use. It is separate from the app-wide
/// client because that one follows redirects to any host; this one refuses
/// plain http and every redirect outside [`REDIRECT_HOSTS`].
pub(super) fn build_installer_client() -> reqwest::Result<reqwest::Client> {
    installer_client_builder().https_only(true).build()
}

/// Bounds on one transfer. Tests shrink them.
#[derive(Debug, Clone, Copy)]
pub(super) struct TransferLimits {
    pub(super) max_bytes: u64,
    pub(super) response_timeout: Duration,
    pub(super) stall_timeout: Duration,
    pub(super) total_timeout: Duration,
}

impl TransferLimits {
    const PRODUCTION: Self = Self {
        max_bytes: MAX_INSTALLER_BYTES,
        response_timeout: RESPONSE_TIMEOUT,
        stall_timeout: STALL_TIMEOUT,
        total_timeout: TOTAL_TIMEOUT,
    };
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum TransferError {
    Cancelled,
    Failed(InstallerFailure),
}

/// Lets a progress report through at most once per [`PROGRESS_INTERVAL`], plus
/// the first report and the one that completes a known total.
#[derive(Debug, Default)]
pub(super) struct ProgressThrottle {
    last: Option<Instant>,
}

impl ProgressThrottle {
    pub(super) fn admit(&mut self, received: u64, total: Option<u64>, now: Instant) -> bool {
        let due = self
            .last
            .is_none_or(|last| now.duration_since(last) >= PROGRESS_INTERVAL);
        if due || total == Some(received) {
            self.last = Some(now);
            return true;
        }
        false
    }
}

/// Streams the response body into `file`, enforcing the size cap and the
/// response and stall timeouts. Returns the number of bytes written.
async fn fetch_into(
    client: &reqwest::Client,
    url: Url,
    file: &mut tokio::fs::File,
    limits: TransferLimits,
    mut on_progress: impl FnMut(u64, Option<u64>),
) -> Result<u64, InstallerFailure> {
    let mut response = tokio::time::timeout(limits.response_timeout, client.get(url).send())
        .await
        .map_err(|_| fail(InstallerFailure::Network, "no response in time"))?
        .map_err(|error| {
            // The client's redirect policy refused the next hop.
            let reason = if error.is_redirect() {
                InstallerFailure::RedirectBlocked
            } else {
                InstallerFailure::Network
            };
            fail(reason, error)
        })?;
    if response.status() != reqwest::StatusCode::OK {
        return Err(fail(InstallerFailure::Network, response.status()));
    }
    let total = response.content_length();
    if let Some(total) = total.filter(|total| *total > limits.max_bytes) {
        return Err(fail(
            InstallerFailure::TooLarge,
            format_args!("{total} bytes announced"),
        ));
    }

    let mut received: u64 = 0;
    on_progress(received, total);
    loop {
        let chunk = tokio::time::timeout(limits.stall_timeout, response.chunk())
            .await
            .map_err(|_| fail(InstallerFailure::Network, "transfer stalled"))?
            .map_err(|error| fail(InstallerFailure::Network, error))?;
        let Some(chunk) = chunk else {
            break;
        };
        received += chunk.len() as u64;
        if received > limits.max_bytes {
            return Err(fail(
                InstallerFailure::TooLarge,
                format_args!("more than {} bytes received", limits.max_bytes),
            ));
        }
        file.write_all(&chunk).await.map_err(storage_failure)?;
        on_progress(received, total);
    }
    if received == 0 || total.is_some_and(|total| total != received) {
        return Err(fail(
            InstallerFailure::Network,
            format_args!("body ended at {received} of {total:?} bytes"),
        ));
    }
    Ok(received)
}

/// Moves the finished partial file to the first free name in `dest_dir`. The
/// move refuses to replace an existing file, so nothing is ever overwritten
/// even when another program creates the same name at the same moment.
fn persist_without_overwrite(
    mut partial: TempPath,
    dest_dir: &Path,
    source: &InstallerSource,
) -> Result<PathBuf, InstallerFailure> {
    for attempt in 0..MAX_NAME_ATTEMPTS {
        let candidate = dest_dir.join(candidate_file_name(
            &source.file_name,
            source.extension,
            attempt,
        ));
        match partial.persist_noclobber(&candidate) {
            Ok(()) => return Ok(candidate),
            Err(error) if error.error.kind() == io::ErrorKind::AlreadyExists => {
                partial = error.path;
            }
            // Dropping the error drops the partial path, which removes the file.
            Err(error) => return Err(storage_failure(error.error)),
        }
    }
    Err(fail(
        InstallerFailure::Storage,
        "no free file name in the Downloads folder",
    ))
}

/// Creates `dest_dir` if needed and `<file name>.<random>.part` in it.
///
/// On Unix the file is requested with mode 0644 and the user's umask applies
/// on top. A stricter umask is honoured, and a looser one never makes the
/// installer writable by anyone but its owner. It is never executable. The
/// temporary-file default is private to the user, which is wrong for a file
/// that ends up in Downloads.
fn create_partial(dest_dir: &Path, file_name: &str) -> io::Result<(std::fs::File, TempPath)> {
    std::fs::create_dir_all(dest_dir)?;
    let prefix = format!("{file_name}.");
    let mut builder = tempfile::Builder::new();
    builder.prefix(&prefix).suffix(PARTIAL_SUFFIX);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        builder.permissions(std::fs::Permissions::from_mode(0o644));
    }
    Ok(builder.tempfile_in(dest_dir)?.into_parts())
}

/// Downloads `source` into `dest_dir` and returns the final path.
///
/// The body is written to `<file name>.<random>.part`, which is removed on
/// every path that does not end in success: failure, cancellation, and the
/// future being dropped. `mark` runs on the complete partial file before it is
/// moved into place, so a file under the final name always carries the mark.
#[allow(clippy::too_many_arguments)]
pub(super) async fn download_installer(
    client: &reqwest::Client,
    source: &InstallerSource,
    dest_dir: &Path,
    limits: TransferLimits,
    cancel: &CancellationToken,
    on_partial: impl FnOnce(&Path),
    mark: impl FnOnce(&Path, &Url) -> io::Result<()>,
    mut emit: impl FnMut(InstallerDownloadEvent),
) -> Result<PathBuf, TransferError> {
    let storage = |error: io::Error| TransferError::Failed(storage_failure(error));

    // Off the async workers: on macOS the first write to Downloads can wait on
    // the system's consent prompt for as long as the user takes to answer it.
    let (file, partial) = {
        let dest_dir = dest_dir.to_path_buf();
        let file_name = source.file_name.clone();
        tokio::task::spawn_blocking(move || create_partial(&dest_dir, &file_name))
            .await
            .map_err(io::Error::other)
            .and_then(|created| created)
            .map_err(storage)?
    };
    on_partial(&partial);
    let mut file = tokio::fs::File::from_std(file);

    let mut throttle = ProgressThrottle::default();
    let transfer = fetch_into(
        client,
        source.url.clone(),
        &mut file,
        limits,
        |received_bytes, total_bytes| {
            if throttle.admit(received_bytes, total_bytes, Instant::now()) {
                emit(InstallerDownloadEvent::Progress {
                    received_bytes,
                    total_bytes,
                });
            }
        },
    );
    tokio::select! {
        biased;
        () = cancel.cancelled() => return Err(TransferError::Cancelled),
        result = tokio::time::timeout(limits.total_timeout, transfer) => match result {
            Ok(Ok(_)) => {}
            Ok(Err(reason)) => return Err(TransferError::Failed(reason)),
            Err(_) => {
                return Err(TransferError::Failed(fail(
                    InstallerFailure::Network,
                    "download took too long",
                )))
            }
        },
    }

    file.flush().await.map_err(storage)?;
    file.sync_all().await.map_err(storage)?;
    drop(file);
    mark(&partial, &source.url)
        .map_err(|error| TransferError::Failed(fail(InstallerFailure::Protection, error)))?;
    // Last point at which a cancel still leaves nothing behind.
    if cancel.is_cancelled() {
        return Err(TransferError::Cancelled);
    }
    persist_without_overwrite(partial, dest_dir, source).map_err(TransferError::Failed)
}

// Written on macOS only. Tests check the value's shape on every platform.
#[cfg(any(target_os = "macos", test))]
const QUARANTINE_AGENT: &str = "Buzz";

/// The value a non-sandboxed browser writes to `com.apple.quarantine` for a
/// web download: flags, hex timestamp, downloading app, and an event id that
/// is empty because no LaunchServices quarantine event is recorded.
#[cfg(any(target_os = "macos", test))]
pub(super) fn quarantine_value(unix_seconds: u64) -> String {
    format!("0081;{unix_seconds:08x};{QUARANTINE_AGENT};")
}

/// The NTFS alternate data stream Windows reads the Mark of the Web from.
pub(super) fn zone_identifier_path(path: &Path) -> PathBuf {
    let mut stream = path.as_os_str().to_os_string();
    stream.push(":Zone.Identifier");
    PathBuf::from(stream)
}

/// Zone 3 is the internet zone, which is what makes SmartScreen check the file.
pub(super) fn zone_identifier_contents(origin: &Url) -> String {
    format!("[ZoneTransfer]\r\nZoneId=3\r\nHostUrl={origin}\r\n")
}

/// Writes the Mark of the Web and reads it back. A volume without alternate
/// data streams fails here, and the download fails with it.
pub(super) fn write_zone_identifier(path: &Path, origin: &Url) -> io::Result<()> {
    let stream = zone_identifier_path(path);
    let contents = zone_identifier_contents(origin);
    std::fs::write(&stream, &contents)?;
    if std::fs::read(&stream)? != contents.as_bytes() {
        return Err(io::Error::other("Zone.Identifier did not persist"));
    }
    Ok(())
}

#[cfg(target_os = "macos")]
pub(super) const QUARANTINE_XATTR: &str = "com.apple.quarantine";

#[cfg(target_os = "macos")]
fn write_quarantine(path: &Path, origin: &Url) -> io::Result<()> {
    const WHERE_FROMS: &str = "com.apple.metadata:kMDItemWhereFroms";

    let unix_seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(io::Error::other)?
        .as_secs();
    let value = quarantine_value(unix_seconds);
    xattr::set(path, QUARANTINE_XATTR, value.as_bytes())?;
    if xattr::get(path, QUARANTINE_XATTR)?.as_deref() != Some(value.as_bytes()) {
        return Err(io::Error::other("quarantine attribute did not persist"));
    }

    // Finder's "Where from". Nothing enforces it, so it is best effort.
    let mut where_froms = Vec::new();
    if plist::to_writer_binary(&mut where_froms, &[origin.as_str()]).is_ok() {
        if let Err(error) = xattr::set(path, WHERE_FROMS, &where_froms) {
            eprintln!("buzz-desktop: could not record the installer's origin: {error}");
        }
    }
    Ok(())
}

/// Marks a finished download the way a browser does, so the system checks the
/// installer the first time it is opened: the quarantine attribute on macOS
/// (Gatekeeper) and the Mark of the Web on Windows (SmartScreen). An error
/// means the mark is not on the file, and the caller must not keep it. Linux
/// has no such mechanism, so nothing is written there.
pub(super) fn mark_as_downloaded(path: &Path, origin: &Url) -> io::Result<()> {
    #[cfg(target_os = "macos")]
    write_quarantine(path, origin)?;
    if cfg!(windows) {
        write_zone_identifier(path, origin)?;
    }
    Ok(())
}

/// The folder installers are saved to. A Linux desktop with no XDG download
/// directory configured reports none, so `~/Downloads` is used there; it is
/// created on first use.
pub(super) fn resolve_download_dir(
    configured: Option<PathBuf>,
    home: Option<PathBuf>,
    is_linux: bool,
) -> Option<PathBuf> {
    configured.or_else(|| home.filter(|_| is_linux).map(|home| home.join("Downloads")))
}

/// Records the partial file by name only. The folder is not recorded: the
/// next launch looks in the Downloads folder it resolves itself, so the record
/// can never point the cleanup at a file anywhere else.
fn write_partial_record(record: &Path, partial: &Path) -> io::Result<()> {
    let name = partial
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| io::Error::other("the partial file's name is not UTF-8"))?;
    if let Some(parent) = record.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(record, name)
}

fn clear_partial_record(record: &Path) {
    match std::fs::remove_file(record) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => {
            eprintln!("buzz-desktop: could not clear the partial installer record: {error}")
        }
    }
}

/// Whether nothing is left at `partial`: it was removed, or it was moved to
/// the installer's final name. An error other than "not found" does not prove
/// the file is gone.
fn partial_is_gone(partial: &Path) -> bool {
    matches!(
        std::fs::symlink_metadata(partial),
        Err(error) if error.kind() == io::ErrorKind::NotFound
    )
}

/// Whether `name` has the shape of a partial file this module writes:
/// `<installer name>.<random>.part`. Such a name holds only letters, digits
/// and `._+-`, so it has no separator and names a file directly in the folder
/// it is joined to.
fn is_partial_file_name(name: &str) -> bool {
    let Some((installer, random)) = name
        .strip_suffix(PARTIAL_SUFFIX)
        .and_then(|rest| rest.rsplit_once('.'))
    else {
        return false;
    };
    !random.is_empty()
        && random.bytes().all(|byte| byte.is_ascii_alphanumeric())
        && INSTALLER_EXTENSIONS
            .iter()
            .any(|extension| is_safe_file_name(installer, extension))
}

/// Removes the partial file a previous run recorded and never cleaned up: a
/// crash, a force quit or a power cut ends the app without any of its cleanup.
///
/// The record holds a file name, and only that name directly inside
/// `download_dir` is touched, and only while it is still a regular file of
/// this module's shape. No folder is searched and no recorded folder is
/// trusted, so a file this app did not create is never removed. Without a
/// Downloads folder nothing can be looked up, and the record is kept.
pub(super) fn remove_recorded_partial(record: &Path, download_dir: Option<&Path>) {
    use std::io::Read;

    // A file name is far shorter than this; anything longer is not a record.
    const MAX_RECORD_BYTES: u64 = 16 * 1024;

    let mut recorded = String::new();
    match std::fs::File::open(record)
        .and_then(|file| file.take(MAX_RECORD_BYTES).read_to_string(&mut recorded))
    {
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => return,
        Err(error) => {
            eprintln!("buzz-desktop: could not read the partial installer record: {error}");
            recorded.clear();
        }
    }
    if is_partial_file_name(&recorded) {
        let Some(download_dir) = download_dir else {
            eprintln!("buzz-desktop: no Downloads folder to look for a leftover partial installer");
            return;
        };
        let partial = download_dir.join(&recorded);
        // A regular file, not a link to one.
        let is_file = std::fs::symlink_metadata(&partial).is_ok_and(|metadata| metadata.is_file());
        if is_file {
            if let Err(error) = std::fs::remove_file(&partial) {
                // The record stays, so the next launch tries again.
                eprintln!(
                    "buzz-desktop: could not remove leftover partial installer {}: {error}",
                    partial.display()
                );
                return;
            }
        }
    }
    clear_partial_record(record);
}

#[derive(Default)]
struct Slots {
    active: Option<Active>,
    last_saved: Option<PathBuf>,
}

struct Active {
    cancel: CancellationToken,
    partial: Option<PathBuf>,
}

/// The one installer download the app runs at a time, and the last file it
/// saved.
#[derive(Default)]
pub struct InstallerDownloads {
    slots: Mutex<Slots>,
    client: OnceLock<Option<reqwest::Client>>,
    /// Where the running download's partial file is recorded. Unset until
    /// [`InstallerDownloads::recover`] runs.
    partial_record: OnceLock<PathBuf>,
}

/// Holds the download slot; dropping it frees the slot.
pub(super) struct ActiveDownload<'a> {
    downloads: &'a InstallerDownloads,
    pub(super) cancel: CancellationToken,
}

impl Drop for ActiveDownload<'_> {
    fn drop(&mut self) {
        // By now the partial file has become the installer or has been
        // removed, unless its removal failed: then the record stays, and the
        // next launch removes the file. The record is settled first, while
        // the slot still keeps the next download out.
        let partial = self
            .downloads
            .slots()
            .active
            .as_ref()
            .and_then(|active| active.partial.clone());
        if partial.as_deref().is_none_or(partial_is_gone) {
            self.downloads.forget_partial();
        }
        self.downloads.slots().active = None;
    }
}

impl InstallerDownloads {
    fn slots(&self) -> MutexGuard<'_, Slots> {
        self.slots.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Claims the download slot, or returns `None` while a download holds it.
    pub(super) fn begin(&self) -> Option<ActiveDownload<'_>> {
        let mut slots = self.slots();
        if slots.active.is_some() {
            return None;
        }
        let cancel = CancellationToken::new();
        slots.active = Some(Active {
            cancel: cancel.clone(),
            partial: None,
        });
        Some(ActiveDownload {
            downloads: self,
            cancel,
        })
    }

    /// Removes the partial file a previous run left behind in `download_dir`,
    /// then records each download's partial file in `record` from here on.
    /// Called once at startup, before any download can start.
    pub(super) fn recover(&self, record: PathBuf, download_dir: Option<&Path>) {
        remove_recorded_partial(&record, download_dir);
        let _ = self.partial_record.set(record);
    }

    pub(super) fn set_partial(&self, partial: &Path) {
        if let Some(active) = self.slots().active.as_mut() {
            active.partial = Some(partial.to_path_buf());
        }
        if let Some(record) = self.partial_record.get() {
            // Without the record a crash leaves this file behind; the download
            // itself is not affected.
            if let Err(error) = write_partial_record(record, partial) {
                eprintln!("buzz-desktop: could not record the partial installer: {error}");
            }
        }
    }

    fn forget_partial(&self) {
        if let Some(record) = self.partial_record.get() {
            clear_partial_record(record);
        }
    }

    /// Asks the running download, if any, to stop. The download removes its
    /// partial file and reports that it was cancelled.
    pub(crate) fn cancel(&self) {
        if let Some(active) = self.slots().active.as_ref() {
            active.cancel.cancel();
        }
    }

    /// Stops the running download as the app exits, on the normal exit path
    /// and from the signal handler. The download task may never be polled
    /// again, so the partial file is removed here. If that fails its record
    /// stays, here and when the download task later frees its slot, and the
    /// next launch removes the file.
    pub(crate) fn abort_for_shutdown(&self) {
        let partial = {
            let slots = self.slots();
            let Some(active) = slots.active.as_ref() else {
                return;
            };
            active.cancel.cancel();
            active.partial.clone()
        };
        if let Some(partial) = partial {
            match std::fs::remove_file(&partial) {
                Ok(()) => {}
                // The download finished or cleaned up first.
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => {
                    eprintln!(
                        "buzz-desktop: could not remove partial installer {}: {error}",
                        partial.display()
                    );
                    return;
                }
            }
        }
        self.forget_partial();
    }

    fn client(&self) -> Option<&reqwest::Client> {
        self.client
            .get_or_init(|| match build_installer_client() {
                Ok(client) => Some(client),
                Err(error) => {
                    eprintln!("buzz-desktop: could not build the installer client: {error}");
                    None
                }
            })
            .as_ref()
    }
}

/// The Downloads folder of the running app.
fn download_dir(app: &AppHandle) -> Option<PathBuf> {
    resolve_download_dir(
        app.path().download_dir().ok(),
        app.path().home_dir().ok(),
        cfg!(target_os = "linux"),
    )
}

async fn run_download(
    app: &AppHandle,
    downloads: &InstallerDownloads,
    active: &ActiveDownload<'_>,
    url: &str,
    on_event: &Channel<InstallerDownloadEvent>,
) -> Result<(PathBuf, PathBuf), TransferError> {
    let source = new_app_installer_target()
        .and_then(|target| validate_installer_url(url, target))
        .ok_or_else(|| TransferError::Failed(fail(InstallerFailure::Rejected, url)))?;
    let client = downloads
        .client()
        .ok_or(TransferError::Failed(InstallerFailure::Network))?;
    let dest_dir = download_dir(app).ok_or_else(|| {
        TransferError::Failed(fail(InstallerFailure::Storage, "no Downloads folder"))
    })?;

    // A closed webview cannot receive events; the transfer is cancelled through
    // the window hooks, not through a failed send.
    let _ = on_event.send(InstallerDownloadEvent::Started {
        file_name: source.file_name.clone(),
    });
    let saved = download_installer(
        client,
        &source,
        &dest_dir,
        TransferLimits::PRODUCTION,
        &active.cancel,
        |partial| downloads.set_partial(partial),
        mark_as_downloaded,
        |event| {
            let _ = on_event.send(event);
        },
    )
    .await?;
    Ok((saved, dest_dir))
}

/// Downloads the new Buzz app's installer into the Downloads folder.
///
/// `url` must be the installer the manifest lists for this machine; it is
/// validated again here. Progress arrives on `on_event`. Only one download
/// runs at a time: a call made while another is running returns
/// `Failed { reason: Busy }` without touching the running one.
#[tauri::command]
pub async fn download_new_app_installer(
    app: AppHandle,
    url: String,
    on_event: Channel<InstallerDownloadEvent>,
) -> InstallerDownloadOutcome {
    let downloads = app.state::<InstallerDownloads>();
    let Some(active) = downloads.begin() else {
        return InstallerDownloadOutcome::Failed {
            reason: InstallerFailure::Busy,
        };
    };
    match run_download(&app, &downloads, &active, &url, &on_event).await {
        Ok((saved, folder)) => {
            let file_name = saved
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            downloads.slots().last_saved = Some(saved);
            InstallerDownloadOutcome::Saved {
                file_name,
                folder: folder.display().to_string(),
            }
        }
        Err(TransferError::Cancelled) => InstallerDownloadOutcome::Cancelled,
        Err(TransferError::Failed(reason)) => InstallerDownloadOutcome::Failed { reason },
    }
}

/// Removes the partial installer a crashed or force-quit run left in the
/// Downloads folder, and starts recording each new one so the next launch can
/// do the same. Run once from the app's setup hook.
pub fn recover_new_app_installer_downloads(app: &AppHandle) {
    match app.path().app_data_dir() {
        Ok(data_dir) => app.state::<InstallerDownloads>().recover(
            data_dir.join(PARTIAL_RECORD_NAME),
            download_dir(app).as_deref(),
        ),
        Err(error) => {
            eprintln!("buzz-desktop: partial installer downloads are not recorded: {error}");
        }
    }
}

/// Cancels the running installer download, if any. The download command then
/// returns `Cancelled` once its partial file is gone.
#[tauri::command]
pub fn cancel_new_app_installer_download(app: AppHandle) {
    app.state::<InstallerDownloads>().cancel();
}

/// Shows the last installer this app saved in the system file manager, with
/// the file selected. Takes no path: the webview cannot point it elsewhere.
#[tauri::command]
pub async fn reveal_new_app_installer(app: AppHandle) -> Result<(), String> {
    let saved = app
        .state::<InstallerDownloads>()
        .slots()
        .last_saved
        .clone()
        .ok_or_else(|| "no installer has been downloaded".to_string())?;
    tauri_plugin_opener::reveal_item_in_dir(&saved).map_err(|error| error.to_string())
}

#[cfg(test)]
#[path = "new_app_installer_tests.rs"]
mod tests;
