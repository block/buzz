use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::sync::{Mutex, OnceLock};

use serde::Deserialize;
use sha2::{Digest, Sha256};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ArtifactManifest {
    schema_version: u32,
    release: Option<String>,
    artifacts: Vec<ManagedArtifact>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ManagedArtifact {
    os: String,
    arch: String,
    url: String,
    filename: String,
    sha256: String,
    max_bytes: u64,
    max_extracted_bytes: u64,
    archive: ArchiveKind,
    executable: String,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ArchiveKind {
    Raw,
    Zip,
    TarGz,
}

struct ManagedRuntime {
    child: Child,
}

fn runtime_slot() -> &'static Mutex<Option<ManagedRuntime>> {
    static SLOT: OnceLock<Mutex<Option<ManagedRuntime>>> = OnceLock::new();
    SLOT.get_or_init(|| Mutex::new(None))
}

fn runtime_transition() -> &'static Mutex<()> {
    static TRANSITION: OnceLock<Mutex<()>> = OnceLock::new();
    TRANSITION.get_or_init(|| Mutex::new(()))
}

fn install_lock() -> &'static tokio::sync::Mutex<()> {
    static LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
}

fn manifest() -> Result<ArtifactManifest, String> {
    let manifest: ArtifactManifest = serde_json::from_str(include_str!("artifacts.json"))
        .map_err(|error| format!("parse managed Ollama artifact manifest: {error}"))?;
    validate_manifest(&manifest)?;
    Ok(manifest)
}

fn validate_manifest(manifest: &ArtifactManifest) -> Result<(), String> {
    if manifest.schema_version != 1 {
        return Err("unsupported managed Ollama artifact manifest schema".to_string());
    }
    if manifest.artifacts.is_empty() && manifest.release.is_some() {
        return Err("managed Ollama manifest declares a release without artifacts".to_string());
    }
    if let Some(release) = manifest.release.as_deref() {
        if !safe_component(release) {
            return Err("managed Ollama release must be one safe path component".to_string());
        }
    }
    for artifact in &manifest.artifacts {
        if artifact.sha256.len() != 64
            || !artifact.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(format!(
                "managed Ollama artifact {} has an invalid SHA-256",
                artifact.filename
            ));
        }
        if artifact.max_bytes == 0 {
            return Err(format!(
                "managed Ollama artifact {} has no download size limit",
                artifact.filename
            ));
        }
        if artifact.max_extracted_bytes == 0 {
            return Err(format!(
                "managed Ollama artifact {} has no extracted-size limit",
                artifact.filename
            ));
        }
        if !safe_component(&artifact.filename) {
            return Err(
                "managed Ollama artifact filename must be one safe path component".to_string(),
            );
        }
        let url = url::Url::parse(&artifact.url)
            .map_err(|error| format!("invalid Ollama artifact URL: {error}"))?;
        if url.scheme() != "https" || url.host_str() != Some("github.com") {
            return Err(
                "managed Ollama artifacts must use an official github.com HTTPS URL".to_string(),
            );
        }
        let executable = Path::new(&artifact.executable);
        if artifact.executable.is_empty()
            || executable.is_absolute()
            || executable.components().any(|component| {
                matches!(
                    component,
                    std::path::Component::ParentDir
                        | std::path::Component::RootDir
                        | std::path::Component::Prefix(_)
                )
            })
        {
            return Err("managed Ollama executable path must be relative".to_string());
        }
    }
    Ok(())
}

fn safe_component(value: &str) -> bool {
    !value.is_empty()
        && value != "."
        && value != ".."
        && !value.contains('/')
        && !value.contains('\\')
        && !value.chars().any(char::is_control)
}

fn selected_artifact(manifest: &ArtifactManifest) -> Option<&ManagedArtifact> {
    manifest.artifacts.iter().find(|artifact| {
        artifact.os == std::env::consts::OS && artifact.arch == std::env::consts::ARCH
    })
}

fn root() -> Result<PathBuf, String> {
    dirs::data_dir()
        .map(|path| path.join("Buzz").join("runtimes").join("ollama"))
        .ok_or_else(|| "failed to resolve the app-data directory for managed Ollama".to_string())
}

fn model_root() -> Result<PathBuf, String> {
    dirs::data_dir()
        .map(|path| path.join("Buzz").join("ollama").join("models"))
        .ok_or_else(|| {
            "failed to resolve the app-data directory for managed Ollama models".to_string()
        })
}

