use std::{
    collections::BTreeMap,
    ffi::{CStr, CString, OsStr, OsString},
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
};

#[cfg(not(unix))]
use std::fs::OpenOptions;

use super::manifest::{
    validate_relative_path, MAX_FILE_BYTES, MAX_PACKAGE_BYTES, MAX_PACKAGE_FILES,
};

pub(super) fn read_package_tree(
    trusted_root: &Path,
    package_root: &Path,
) -> Result<BTreeMap<String, Vec<u8>>, String> {
    #[cfg(unix)]
    {
        let directory = SecureDirectory::open_beneath(trusted_root, package_root)?;
        let mut files = BTreeMap::new();
        let mut budget = PackageScanBudget::new();
        scan_secure_directory(&directory, "", &mut files, &mut budget)?;
        Ok(files)
    }
    #[cfg(not(unix))]
    {
        let canonical_root = package_root
            .canonicalize()
            .map_err(|error| format!("resolve project canvas package: {error}"))?;
        if !canonical_root.starts_with(trusted_root) {
            return Err("project canvas package escaped its trusted root".to_string());
        }
        let mut files = BTreeMap::new();
        let mut budget = PackageScanBudget::new();
        scan_path_directory(&canonical_root, &canonical_root, &mut files, &mut budget)?;
        Ok(files)
    }
}

struct PackageScanBudget {
    remaining_entries: usize,
    remaining_bytes: usize,
}

impl PackageScanBudget {
    fn new() -> Self {
        Self {
            remaining_entries: MAX_PACKAGE_FILES,
            remaining_bytes: MAX_PACKAGE_BYTES,
        }
    }

    fn consume_entry(&mut self) -> Result<(), String> {
        self.remaining_entries = self
            .remaining_entries
            .checked_sub(1)
            .ok_or_else(package_entry_limit_error)?;
        Ok(())
    }

    fn consume_bytes(&mut self, bytes: usize) -> Result<(), String> {
        self.remaining_bytes = self
            .remaining_bytes
            .checked_sub(bytes)
            .ok_or_else(package_size_limit_error)?;
        Ok(())
    }
}

fn package_entry_limit_error() -> String {
    format!("project canvas package exceeds {MAX_PACKAGE_FILES} entries")
}

fn package_size_limit_error() -> String {
    "project canvas package exceeds 32 MiB".to_string()
}

#[cfg(unix)]
fn scan_secure_directory(
    directory: &SecureDirectory,
    prefix: &str,
    files: &mut BTreeMap<String, Vec<u8>>,
    budget: &mut PackageScanBudget,
) -> Result<(), String> {
    for name in directory.entry_names(budget.remaining_entries)? {
        budget.consume_entry()?;
        if name == OsStr::new(".DS_Store") {
            continue;
        }
        let name = name
            .into_string()
            .map_err(|_| "project canvas package paths must be UTF-8".to_string())?;
        let relative = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{prefix}/{name}")
        };
        let relative = validate_relative_path(&relative)?;
        if let Ok(child) = directory.open_subdirectory(OsStr::new(&name)) {
            scan_secure_directory(&child, &relative, files, budget)?;
            continue;
        }
        let cap = budget.remaining_bytes.min(MAX_FILE_BYTES);
        let bytes = directory
            .read_regular_file(OsStr::new(&name), cap)
            .map_err(|error| {
                if cap < MAX_FILE_BYTES && error.contains("exceeds its size limit") {
                    package_size_limit_error()
                } else {
                    error
                }
            })?;
        budget.consume_bytes(bytes.len())?;
        files.insert(relative, bytes);
    }
    Ok(())
}

