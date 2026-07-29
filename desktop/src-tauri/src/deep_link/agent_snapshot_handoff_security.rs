use std::{
    io::Read,
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

#[cfg(target_os = "macos")]
use std::ffi::CString;
#[cfg(target_os = "macos")]
use std::os::fd::{AsRawFd, FromRawFd};
#[cfg(target_os = "macos")]
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};

use url::Url;

use crate::{
    decode_snapshot_from_bytes, managed_agents::agent_snapshot::MemoryLevel,
    MAX_SNAPSHOT_JSON_BYTES,
};

pub(super) const AGENT_SNAPSHOT_HANDOFF_MAX_AGE: Duration = Duration::from_secs(10 * 60);
pub(super) const AGENT_SNAPSHOT_HANDOFF_FUTURE_SKEW: Duration = Duration::from_secs(60);

pub(super) fn parse_agent_snapshot_handoff_id(url: &Url) -> Option<String> {
    if !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
        || !url.path().is_empty()
        || url.fragment().is_some()
    {
        return None;
    }
    let mut params = url.query_pairs();
    let (key, value) = params.next()?;
    if key != "handoff" || params.next().is_some() {
        return None;
    }
    let candidate = value.as_ref();
    let parsed = uuid::Uuid::parse_str(candidate).ok()?;
    (parsed.to_string() == candidate).then(|| candidate.to_owned())
}

pub(super) fn agent_snapshot_handoff_dir() -> Result<PathBuf, String> {
    dirs::home_dir()
        .map(|home| {
            home.join("Library")
                .join("Application Support")
                .join("Co-Agent")
                .join("buzz-handoffs")
        })
        .ok_or_else(|| "cannot resolve the current user's home directory".to_string())
}

#[cfg(target_os = "macos")]
pub(super) fn validate_agent_snapshot_handoff_metadata(
    metadata: &std::fs::Metadata,
    expected_uid: u32,
    now: SystemTime,
) -> Result<(), String> {
    if !metadata.file_type().is_file() {
        return Err("handoff is not a regular file".to_string());
    }
    if metadata.uid() != expected_uid {
        return Err("handoff is not owned by the current user".to_string());
    }
    if metadata.mode() & 0o077 != 0 {
        return Err("handoff has group or world permissions".to_string());
    }
    if metadata.nlink() != 1 {
        return Err("handoff must have exactly one filesystem link".to_string());
    }
    if metadata.len() > MAX_SNAPSHOT_JSON_BYTES as u64 {
        return Err("handoff exceeds the agent JSON snapshot size limit".to_string());
    }
    let modified = metadata
        .modified()
        .map_err(|error| format!("cannot inspect handoff age: {error}"))?;
    match now.duration_since(modified) {
        Ok(age) if age > AGENT_SNAPSHOT_HANDOFF_MAX_AGE => {
            return Err("handoff has expired".to_string());
        }
        Ok(_) => {}
        Err(error) if error.duration() > AGENT_SNAPSHOT_HANDOFF_FUTURE_SKEW => {
            return Err("handoff modification time is too far in the future".to_string());
        }
        Err(_) => {}
    }
    Ok(())
}

#[cfg(target_os = "macos")]
pub(super) fn validate_agent_snapshot_handoff_directory_metadata(
    metadata: &std::fs::Metadata,
    expected_uid: u32,
) -> Result<(), String> {
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err("handoff directory is not a real directory".to_string());
    }
    if metadata.uid() != expected_uid {
        return Err("handoff directory is not owned by the current user".to_string());
    }
    if metadata.mode() & 0o077 != 0 {
        return Err("handoff directory has group or world permissions".to_string());
    }
    Ok(())
}

