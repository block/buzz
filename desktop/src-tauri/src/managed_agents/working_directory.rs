use std::path::{Path, PathBuf};

use super::{default_agent_workdir, BackendKind, ManagedAgentRecord};

/// Normalize a user-selected local working directory for persistence.
///
/// Blank input clears the override. Non-blank input supports `~` and `~/...`,
/// must resolve to an existing absolute directory, and is canonicalized so the
/// process and restart snapshot use one stable spelling.
pub fn validate_working_directory(input: Option<&str>) -> Result<Option<String>, String> {
    let Some(input) = input.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    let expanded = expand_home(input)?;
    if !expanded.is_absolute() {
        return Err("working directory must be an absolute path".to_string());
    }
    let canonical = expanded.canonicalize().map_err(|_| {
        "The selected working folder is unavailable or cannot be accessed.".to_string()
    })?;
    if !canonical.is_dir() {
        return Err("The selected working folder is not a directory.".to_string());
    }
    if canonical.parent().is_none() {
        return Err("working directory cannot be a filesystem root".to_string());
    }
    let canonical = canonical
        .into_os_string()
        .into_string()
        .map_err(|_| "working directory must be valid UTF-8".to_string())?;
    Ok(Some(canonical))
}

fn expand_home(input: &str) -> Result<PathBuf, String> {
    if input == "~" {
        return dirs::home_dir()
            .ok_or_else(|| "cannot expand `~`: home directory is unavailable".to_string());
    }
    if let Some(rest) = input
        .strip_prefix("~/")
        .or_else(|| input.strip_prefix("~\\"))
    {
        return dirs::home_dir()
            .map(|home| home.join(rest))
            .ok_or_else(|| "cannot expand `~`: home directory is unavailable".to_string());
    }
    if input.starts_with('~') {
        return Err("working directory only supports `~` or `~/...` home expansion".to_string());
    }
    Ok(PathBuf::from(input))
}

/// Resolve and revalidate the effective CWD immediately before spawn.
///
/// Stored overrides are already canonicalized on write, but a directory may
/// have been removed or changed since then. The compatibility fallback retains
/// the existing Buzz-nest/home behavior for records with no override.
pub fn effective_agent_workdir(record: &ManagedAgentRecord) -> Result<Option<PathBuf>, String> {
    if record.backend != BackendKind::Local {
        return Ok(None);
    }
    match record.working_directory.as_deref() {
        Some(path) => validate_working_directory(Some(path)).map(|path| path.map(PathBuf::from)),
        None => Ok(default_agent_workdir()),
    }
}

pub fn validate_backend_working_directory(
    backend: &BackendKind,
    input: Option<&str>,
) -> Result<Option<String>, String> {
    let working_directory = validate_working_directory(input)?;
    if *backend != BackendKind::Local && working_directory.is_some() {
        return Err("working directory is only supported for local agents".to_string());
    }
    Ok(working_directory)
}

pub(crate) fn path_for_snapshot(path: Option<&Path>) -> Option<String> {
    path.map(|value| value.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blank_clears_the_override() {
        assert_eq!(validate_working_directory(None).unwrap(), None);
        assert_eq!(validate_working_directory(Some("  ")).unwrap(), None);
    }

    #[test]
    fn relative_paths_are_rejected() {
        let error = validate_working_directory(Some("relative/path")).unwrap_err();
        assert!(error.contains("absolute"), "{error}");
    }

    #[test]
    fn filesystem_root_is_rejected() {
        let current = std::env::current_dir().unwrap();
        let root = current.ancestors().last().unwrap();
        let error = validate_working_directory(Some(root.to_string_lossy().as_ref())).unwrap_err();
        assert!(error.contains("filesystem root"), "{error}");
    }

    #[test]
    fn existing_directory_is_canonicalized() {
        let temp = tempfile::tempdir().unwrap();
        let nested = temp.path().join("nested");
        std::fs::create_dir(&nested).unwrap();
        let input = nested.join("..").join("nested");
        assert_eq!(
            validate_working_directory(Some(input.to_str().unwrap())).unwrap(),
            Some(
                nested
                    .canonicalize()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned()
            )
        );
    }

    #[test]
    fn missing_directory_is_rejected() {
        let temp = tempfile::tempdir().unwrap();
        let error = validate_working_directory(Some(temp.path().join("missing").to_str().unwrap()))
            .unwrap_err();
        assert!(error.contains("unavailable"), "{error}");
        assert!(
            !error.contains(&temp.path().to_string_lossy().to_string()),
            "error leaked path: {error}"
        );
    }

    #[test]
    fn files_are_rejected() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("file");
        std::fs::write(&file, "x").unwrap();
        let error = validate_working_directory(Some(file.to_str().unwrap())).unwrap_err();
        assert!(error.contains("not a directory"), "{error}");
        assert!(
            !error.contains(&temp.path().to_string_lossy().to_string()),
            "error leaked path: {error}"
        );
    }

    #[test]
    fn provider_backends_reject_working_directories() {
        let temp = tempfile::tempdir().unwrap();
        let backend = BackendKind::Provider {
            id: "provider".into(),
            config: serde_json::Value::Null,
        };
        let error =
            validate_backend_working_directory(&backend, Some(temp.path().to_str().unwrap()))
                .unwrap_err();
        assert!(error.contains("only supported for local agents"), "{error}");
    }
}