#[cfg(not(unix))]
fn scan_path_directory(
    canonical_root: &Path,
    directory: &Path,
    files: &mut BTreeMap<String, Vec<u8>>,
    budget: &mut PackageScanBudget,
) -> Result<(), String> {
    for entry in fs::read_dir(directory)
        .map_err(|error| format!("read project canvas package directory: {error}"))?
    {
        let entry = entry.map_err(|error| format!("read project canvas package entry: {error}"))?;
        budget.consume_entry()?;
        if entry.file_name() == ".DS_Store" {
            continue;
        }
        let path = entry.path();
        let relative = path
            .strip_prefix(canonical_root)
            .map_err(|_| "project canvas file escaped its package".to_string())?;
        let metadata = fs::symlink_metadata(&path)
            .map_err(|error| format!("inspect project canvas package entry: {error}"))?;
        if metadata.file_type().is_symlink() {
            return Err("project canvas package cannot contain symlinks".to_string());
        }
        if metadata.is_dir() {
            scan_path_directory(canonical_root, &path, files, budget)?;
            continue;
        }
        if !metadata.is_file() {
            return Err("project canvas package contains an invalid entry".to_string());
        }
        let relative = validate_relative_path(
            &relative
                .to_str()
                .ok_or_else(|| "project canvas package paths must be UTF-8".to_string())?
                .replace(std::path::MAIN_SEPARATOR, "/"),
        )?;
        let cap = budget.remaining_bytes.min(MAX_FILE_BYTES);
        let file = OpenOptions::new()
            .read(true)
            .open(path)
            .map_err(|error| format!("open project canvas file: {error}"))?;
        let bytes = read_bounded_regular_file(file, cap).map_err(|error| {
            if cap < MAX_FILE_BYTES && error.contains("exceeds its size limit") {
                package_size_limit_error()
            } else {
                error
            }
        })?;
        budget.consume_bytes(bytes.len())?;
        files.insert(relative, bytes);
    }
    Ok(())
}

#[cfg(unix)]
struct SecureDirectory {
    file: File,
}

#[cfg(unix)]
impl SecureDirectory {
    fn open_beneath(trusted_root: &Path, target: &Path) -> Result<Self, String> {
        let relative = target
            .strip_prefix(trusted_root)
            .map_err(|_| "project canvas path escaped its trusted root".to_string())?;
        let mut directory = Self {
            file: open_directory_path(trusted_root)?,
        };
        for component in relative.components() {
            let std::path::Component::Normal(segment) = component else {
                return Err("invalid project canvas storage path".to_string());
            };
            directory = directory.open_subdirectory(segment)?;
        }
        Ok(directory)
    }

    fn open_subdirectory(&self, name: &OsStr) -> Result<Self, String> {
        Ok(Self {
            file: openat_file(
                &self.file,
                name,
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                "open project canvas directory",
            )?,
        })
    }

    fn read_regular_file(&self, name: &OsStr, cap: usize) -> Result<Vec<u8>, String> {
        let file = openat_file(
            &self.file,
            name,
            libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK,
            "open project canvas file",
        )?;
        read_bounded_regular_file(file, cap)
    }

    fn entry_names(&self, maximum: usize) -> Result<Vec<OsString>, String> {
        use std::os::fd::AsRawFd;

        let duplicate = unsafe { libc::fcntl(self.file.as_raw_fd(), libc::F_DUPFD_CLOEXEC, 0) };
        if duplicate < 0 {
            return Err(format!(
                "duplicate project canvas directory: {}",
                std::io::Error::last_os_error()
            ));
        }
        let stream = unsafe { libc::fdopendir(duplicate) };
        if stream.is_null() {
            unsafe { libc::close(duplicate) };
            return Err(format!(
                "open project canvas directory stream: {}",
                std::io::Error::last_os_error()
            ));
        }
        let stream = DirectoryStream(stream);
        let mut names = Vec::new();
        loop {
            clear_errno();
            let entry = unsafe { libc::readdir(stream.0) };
            if entry.is_null() {
                let error = current_errno();
                if error != 0 {
                    return Err(format!(
                        "read project canvas directory: {}",
                        std::io::Error::from_raw_os_error(error)
                    ));
                }
                break;
            }
            let name = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) };
            if name.to_bytes() == b"." || name.to_bytes() == b".." {
                continue;
            }
            if names.len() >= maximum {
                return Err(package_entry_limit_error());
            }
            use std::os::unix::ffi::OsStringExt;
            names.push(OsString::from_vec(name.to_bytes().to_vec()));
        }
        Ok(names)
    }
}

#[cfg(unix)]
struct DirectoryStream(*mut libc::DIR);

#[cfg(unix)]
impl Drop for DirectoryStream {
    fn drop(&mut self) {
        unsafe { libc::closedir(self.0) };
    }
}

