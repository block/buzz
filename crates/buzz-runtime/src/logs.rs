//! Independent bounded stdout/stderr rotation and local tail helpers.

use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

pub const MAX_LOG_FILE_BYTES: u64 = 10 * 1024 * 1024;
pub const RETAINED_LOG_FILES: usize = 3;
pub const MAX_LOG_TAIL_BYTES: usize = 1024 * 1024;

const REDACTION_MARKER: &[u8] = b"[REDACTED]";

/// Streaming byte redaction for durable process output.
///
/// The writer retains at most `longest secret - 1` bytes between writes, so a
/// value split across pipe reads is removed before any part reaches disk. The
/// wrapped rotating writer therefore sees only redacted bytes, including at a
/// file boundary.
pub struct RedactingWriter<W> {
    inner: W,
    secrets: Vec<Vec<u8>>,
    pending: Vec<u8>,
    longest: usize,
}

impl<W: Write> RedactingWriter<W> {
    pub fn new(inner: W, mut secrets: Vec<Vec<u8>>) -> Self {
        secrets.retain(|secret| !secret.is_empty());
        secrets.sort_unstable_by(|left, right| {
            right.len().cmp(&left.len()).then_with(|| left.cmp(right))
        });
        secrets.dedup();
        let longest = secrets.iter().map(Vec::len).max().unwrap_or(0);
        Self {
            inner,
            secrets,
            pending: Vec::with_capacity(longest.saturating_sub(1)),
            longest,
        }
    }

    /// Writes the final retained suffix and returns the wrapped writer.
    pub fn finish(mut self) -> io::Result<W> {
        self.drain_pending(true)?;
        self.inner.flush()?;
        Ok(self.inner)
    }

    fn drain_pending(&mut self, finish: bool) -> io::Result<()> {
        if self.secrets.is_empty() {
            self.inner.write_all(&self.pending)?;
            self.pending.clear();
            return Ok(());
        }
        let safe_start_limit = if finish {
            self.pending.len()
        } else {
            self.pending
                .len()
                .saturating_sub(self.longest.saturating_sub(1))
        };
        let mut cursor = 0;
        while cursor < safe_start_limit {
            if let Some(secret) = self
                .secrets
                .iter()
                .find(|secret| self.pending[cursor..].starts_with(secret))
            {
                self.inner.write_all(REDACTION_MARKER)?;
                cursor += secret.len();
            } else {
                self.inner.write_all(&self.pending[cursor..cursor + 1])?;
                cursor += 1;
            }
        }
        self.pending.drain(..cursor);
        Ok(())
    }
}

impl<W: Write> Write for RedactingWriter<W> {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.pending.extend_from_slice(buffer);
        self.drain_pending(false)?;
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        // A flush is not an end-of-stream: keep the bounded suffix because the
        // next pipe read may complete a secret.
        self.drain_pending(false)?;
        self.inner.flush()
    }
}

pub struct RotatingLogWriter {
    base: PathBuf,
    file: Option<File>,
    length: u64,
}
impl std::fmt::Debug for RotatingLogWriter {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RotatingLogWriter")
            .field("base", &self.base)
            .field("length", &self.length)
            .finish()
    }
}
impl RotatingLogWriter {
    pub fn open(path: impl AsRef<Path>) -> io::Result<Self> {
        let base = path.as_ref().to_owned();
        if let Some(parent) = base.parent() {
            ensure_owner_dir(parent)?;
        }
        let file = open_append_owner_only(&base)?;
        let length = file.metadata()?.len();
        let mut writer = Self {
            base,
            file: Some(file),
            length,
        };
        if writer.length >= MAX_LOG_FILE_BYTES {
            writer.rotate()?;
        }
        Ok(writer)
    }
    fn rotate(&mut self) -> io::Result<()> {
        if let Some(file) = self.file.take() {
            file.sync_all()?;
        }
        match fs::remove_file(rotated_path(&self.base, RETAINED_LOG_FILES - 1)) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        for index in (1..RETAINED_LOG_FILES - 1).rev() {
            match fs::rename(
                rotated_path(&self.base, index),
                rotated_path(&self.base, index + 1),
            ) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
        }
        if self.base.exists() {
            fs::rename(&self.base, rotated_path(&self.base, 1))?;
        }
        self.file = Some(open_append_owner_only(&self.base)?);
        self.length = 0;
        Ok(())
    }
}
impl Write for RotatingLogWriter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        let mut offset = 0;
        while offset < buffer.len() {
            if self.length >= MAX_LOG_FILE_BYTES {
                self.rotate()?;
            }
            let available = (MAX_LOG_FILE_BYTES - self.length) as usize;
            let end = offset.saturating_add(available).min(buffer.len());
            self.file
                .as_mut()
                .ok_or_else(|| io::Error::new(io::ErrorKind::BrokenPipe, "log file unavailable"))?
                .write_all(&buffer[offset..end])?;
            self.length += (end - offset) as u64;
            offset = end;
        }
        Ok(buffer.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        self.file
            .as_mut()
            .ok_or_else(|| io::Error::new(io::ErrorKind::BrokenPipe, "log file unavailable"))?
            .flush()
    }
}

