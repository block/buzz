use std::ffi::OsStr;
use std::path::{Component, Path, PathBuf};

pub(crate) fn safe_ssh_auth_sock_parent(socket: &Path, home: Option<&Path>) -> Option<String> {
    let parent = socket.parent()?;
    if !parent.is_absolute() {
        tracing::warn!(
            path = %parent.display(),
            "dropping SSH_AUTH_SOCK parent from Codex sandbox config: path is not absolute"
        );
        return None;
    }
    if !ssh_auth_sock_parent_is_narrow_enough(parent, home) {
        tracing::warn!(
            path = %parent.display(),
            "dropping SSH_AUTH_SOCK parent from Codex sandbox config: path is too broad"
        );
        return None;
    }

    let canonical_parent = match parent.canonicalize() {
        Ok(path) if path.is_dir() => path,
        _ => {
            tracing::warn!(
                path = %parent.display(),
                "dropping SSH_AUTH_SOCK parent from Codex sandbox config: parent directory is missing"
            );
            return None;
        }
    };
    if !ssh_auth_sock_parent_is_narrow_enough(&canonical_parent, home) {
        tracing::warn!(
            path = %canonical_parent.display(),
            "dropping SSH_AUTH_SOCK parent from Codex sandbox config: path is too broad"
        );
        return None;
    }
    if !ssh_auth_sock_parent_has_allowed_shape(&canonical_parent) {
        tracing::warn!(
            path = %parent.display(),
            canonical_path = %canonical_parent.display(),
            "dropping SSH_AUTH_SOCK parent from Codex sandbox config: path shape is not recognized"
        );
        return None;
    }
    if !ssh_auth_sock_parent_contains_only_socket(&canonical_parent, socket) {
        return None;
    }

    let normalized_parent = normalize_path_lexically(&canonical_parent);
    match normalized_parent.to_str() {
        Some(path) => Some(path.to_string()),
        None => {
            tracing::warn!(
                path = %normalized_parent.display(),
                "dropping SSH_AUTH_SOCK parent from Codex sandbox config: path is not valid UTF-8"
            );
            None
        }
    }
}

fn ssh_auth_sock_parent_is_narrow_enough(path: &Path, home: Option<&Path>) -> bool {
    !is_filesystem_root_or_home_ancestor(path, home)
        && !home.is_some_and(|home| paths_equal_lexically(path, &home.join(".ssh")))
}

fn ssh_auth_sock_parent_has_allowed_shape(path: &Path) -> bool {
    // Current allowlist covers OpenSSH /tmp/ssh-* and macOS launchd socket
    // directories. Linux XDG_RUNTIME_DIR shapes fail closed until modeled.
    let path = normalize_path_lexically(path);
    is_tmp_ssh_auth_sock_parent(&path) || is_macos_ssh_auth_sock_parent(&path)
}

fn is_tmp_ssh_auth_sock_parent(path: &Path) -> bool {
    // OpenSSH creates /tmp/ssh-* directories with mkdtemp, whose exclusive
    // create gives the user-owned directory a race-free name under /tmp. The
    // later one-entry socket check still rejects reused or populated dirs.
    path_has_file_name_prefix(path, "ssh-")
        && path
            .parent()
            .is_some_and(|parent| is_known_temp_root(parent))
}

fn is_macos_ssh_auth_sock_parent(path: &Path) -> bool {
    path_has_file_name_prefix(path, "com.apple.launchd.")
        && path.parent().is_some_and(|parent| {
            paths_equal_lexically(parent, Path::new("/var/run"))
                || paths_equal_lexically(parent, Path::new("/private/var/run"))
                || is_known_temp_root(parent)
        })
}

fn path_has_file_name_prefix(path: &Path, prefix: &str) -> bool {
    path.file_name()
        .and_then(OsStr::to_str)
        .is_some_and(|name| {
            name.strip_prefix(prefix)
                .is_some_and(|suffix| !suffix.is_empty())
        })
}

fn is_known_temp_root(path: &Path) -> bool {
    paths_equal_lexically(path, Path::new("/tmp"))
        || paths_equal_lexically(path, Path::new("/private/tmp"))
}

fn ssh_auth_sock_parent_contains_only_socket(parent: &Path, socket: &Path) -> bool {
    let Some(socket_file_name) = socket.file_name() else {
        tracing::warn!(
            path = %socket.display(),
            "dropping SSH_AUTH_SOCK parent from Codex sandbox config: socket file name is missing"
        );
        return false;
    };

    let entries = match std::fs::read_dir(parent) {
        Ok(entries) => entries,
        Err(error) => {
            tracing::warn!(
                path = %parent.display(),
                error = %error,
                "dropping SSH_AUTH_SOCK parent from Codex sandbox config: parent directory is not readable"
            );
            return false;
        }
    };

    let mut saw_socket = false;
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                tracing::warn!(
                    path = %parent.display(),
                    error = %error,
                    "dropping SSH_AUTH_SOCK parent from Codex sandbox config: parent directory entry is not readable"
                );
                return false;
            }
        };

        let entry_file_name = entry.file_name();
        if entry_file_name.as_os_str() != socket_file_name {
            tracing::warn!(
                path = %parent.display(),
                entry = %entry.path().display(),
                "dropping SSH_AUTH_SOCK parent from Codex sandbox config: parent directory contains another entry"
            );
            return false;
        }
        if !ssh_auth_sock_entry_is_socket(&entry) {
            return false;
        }
        saw_socket = true;
    }

    if !saw_socket {
        tracing::warn!(
            path = %socket.display(),
            "dropping SSH_AUTH_SOCK parent from Codex sandbox config: socket path is missing"
        );
    }
    saw_socket
}

fn ssh_auth_sock_entry_is_socket(entry: &std::fs::DirEntry) -> bool {
    match entry.file_type() {
        Ok(file_type) if ssh_auth_sock_file_type_matches(&file_type) => true,
        Ok(_) => {
            tracing::warn!(
                path = %entry.path().display(),
                expected = ssh_auth_sock_file_type_name(),
                "dropping SSH_AUTH_SOCK parent from Codex sandbox config: socket path has unexpected type"
            );
            false
        }
        Err(error) => {
            tracing::warn!(
                path = %entry.path().display(),
                error = %error,
                "dropping SSH_AUTH_SOCK parent from Codex sandbox config: socket metadata is not readable"
            );
            false
        }
    }
}

#[cfg(unix)]
fn ssh_auth_sock_file_type_matches(file_type: &std::fs::FileType) -> bool {
    use std::os::unix::fs::FileTypeExt;

    file_type.is_socket()
}

#[cfg(not(unix))]
fn ssh_auth_sock_file_type_matches(file_type: &std::fs::FileType) -> bool {
    file_type.is_file()
}

#[cfg(unix)]
fn ssh_auth_sock_file_type_name() -> &'static str {
    "Unix socket"
}

#[cfg(not(unix))]
fn ssh_auth_sock_file_type_name() -> &'static str {
    "file"
}

fn is_filesystem_root_or_home_ancestor(path: &Path, home: Option<&Path>) -> bool {
    let normalized = normalize_path_lexically(path);
    if normalized.has_root() && normalized.parent().is_none() {
        return true;
    }

    let Some(home) = home else {
        return false;
    };
    let normalized_home = normalize_path_lexically(home);
    normalized.has_root() && normalized_home.starts_with(&normalized)
}

fn paths_equal_lexically(a: &Path, b: &Path) -> bool {
    normalize_path_lexically(a) == normalize_path_lexically(b)
}

fn normalize_path_lexically(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}