#[cfg(unix)]
fn open_directory_path(path: &Path) -> Result<File, String> {
    use std::os::{fd::FromRawFd, unix::ffi::OsStrExt};

    let path = CString::new(path.as_os_str().as_bytes())
        .map_err(|_| "project canvas paths cannot contain NUL bytes".to_string())?;
    let descriptor = unsafe {
        libc::open(
            path.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if descriptor < 0 {
        return Err(format!(
            "open trusted project canvas directory: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(unsafe { File::from_raw_fd(descriptor) })
}

#[cfg(unix)]
fn openat_file(parent: &File, name: &OsStr, flags: i32, context: &str) -> Result<File, String> {
    use std::os::{
        fd::{AsRawFd, FromRawFd},
        unix::ffi::OsStrExt,
    };

    let name = CString::new(name.as_bytes())
        .map_err(|_| "project canvas paths cannot contain NUL bytes".to_string())?;
    let descriptor = unsafe { libc::openat(parent.as_raw_fd(), name.as_ptr(), flags) };
    if descriptor < 0 {
        return Err(format!("{context}: {}", std::io::Error::last_os_error()));
    }
    Ok(unsafe { File::from_raw_fd(descriptor) })
}

fn read_bounded_regular_file(file: File, cap: usize) -> Result<Vec<u8>, String> {
    let metadata = file
        .metadata()
        .map_err(|error| format!("inspect project canvas file: {error}"))?;
    if !metadata.is_file() {
        return Err("project canvas file is not a permitted regular file".to_string());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.nlink() != 1 {
            return Err("project canvas files cannot be hard linked".to_string());
        }
    }
    if metadata.len() > cap as u64 {
        return Err("project canvas file exceeds its size limit".to_string());
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(cap as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("read project canvas file: {error}"))?;
    if bytes.len() > cap {
        return Err("project canvas file exceeds its size limit".to_string());
    }
    Ok(bytes)
}

#[cfg(any(target_os = "macos", target_os = "ios", target_os = "freebsd"))]
fn clear_errno() {
    unsafe { *libc::__error() = 0 };
}

#[cfg(any(target_os = "macos", target_os = "ios", target_os = "freebsd"))]
fn current_errno() -> i32 {
    unsafe { *libc::__error() }
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn clear_errno() {
    unsafe { *libc::__errno_location() = 0 };
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn current_errno() -> i32 {
    unsafe { *libc::__errno_location() }
}

#[cfg(all(
    unix,
    not(any(
        target_os = "macos",
        target_os = "ios",
        target_os = "freebsd",
        target_os = "linux",
        target_os = "android"
    ))
))]
fn clear_errno() {}

#[cfg(all(
    unix,
    not(any(
        target_os = "macos",
        target_os = "ios",
        target_os = "freebsd",
        target_os = "linux",
        target_os = "android"
    ))
))]
fn current_errno() -> i32 {
    0
}

pub(super) fn canonical_canvas_root(root: &Path, create: bool) -> Result<Option<PathBuf>, String> {
    if !root.exists() {
        if !create {
            return Ok(None);
        }
        fs::create_dir_all(root).map_err(|error| format!("create project canvas root: {error}"))?;
    }
    ensure_no_symlink(root)?;
    let canonical = root
        .canonicalize()
        .map_err(|error| format!("resolve project canvas root: {error}"))?;
    if !canonical.is_dir() {
        return Err("project canvas root is not a directory".to_string());
    }
    Ok(Some(canonical))
}

pub(super) fn ensure_secure_descendant(
    trusted_root: &Path,
    target: &Path,
    create: bool,
) -> Result<(), String> {
    let relative = target
        .strip_prefix(trusted_root)
        .map_err(|_| "project canvas path escaped the canvas root".to_string())?;
    let mut current = trusted_root.to_path_buf();
    for component in relative.components() {
        let std::path::Component::Normal(segment) = component else {
            return Err("invalid project canvas storage path".to_string());
        };
        current.push(segment);
        match fs::symlink_metadata(&current) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() || !metadata.is_dir() {
                    return Err(format!(
                        "project canvas directory is not a real directory: {}",
                        current.display()
                    ));
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound && create => {
                fs::create_dir(&current)
                    .map_err(|error| format!("create project canvas directory: {error}"))?;
            }
            Err(error) => {
                return Err(format!("inspect project canvas directory: {error}"));
            }
        }
    }
    let canonical = target
        .canonicalize()
        .map_err(|error| format!("resolve project canvas directory: {error}"))?;
    if !canonical.starts_with(trusted_root) {
        return Err("project canvas directory escaped the canvas root".to_string());
    }
    Ok(())
}

pub(super) fn ensure_secure_file(trusted_root: &Path, path: &Path) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "project canvas file has no parent".to_string())?;
    ensure_secure_descendant(trusted_root, parent, false)?;
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("inspect project canvas file: {error}"))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(format!(
            "project canvas file is not a real file: {}",
            path.display()
        ));
    }
    let canonical = path
        .canonicalize()
        .map_err(|error| format!("resolve project canvas file: {error}"))?;
    if !canonical.starts_with(trusted_root) {
        return Err("project canvas file escaped the canvas root".to_string());
    }
    Ok(())
}