fn executable_path() -> Option<PathBuf> {
    let manifest = manifest().ok()?;
    let release = manifest.release.as_deref()?;
    let artifact = selected_artifact(&manifest)?;
    root()
        .ok()
        .map(|root| root.join(release).join(&artifact.executable))
}

pub(crate) fn install_supported() -> bool {
    manifest().ok().is_some_and(|manifest| {
        manifest.release.is_some() && selected_artifact(&manifest).is_some()
    })
}

pub(crate) fn runtime_installed() -> bool {
    executable_path().is_some_and(|path| path.is_file())
}

pub(crate) fn runtime_running() -> Result<bool, String> {
    let mut slot = runtime_slot()
        .lock()
        .map_err(|_| "managed Ollama runtime lock poisoned".to_string())?;
    let exited = match slot.as_mut() {
        Some(runtime) => runtime
            .child
            .try_wait()
            .map_err(|error| format!("inspect managed Ollama runtime: {error}"))?
            .is_some(),
        None => return Ok(false),
    };
    if exited {
        slot.take();
        Ok(false)
    } else {
        Ok(true)
    }
}

pub(crate) async fn install() -> Result<(), String> {
    let _guard = install_lock().lock().await;
    if runtime_installed() {
        return Ok(());
    }
    let manifest = manifest()?;
    let release = manifest.release.clone().ok_or_else(|| {
        "managed Ollama installation is not enabled in this Buzz build; connect to an existing Ollama installation instead".to_string()
    })?;
    let artifact = selected_artifact(&manifest).cloned().ok_or_else(|| {
        format!(
            "managed Ollama is not packaged for {}-{}",
            std::env::consts::OS,
            std::env::consts::ARCH
        )
    })?;
    tokio::task::spawn_blocking(move || install_blocking(&release, &artifact))
        .await
        .map_err(|error| format!("managed Ollama installer task failed: {error}"))?
}

fn install_blocking(release: &str, artifact: &ManagedArtifact) -> Result<(), String> {
    let root = root()?;
    std::fs::create_dir_all(&root)
        .map_err(|error| format!("create managed Ollama root: {error}"))?;
    let final_dir = root.join(release);
    let temp_dir = root.join(format!("{release}.tmp"));
    let archive_path = root.join(format!("{}.download", artifact.filename));
    if temp_dir.exists() {
        std::fs::remove_dir_all(&temp_dir)
            .map_err(|error| format!("remove stale Ollama install: {error}"))?;
    }
    if archive_path.exists() {
        std::fs::remove_file(&archive_path)
            .map_err(|error| format!("remove stale Ollama download: {error}"))?;
    }
    download(artifact, &archive_path)?;
    std::fs::create_dir_all(&temp_dir)
        .map_err(|error| format!("create Ollama install staging directory: {error}"))?;
    let install_result = extract(artifact, &archive_path, &temp_dir).and_then(|()| {
        let executable = temp_dir.join(&artifact.executable);
        if !executable.is_file() {
            return Err(
                "managed Ollama archive did not contain the declared executable".to_string(),
            );
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut permissions = executable
                .metadata()
                .map_err(|error| format!("inspect managed Ollama executable: {error}"))?
                .permissions();
            permissions.set_mode(0o700);
            std::fs::set_permissions(&executable, permissions)
                .map_err(|error| format!("set managed Ollama executable permissions: {error}"))?;
        }
        replace_directory(&temp_dir, &final_dir)
    });
    let _ = std::fs::remove_file(&archive_path);
    if install_result.is_err() {
        let _ = std::fs::remove_dir_all(&temp_dir);
    }
    install_result
}

