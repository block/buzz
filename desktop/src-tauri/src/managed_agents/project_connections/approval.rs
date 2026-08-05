use std::{fs, io::Read as _, path::Path};

use sha2::{Digest, Sha256};

pub(super) fn executable_sha256(path: &Path) -> Result<String, String> {
    let mut file =
        fs::File::open(path).map_err(|_| "Buzz could not read this executable.".to_string())?;
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|_| "Buzz could not read this executable.".to_string())?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(hex::encode(digest.finalize()))
}

pub(super) fn approved_execution_sha256(
    executable_fingerprint: &str,
    args: &[String],
) -> Result<String, String> {
    let mut digest = Sha256::new();
    digest.update(b"buzz-project-connection-v1\0");
    digest.update(executable_fingerprint.as_bytes());
    for arg in args {
        digest.update(b"\0arg\0");
        digest.update(arg.as_bytes());
        let candidate = arg.split_once('=').map_or(arg.as_str(), |(_, value)| value);
        let path = Path::new(candidate);
        if path.is_file() {
            digest.update(b"\0file\0");
            digest.update(
                executable_sha256(path)
                    .map_err(|_| "Buzz could not read a file used by this connection.".to_string())?
                    .as_bytes(),
            );
        }
    }
    Ok(hex::encode(digest.finalize()))
}

pub(super) fn canonical_connection_command(command: &str) -> Result<(String, String), String> {
    let path = Path::new(command.trim());
    if !path.is_absolute() {
        return Err("Enter the executable's absolute path.".to_string());
    }
    let canonical =
        fs::canonicalize(path).map_err(|_| "Buzz could not verify this executable.".to_string())?;
    let metadata = canonical
        .metadata()
        .map_err(|_| "Buzz could not verify this executable.".to_string())?;
    if !metadata.is_file() {
        return Err("The MCP server path is not an executable file.".to_string());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        if metadata.permissions().mode() & 0o111 == 0 {
            return Err("The MCP server file is not executable.".to_string());
        }
    }
    let canonical = canonical
        .to_str()
        .map(str::to_string)
        .ok_or_else(|| "The MCP server path is not valid Unicode.".to_string())?;
    let fingerprint = executable_sha256(Path::new(&canonical))?;
    Ok((canonical, fingerprint))
}