fn validate_agent_snapshot_handoff_shape(bytes: &[u8]) -> Result<(), String> {
    fn require_only_keys(
        value: &serde_json::Value,
        allowed: &[&str],
        section: &str,
    ) -> Result<(), String> {
        let object = value
            .as_object()
            .ok_or_else(|| format!("handoff {section} must be a JSON object"))?;
        if let Some(key) = object.keys().find(|key| !allowed.contains(&key.as_str())) {
            return Err(format!("handoff {section} contains forbidden field {key}"));
        }
        Ok(())
    }

    let value: serde_json::Value = serde_json::from_slice(bytes)
        .map_err(|error| format!("invalid agent snapshot handoff JSON: {error}"))?;
    require_only_keys(
        &value,
        &["format", "version", "definition", "profile", "memory"],
        "root",
    )?;
    let root = value
        .as_object()
        .ok_or_else(|| "handoff root must be a JSON object".to_string())?;
    for (section, allowed) in [
        ("definition", &["name", "systemPrompt", "runtime"][..]),
        ("profile", &["displayName"][..]),
        ("memory", &["level"][..]),
    ] {
        let child = root
            .get(section)
            .ok_or_else(|| format!("handoff is missing {section}"))?;
        require_only_keys(child, allowed, section)?;
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn load_agent_snapshot_handoff_from_dir(
    dir: &Path,
    handoff_id: &str,
    consume_expected: Option<&[u8]>,
) -> Result<Vec<u8>, String> {
    let parsed =
        uuid::Uuid::parse_str(handoff_id).map_err(|_| "handoff id is not a UUID".to_string())?;
    if parsed.to_string() != handoff_id {
        return Err("handoff id is not a canonical lowercase UUID".to_string());
    }

    let expected_uid = unsafe { libc::geteuid() };
    let directory = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(dir)
        .map_err(|error| format!("cannot securely open handoff directory: {error}"))?;
    let dir_metadata = directory
        .metadata()
        .map_err(|error| format!("cannot inspect open handoff directory: {error}"))?;
    validate_agent_snapshot_handoff_directory_metadata(&dir_metadata, expected_uid)?;

    let file_name = format!("{handoff_id}.agent.json");
    let file_name_c = CString::new(file_name.as_bytes())
        .map_err(|_| "handoff filename contains a NUL byte".to_string())?;
    // Resolve the child relative to the already validated directory descriptor.
    // A pathname rename or symlink substitution cannot redirect this open into a
    // replacement directory after the directory metadata check.
    let file_fd = unsafe {
        libc::openat(
            directory.as_raw_fd(),
            file_name_c.as_ptr(),
            libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if file_fd < 0 {
        return Err(format!(
            "cannot securely open handoff: {}",
            std::io::Error::last_os_error()
        ));
    }
    let mut file = unsafe { std::fs::File::from_raw_fd(file_fd) };
    let opened_metadata = file
        .metadata()
        .map_err(|error| format!("cannot inspect open handoff: {error}"))?;
    validate_agent_snapshot_handoff_metadata(&opened_metadata, expected_uid, SystemTime::now())?;

    let mut bytes = Vec::with_capacity(opened_metadata.len() as usize);
    file.by_ref()
        .take(MAX_SNAPSHOT_JSON_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("cannot read handoff: {error}"))?;
    if bytes.len() > MAX_SNAPSHOT_JSON_BYTES {
        return Err("handoff exceeds the agent JSON snapshot size limit".to_string());
    }
    let read_metadata = file
        .metadata()
        .map_err(|error| format!("cannot re-inspect open handoff after read: {error}"))?;
    if read_metadata.dev() != opened_metadata.dev()
        || read_metadata.ino() != opened_metadata.ino()
        || read_metadata.len() != opened_metadata.len()
        || read_metadata.uid() != opened_metadata.uid()
        || read_metadata.mode() != opened_metadata.mode()
        || read_metadata.nlink() != opened_metadata.nlink()
        || read_metadata.mtime() != opened_metadata.mtime()
        || read_metadata.mtime_nsec() != opened_metadata.mtime_nsec()
    {
        return Err("handoff changed while it was being read".to_string());
    }

    validate_agent_snapshot_handoff_shape(&bytes)?;
    let snapshot = decode_snapshot_from_bytes(&bytes)
        .map_err(|error| format!("invalid agent snapshot handoff: {error}"))?;
    if snapshot.definition.runtime.as_deref() != Some("hermes") {
        return Err("agent snapshot handoff runtime must be hermes".to_string());
    }
    if snapshot.memory.level != MemoryLevel::None || !snapshot.memory.entries.is_empty() {
        return Err("agent snapshot handoff must be config-only".to_string());
    }

    let Some(expected) = consume_expected else {
        return Ok(bytes);
    };
    if bytes != expected {
        return Err("handoff contents changed before preview acknowledgement".to_string());
    }

    let mut current_metadata = std::mem::MaybeUninit::<libc::stat>::uninit();
    let stat_result = unsafe {
        libc::fstatat(
            directory.as_raw_fd(),
            file_name_c.as_ptr(),
            current_metadata.as_mut_ptr(),
            libc::AT_SYMLINK_NOFOLLOW,
        )
    };
    if stat_result != 0 {
        return Err(format!(
            "cannot re-inspect handoff before deletion: {}",
            std::io::Error::last_os_error()
        ));
    }
    let current_metadata = unsafe { current_metadata.assume_init() };
    if current_metadata.st_dev != opened_metadata.dev() as libc::dev_t
        || current_metadata.st_ino != opened_metadata.ino() as libc::ino_t
    {
        return Err("handoff changed while it was being read".to_string());
    }
    let unlink_result = unsafe { libc::unlinkat(directory.as_raw_fd(), file_name_c.as_ptr(), 0) };
    if unlink_result != 0 {
        return Err(format!(
            "cannot delete accepted handoff: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(bytes)
}

#[cfg(target_os = "macos")]
pub(super) fn read_agent_snapshot_handoff_from_dir(
    dir: &Path,
    handoff_id: &str,
) -> Result<Vec<u8>, String> {
    load_agent_snapshot_handoff_from_dir(dir, handoff_id, None)
}

#[cfg(target_os = "macos")]
pub(super) fn consume_agent_snapshot_handoff_from_dir(
    dir: &Path,
    handoff_id: &str,
    expected: &[u8],
) -> Result<(), String> {
    load_agent_snapshot_handoff_from_dir(dir, handoff_id, Some(expected)).map(|_| ())
}

#[cfg(not(target_os = "macos"))]
pub(super) fn read_agent_snapshot_handoff_from_dir(
    _dir: &Path,
    _handoff_id: &str,
) -> Result<Vec<u8>, String> {
    Err("agent snapshot handoffs currently require macOS file security".to_string())
}

#[cfg(not(target_os = "macos"))]
pub(super) fn consume_agent_snapshot_handoff_from_dir(
    _dir: &Path,
    _handoff_id: &str,
    _expected: &[u8],
) -> Result<(), String> {
    Err("agent snapshot handoffs currently require macOS file security".to_string())
}