fn download(artifact: &ManagedArtifact, destination: &Path) -> Result<(), String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(15 * 60))
        .redirect(reqwest::redirect::Policy::limited(3))
        .build()
        .map_err(|error| format!("build managed Ollama download client: {error}"))?;
    let mut response = client
        .get(&artifact.url)
        .send()
        .map_err(|error| format!("download managed Ollama: {error}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "managed Ollama download failed with HTTP {}",
            response.status().as_u16()
        ));
    }
    if response
        .content_length()
        .is_some_and(|size| size > artifact.max_bytes)
    {
        return Err("managed Ollama download exceeded its declared size limit".to_string());
    }
    let mut output = std::fs::File::create(destination)
        .map_err(|error| format!("create managed Ollama download: {error}"))?;
    let mut hasher = Sha256::new();
    let mut downloaded = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = response
            .read(&mut buffer)
            .map_err(|error| format!("read managed Ollama download: {error}"))?;
        if count == 0 {
            break;
        }
        downloaded = downloaded.saturating_add(count as u64);
        if downloaded > artifact.max_bytes {
            let _ = std::fs::remove_file(destination);
            return Err("managed Ollama download exceeded its declared size limit".to_string());
        }
        output
            .write_all(&buffer[..count])
            .map_err(|error| format!("write managed Ollama download: {error}"))?;
        hasher.update(&buffer[..count]);
    }
    output
        .flush()
        .map_err(|error| format!("flush managed Ollama download: {error}"))?;
    let actual = hex::encode(hasher.finalize());
    if actual != artifact.sha256.to_ascii_lowercase() {
        let _ = std::fs::remove_file(destination);
        return Err("managed Ollama download checksum mismatch".to_string());
    }
    Ok(())
}

fn extract(
    artifact: &ManagedArtifact,
    archive_path: &Path,
    destination: &Path,
) -> Result<(), String> {
    match artifact.archive {
        ArchiveKind::Raw => {
            let size = std::fs::metadata(archive_path)
                .map_err(|error| format!("inspect managed Ollama executable: {error}"))?
                .len();
            if size > artifact.max_extracted_bytes {
                return Err(
                    "managed Ollama executable exceeded its extracted-size limit".to_string(),
                );
            }
            let target = destination.join(&artifact.executable);
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent).map_err(|error| {
                    format!("create managed Ollama executable directory: {error}")
                })?;
            }
            std::fs::copy(archive_path, target)
                .map_err(|error| format!("stage managed Ollama executable: {error}"))?;
            Ok(())
        }
        ArchiveKind::Zip => extract_zip(archive_path, destination, artifact.max_extracted_bytes),
        ArchiveKind::TarGz => {
            extract_tar_gz(archive_path, destination, artifact.max_extracted_bytes)
        }
    }
}

fn extract_zip(
    archive_path: &Path,
    destination: &Path,
    max_extracted_bytes: u64,
) -> Result<(), String> {
    let file = std::fs::File::open(archive_path)
        .map_err(|error| format!("open managed Ollama zip: {error}"))?;
    let mut archive =
        zip::ZipArchive::new(file).map_err(|error| format!("read managed Ollama zip: {error}"))?;
    let mut extracted = 0_u64;
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|error| format!("read managed Ollama zip entry: {error}"))?;
        if entry.size() > max_extracted_bytes.saturating_sub(extracted) {
            return Err("managed Ollama archive exceeded its extracted-size limit".to_string());
        }
        let relative = entry
            .enclosed_name()
            .ok_or_else(|| "managed Ollama zip contains an unsafe path".to_string())?;
        let target = destination.join(relative);
        if entry.is_dir() {
            std::fs::create_dir_all(&target)
                .map_err(|error| format!("create managed Ollama zip directory: {error}"))?;
        } else {
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|error| format!("create managed Ollama zip parent: {error}"))?;
            }
            let mut output = std::fs::File::create(&target)
                .map_err(|error| format!("create managed Ollama zip file: {error}"))?;
            let remaining = max_extracted_bytes.saturating_sub(extracted);
            let copied = std::io::copy(
                &mut entry.by_ref().take(remaining.saturating_add(1)),
                &mut output,
            )
            .map_err(|error| format!("extract managed Ollama zip file: {error}"))?;
            if copied > remaining {
                return Err("managed Ollama archive exceeded its extracted-size limit".to_string());
            }
            extracted = extracted
                .checked_add(copied)
                .ok_or_else(|| "managed Ollama extracted-size total overflowed".to_string())?;
        }
    }
    Ok(())
}