pub fn tail_rotating_log(
    path: impl AsRef<Path>,
    lines: u16,
    byte_limit: usize,
) -> io::Result<Vec<String>> {
    let line_limit = usize::from(lines).min(1_000);
    let byte_limit = byte_limit.min(MAX_LOG_TAIL_BYTES);
    if line_limit == 0 || byte_limit == 0 {
        return Ok(Vec::new());
    }
    let base = path.as_ref();
    let mut remaining = byte_limit;
    let mut chunks = Vec::new();
    for index in 0..RETAINED_LOG_FILES {
        let candidate = if index == 0 {
            base.to_owned()
        } else {
            rotated_path(base, index)
        };
        let mut file = match File::open(candidate) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        };
        let length = file.metadata()?.len();
        let take = remaining.min(length as usize);
        file.seek(SeekFrom::End(-(take as i64)))?;
        let mut bytes = vec![0_u8; take];
        file.read_exact(&mut bytes)?;
        chunks.push(bytes);
        remaining -= take;
        if remaining == 0 {
            break;
        }
    }
    chunks.reverse();
    let bytes = chunks.concat();
    let mut text = String::from_utf8_lossy(&bytes).into_owned();
    if text.len() > byte_limit {
        let mut start = text.len() - byte_limit;
        while !text.is_char_boundary(start) {
            start += 1
        }
        text = text[start..].to_owned();
    }
    let mut output: Vec<String> = text.lines().map(ToOwned::to_owned).collect();
    if output.len() > line_limit {
        output.drain(..output.len() - line_limit);
    }
    Ok(output)
}
fn rotated_path(base: &Path, index: usize) -> PathBuf {
    let mut name = base.as_os_str().to_owned();
    name.push(format!(".{index}"));
    PathBuf::from(name)
}
fn open_append_owner_only(path: &Path) -> io::Result<File> {
    if path.exists() {
        let metadata = fs::symlink_metadata(path)?;
        if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "log path is not a regular file",
            ));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if metadata.permissions().mode() & 0o077 != 0 {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "log file is not owner-only",
                ));
            }
        }
    }
    let mut options = OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options.open(path)?;
    #[cfg(windows)]
    crate::artifacts::harden_windows_acl(path, false)
        .map_err(|error| io::Error::new(io::ErrorKind::PermissionDenied, error.to_string()))?;
    Ok(file)
}

fn ensure_owner_dir(path: &Path) -> io::Result<()> {
    create_missing_owner_dirs(path)?;
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "log directory is not a real directory",
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "log directory is not owner-only",
            ));
        }
    }
    #[cfg(windows)]
    crate::artifacts::harden_windows_acl(path, true)
        .map_err(|error| io::Error::new(io::ErrorKind::PermissionDenied, error.to_string()))?;
    Ok(())
}
fn create_missing_owner_dirs(path: &Path) -> io::Result<()> {
    if path.exists() {
        let metadata = fs::symlink_metadata(path)?;
        if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "log ancestor is not a real directory",
            ));
        }
        return Ok(());
    }
    if let Some(parent) = path.parent().filter(|parent| *parent != path) {
        create_missing_owner_dirs(parent)?
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        fs::DirBuilder::new().mode(0o700).create(path)?;
    }
    #[cfg(not(unix))]
    {
        fs::create_dir(path)?;
        #[cfg(windows)]
        crate::artifacts::harden_windows_acl(path, true)
            .map_err(|error| io::Error::new(io::ErrorKind::PermissionDenied, error.to_string()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_binary_secret_split_across_writes_without_reordering_output() {
        let secret = b"\xffSECRET\0VALUE\xfe".to_vec();
        let mut writer = RedactingWriter::new(Vec::new(), vec![secret.clone()]);
        writer.write_all(b"before:").unwrap();
        writer.write_all(&secret[..5]).unwrap();
        writer.flush().unwrap();
        writer.write_all(&secret[5..]).unwrap();
        writer.write_all(b":after").unwrap();
        let output = writer.finish().unwrap();

        assert_eq!(output, b"before:[REDACTED]:after");
        assert!(!output
            .windows(secret.len())
            .any(|window| window == secret.as_slice()));
    }

    #[test]
    fn redacts_before_bytes_cross_a_rotation_boundary() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("logs").join("stdout.log");
        ensure_owner_dir(path.parent().unwrap()).unwrap();
        let file = open_append_owner_only(&path).unwrap();
        file.set_len(MAX_LOG_FILE_BYTES - 3).unwrap();
        drop(file);

        let secret = b"ROTATION_SECRET_SENTINEL".to_vec();
        let rotating = RotatingLogWriter::open(&path).unwrap();
        let mut writer = RedactingWriter::new(rotating, vec![secret.clone()]);
        writer.write_all(b"ok-").unwrap();
        writer.write_all(&secret[..9]).unwrap();
        writer.flush().unwrap();
        writer.write_all(&secret[9..]).unwrap();
        writer.write_all(b"-done").unwrap();
        writer.finish().unwrap();

        let rotated = fs::read(rotated_path(&path, 1)).unwrap();
        let current = fs::read(&path).unwrap();
        let durable_suffix: Vec<u8> = rotated
            .into_iter()
            .rev()
            .take(32)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .chain(current)
            .collect();
        assert!(durable_suffix
            .windows(b"ok-[REDACTED]-done".len())
            .any(|window| window == b"ok-[REDACTED]-done"));
        assert!(!durable_suffix
            .windows(secret.len())
            .any(|window| window == secret.as_slice()));
    }
}