pub(super) fn read_file_with_cap(
    trusted_root: &Path,
    path: &Path,
    cap: usize,
) -> Result<Vec<u8>, String> {
    ensure_secure_file(trusted_root, path)?;
    #[cfg(unix)]
    {
        let parent = path
            .parent()
            .ok_or_else(|| "project canvas file has no parent".to_string())?;
        let directory = SecureDirectory::open_beneath(trusted_root, parent)?;
        let name = path
            .file_name()
            .ok_or_else(|| "project canvas file has no name".to_string())?;
        directory.read_regular_file(name, cap)
    }
    #[cfg(not(unix))]
    {
        let metadata =
            fs::metadata(path).map_err(|error| format!("inspect project canvas file: {error}"))?;
        if metadata.len() > cap as u64 {
            return Err("project canvas control file exceeds its size limit".to_string());
        }
        let mut options = OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NOFOLLOW);
        }
        let mut bytes = Vec::with_capacity(metadata.len() as usize);
        options
            .open(path)
            .map_err(|error| format!("open project canvas control file: {error}"))?
            .take(cap as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| format!("read project canvas control file: {error}"))?;
        if bytes.len() > cap {
            return Err("project canvas control file exceeds its size limit".to_string());
        }
        Ok(bytes)
    }
}

pub(super) fn ensure_no_symlink(path: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("inspect project canvas path: {error}"))?;
    if metadata.file_type().is_symlink() {
        return Err(format!(
            "project canvas paths cannot be symlinks: {}",
            path.display()
        ));
    }
    Ok(())
}

pub(super) fn make_snapshot_read_only(root: &Path) -> Result<(), String> {
    for entry in
        fs::read_dir(root).map_err(|error| format!("read project canvas snapshot: {error}"))?
    {
        let path = entry
            .map_err(|error| format!("read project canvas snapshot entry: {error}"))?
            .path();
        if path.is_dir() {
            make_snapshot_read_only(&path)?;
        } else {
            let mut permissions = fs::metadata(&path)
                .map_err(|error| format!("inspect project canvas snapshot: {error}"))?
                .permissions();
            permissions.set_readonly(true);
            fs::set_permissions(&path, permissions)
                .map_err(|error| format!("lock project canvas snapshot file: {error}"))?;
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(root, fs::Permissions::from_mode(0o555))
            .map_err(|error| format!("lock project canvas snapshot directory: {error}"))?;
    }
    Ok(())
}

pub(super) fn make_tree_writable(root: &Path) -> Result<(), String> {
    if !root.exists() {
        return Ok(());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(root, fs::Permissions::from_mode(0o755))
            .map_err(|error| format!("unlock project canvas staging directory: {error}"))?;
    }
    for entry in fs::read_dir(root)
        .map_err(|error| format!("read project canvas staging directory: {error}"))?
    {
        let path = entry
            .map_err(|error| format!("read project canvas staging entry: {error}"))?
            .path();
        if path.is_dir() {
            make_tree_writable(&path)?;
        } else {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&path, fs::Permissions::from_mode(0o644))
                    .map_err(|error| format!("unlock project canvas staging file: {error}"))?;
            }
            #[cfg(windows)]
            {
                let mut permissions = fs::metadata(&path)
                    .map_err(|error| format!("inspect project canvas staging file: {error}"))?
                    .permissions();
                permissions.set_readonly(false);
                fs::set_permissions(&path, permissions)
                    .map_err(|error| format!("unlock project canvas staging file: {error}"))?;
            }
        }
    }
    Ok(())
}