fn extract_tar_gz(
    archive_path: &Path,
    destination: &Path,
    max_extracted_bytes: u64,
) -> Result<(), String> {
    let file = std::fs::File::open(archive_path)
        .map_err(|error| format!("open managed Ollama archive: {error}"))?;
    let decoder = flate2::read::GzDecoder::new(file);
    let mut archive = tar::Archive::new(decoder);
    let mut extracted = 0_u64;
    for entry in archive
        .entries()
        .map_err(|error| format!("read managed Ollama archive: {error}"))?
    {
        let mut entry =
            entry.map_err(|error| format!("read managed Ollama archive entry: {error}"))?;
        if !entry.header().entry_type().is_file() && !entry.header().entry_type().is_dir() {
            return Err(
                "managed Ollama archive contains a link or unsupported entry type".to_string(),
            );
        }
        let size = entry
            .header()
            .size()
            .map_err(|error| format!("read managed Ollama archive entry size: {error}"))?;
        extracted = extracted
            .checked_add(size)
            .ok_or_else(|| "managed Ollama extracted-size total overflowed".to_string())?;
        if extracted > max_extracted_bytes {
            return Err("managed Ollama archive exceeded its extracted-size limit".to_string());
        }
        let path = entry
            .path()
            .map_err(|error| format!("read managed Ollama archive path: {error}"))?;
        if path.is_absolute()
            || path
                .components()
                .any(|part| matches!(part, std::path::Component::ParentDir))
        {
            return Err("managed Ollama archive contains an unsafe path".to_string());
        }
        entry
            .unpack_in(destination)
            .map_err(|error| format!("extract managed Ollama archive: {error}"))?;
    }
    Ok(())
}

fn replace_directory(staged: &Path, final_dir: &Path) -> Result<(), String> {
    let old = final_dir.with_extension("old");
    if old.exists() {
        std::fs::remove_dir_all(&old)
            .map_err(|error| format!("remove previous Ollama backup: {error}"))?;
    }
    if final_dir.exists() {
        std::fs::rename(final_dir, &old)
            .map_err(|error| format!("stage previous Ollama runtime: {error}"))?;
    }
    if let Err(error) = std::fs::rename(staged, final_dir) {
        if old.exists() {
            let _ = std::fs::rename(&old, final_dir);
        }
        return Err(format!("install managed Ollama runtime: {error}"));
    }
    let _ = std::fs::remove_dir_all(old);
    Ok(())
}

pub(crate) fn start() -> Result<(), String> {
    let _transition = runtime_transition()
        .lock()
        .map_err(|_| "managed Ollama runtime transition lock poisoned".to_string())?;
    if runtime_running()? {
        return Ok(());
    }
    let executable = executable_path()
        .filter(|path| path.is_file())
        .ok_or_else(|| "managed Ollama is not installed".to_string())?;
    let address: std::net::SocketAddr = "127.0.0.1:11434"
        .parse()
        .map_err(|error| format!("parse managed Ollama loopback address: {error}"))?;
    if std::net::TcpStream::connect_timeout(&address, std::time::Duration::from_millis(300)).is_ok()
    {
        return Err(
            "port 11434 is already in use; connect to the existing Ollama installation or stop the owning process"
                .to_string(),
        );
    }
    let models = model_root()?;
    std::fs::create_dir_all(&models)
        .map_err(|error| format!("create managed Ollama model directory: {error}"))?;
    let logs = root()?.join("logs");
    std::fs::create_dir_all(&logs)
        .map_err(|error| format!("create managed Ollama log directory: {error}"))?;
    let stdout = std::fs::File::create(logs.join("ollama.log"))
        .map_err(|error| format!("create managed Ollama log: {error}"))?;
    let stderr = stdout
        .try_clone()
        .map_err(|error| format!("clone managed Ollama log: {error}"))?;
    let mut command = std::process::Command::new(executable);
    command
        .arg("serve")
        .env("OLLAMA_HOST", "127.0.0.1:11434")
        .env("OLLAMA_MODELS", models)
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr));
    crate::util::configure_no_window(&mut command);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = command
        .spawn()
        .map_err(|error| format!("start managed Ollama: {error}"))?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        if std::net::TcpStream::connect_timeout(&address, std::time::Duration::from_millis(200))
            .is_ok()
        {
            break;
        }
        if let Some(status) = child
            .try_wait()
            .map_err(|error| format!("inspect managed Ollama startup: {error}"))?
        {
            return Err(format!(
                "managed Ollama exited before becoming ready ({status})"
            ));
        }
        if std::time::Instant::now() >= deadline {
            let pid = child.id();
            let _ = crate::managed_agents::terminate_process(pid);
            let _ = child.wait();
            return Err("managed Ollama did not become ready within 10 seconds".to_string());
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    let mut slot = runtime_slot()
        .lock()
        .map_err(|_| "managed Ollama runtime lock poisoned".to_string())?;
    *slot = Some(ManagedRuntime { child });
    Ok(())
}

pub(crate) fn stop() -> Result<(), String> {
    let _transition = runtime_transition()
        .lock()
        .map_err(|_| "managed Ollama runtime transition lock poisoned".to_string())?;
    let mut slot = runtime_slot()
        .lock()
        .map_err(|_| "managed Ollama runtime lock poisoned".to_string())?;
    let Some(mut runtime) = slot.take() else {
        return Ok(());
    };
    let result = crate::managed_agents::terminate_process(runtime.child.id()).and_then(|()| {
        runtime
            .child
            .wait()
            .map(|_| ())
            .map_err(|error| format!("wait for managed Ollama: {error}"))
    });
    if result.is_err() {
        *slot = Some(runtime);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checked_in_manifest_is_valid_and_unpinned_until_checksums_are_verified() {
        let manifest = manifest().unwrap();
        assert_eq!(manifest.schema_version, 1);
        assert!(manifest.release.is_none());
        assert!(manifest.artifacts.is_empty());
        assert!(!install_supported());
    }

    #[test]
    fn rejects_unbounded_or_unverified_manifest_entries() {
        let manifest = ArtifactManifest {
            schema_version: 1,
            release: Some("v1".to_string()),
            artifacts: vec![ManagedArtifact {
                os: "linux".to_string(),
                arch: "x86_64".to_string(),
                url: "https://github.com/ollama/ollama/releases/download/v1/ollama".to_string(),
                filename: "ollama".to_string(),
                sha256: "nope".to_string(),
                max_bytes: 0,
                max_extracted_bytes: 0,
                archive: ArchiveKind::Raw,
                executable: "ollama".to_string(),
            }],
        };
        assert!(validate_manifest(&manifest).is_err());
    }

    #[test]
    fn raw_extraction_enforces_uncompressed_limit() {
        let source = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(source.path(), b"four").unwrap();
        let destination = tempfile::tempdir().unwrap();
        let artifact = test_artifact(ArchiveKind::Raw, 3);
        assert!(extract(&artifact, source.path(), destination.path()).is_err());
    }

    #[test]
    fn zip_extraction_enforces_cumulative_uncompressed_limit() {
        let mut archive = tempfile::NamedTempFile::new().unwrap();
        {
            let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
            writer
                .start_file("ollama", zip::write::SimpleFileOptions::default())
                .unwrap();
            writer.write_all(b"four").unwrap();
            let bytes = writer.finish().unwrap().into_inner();
            archive.write_all(&bytes).unwrap();
        }
        let destination = tempfile::tempdir().unwrap();
        assert!(extract_zip(archive.path(), destination.path(), 3).is_err());
    }

    #[test]
    fn tar_gz_extraction_enforces_cumulative_uncompressed_limit() {
        let archive = tempfile::NamedTempFile::new().unwrap();
        {
            let file = archive.reopen().unwrap();
            let encoder = flate2::write::GzEncoder::new(file, flate2::Compression::default());
            let mut builder = tar::Builder::new(encoder);
            let mut header = tar::Header::new_gnu();
            header.set_size(4);
            header.set_mode(0o700);
            header.set_cksum();
            builder
                .append_data(&mut header, "ollama", &b"four"[..])
                .unwrap();
            let encoder = builder.into_inner().unwrap();
            encoder.finish().unwrap();
        }
        let destination = tempfile::tempdir().unwrap();
        assert!(extract_tar_gz(archive.path(), destination.path(), 3).is_err());
    }

    fn test_artifact(archive: ArchiveKind, max_extracted_bytes: u64) -> ManagedArtifact {
        ManagedArtifact {
            os: std::env::consts::OS.to_string(),
            arch: std::env::consts::ARCH.to_string(),
            url: "https://github.com/ollama/ollama/releases/download/v1/ollama".to_string(),
            filename: "ollama".to_string(),
            sha256: "a".repeat(64),
            max_bytes: 1024,
            max_extracted_bytes,
            archive,
            executable: "ollama".to_string(),
        }
    }
}
