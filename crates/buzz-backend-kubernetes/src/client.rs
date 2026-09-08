//! Cluster auth and client construction (spec §Cluster auth,
//! `docs/remote-agents.md:985-995`).
//!
//! Standard kubeconfig resolution (`$KUBECONFIG` → `~/.kube/config`).
//! `provider_config` carries `context` and `namespace` only — credentials
//! never transit config (I2, `:196-198`).

use kube::config::{ExecConfig, KubeConfigOptions, Kubeconfig};
use kube::{Client, Config};
use std::path::{Path, PathBuf};

/// Directories prepended to `PATH` before the client is built.
///
/// Kubeconfigs at Block near-universally authenticate through `exec`
/// credential plugins (`aws eks get-token`, `gke-gcloud-auth-plugin`) that
/// resolve via `PATH` — and this provider inherits a Finder-launched
/// desktop's minimal `PATH`, which contains none of the places those plugins
/// install to (`:989-994`).
const PATH_PREPEND: [&str; 2] = ["/opt/homebrew/bin", "/usr/local/bin"];

/// Compute the new `PATH` value: plugin directories first, inherited entries
/// after, in order. Pure so the ordering can be tested without mutating the
/// process's environment.
fn prepended_path(home: Option<&Path>, existing: &std::ffi::OsStr) -> Option<std::ffi::OsString> {
    let mut dirs: Vec<PathBuf> = PATH_PREPEND.iter().map(PathBuf::from).collect();
    if let Some(home) = home {
        dirs.push(home.join(".local/bin"));
    }
    // An empty inherited PATH splits into one empty entry, which POSIX
    // resolves as the current directory — a place a credential plugin should
    // never be looked up. Drop empties rather than propagate them.
    dirs.extend(std::env::split_paths(existing).filter(|p| !p.as_os_str().is_empty()));
    std::env::join_paths(dirs).ok()
}

/// Prepend the plugin directories to this process's `PATH`.
///
/// Modifies the provider's own environment, which is sound here: one process
/// per operation, called before any client or task exists, and the child
/// processes that read it are exactly the credential plugins this exists for.
fn prepend_plugin_path() {
    let home = std::env::var_os("HOME");
    let existing = std::env::var_os("PATH").unwrap_or_default();
    if let Some(joined) = prepended_path(home.as_ref().map(Path::new), &existing) {
        std::env::set_var("PATH", joined);
    }
}

/// Is `command` runnable — an executable on `PATH`, or an existing path?
fn resolves_on_path(command: &str) -> bool {
    if command.contains(std::path::MAIN_SEPARATOR) {
        return Path::new(command).is_file();
    }
    std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).any(|dir| dir.join(command).is_file()))
        .unwrap_or(false)
}

/// The exec plugin the selected context authenticates with, if any.
///
/// Read from the kubeconfig directly rather than from `Config`, which does not
/// expose it. A read failure yields `None`: this lookup exists only to improve
/// an error message, and must never be the thing that fails a deploy.
fn exec_plugin_for(context: Option<&str>) -> Option<ExecConfig> {
    let kubeconfig = Kubeconfig::read().ok()?;
    let context_name = context
        .map(str::to_string)
        .or_else(|| kubeconfig.current_context.clone())?;
    let user_name = kubeconfig
        .contexts
        .iter()
        .find(|c| c.name == context_name)
        .and_then(|c| c.context.as_ref())
        .and_then(|c| c.user.clone())?;
    kubeconfig
        .auth_infos
        .iter()
        .find(|a| a.name == user_name)
        .and_then(|a| a.auth_info.as_ref())
        .and_then(|a| a.exec.clone())
}

/// Turn a client-construction failure into an error a user can act on.
///
/// When the context authenticates through an exec plugin that is not on
/// `PATH`, that is almost always the cause, and the actionable fact is the
/// plugin's name — not a kube-rs error chain (`:994-995`).
fn explain(context: Option<&str>, error: &kube::Error) -> String {
    if let Some(command) = exec_plugin_for(context).and_then(|e| e.command) {
        if !resolves_on_path(&command) {
            return format!(
                "kubeconfig context {} authenticates with the credential plugin \
                 {command:?}, which is not on PATH. Install it or add its \
                 directory to PATH, then try again.",
                context.unwrap_or("(current)")
            );
        }
    }
    format!(
        "could not connect to the cluster using kubeconfig context {}: {error}",
        context.unwrap_or("(current)")
    )
}

/// Build a client for the selected context.
pub async fn connect(context: Option<&str>) -> Result<Client, String> {
    prepend_plugin_path();

    let options = KubeConfigOptions {
        context: context.map(str::to_string),
        ..Default::default()
    };
    let config = Config::from_kubeconfig(&options).await.map_err(|e| {
        // A named context that does not exist is a user typo, and the
        // kube-rs message for it is already specific.
        format!(
            "could not load kubeconfig for context {}: {e}",
            context.unwrap_or("(current)")
        )
    })?;

    Client::try_from(config).map_err(|e| explain(context, &e))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The three plugin directories must end up ahead of the inherited PATH,
    /// or a Finder-launched desktop never finds `aws`/`gke-gcloud-auth-plugin`.
    /// Tested on the pure computation: mutating the process PATH here would
    /// race every other test in the binary.
    #[test]
    fn plugin_directories_are_prepended_in_order() {
        let joined = prepended_path(
            Some(Path::new("/tmp/fake-home")),
            std::ffi::OsStr::new("/inherited/bin:/usr/bin"),
        )
        .unwrap();
        let dirs: Vec<PathBuf> = std::env::split_paths(&joined).collect();
        assert_eq!(
            dirs,
            [
                "/opt/homebrew/bin",
                "/usr/local/bin",
                "/tmp/fake-home/.local/bin",
                "/inherited/bin",
                "/usr/bin",
            ]
            .map(PathBuf::from)
        );
    }

    /// No `HOME` is not a failure — the two absolute directories still apply.
    #[test]
    fn missing_home_still_prepends_the_absolute_directories() {
        let joined = prepended_path(None, std::ffi::OsStr::new("/inherited/bin")).unwrap();
        let dirs: Vec<PathBuf> = std::env::split_paths(&joined).collect();
        assert_eq!(
            dirs,
            ["/opt/homebrew/bin", "/usr/local/bin", "/inherited/bin"].map(PathBuf::from)
        );
    }

    /// An empty inherited PATH must not produce an empty entry, which the
    /// shell and `resolves_on_path` would both read as the cwd.
    #[test]
    fn empty_inherited_path_yields_no_empty_entry() {
        let joined = prepended_path(None, std::ffi::OsStr::new("")).unwrap();
        let dirs: Vec<PathBuf> = std::env::split_paths(&joined).collect();
        assert_eq!(
            dirs,
            ["/opt/homebrew/bin", "/usr/local/bin"].map(PathBuf::from)
        );
    }

    #[test]
    fn resolves_absolute_paths_directly() {
        assert!(resolves_on_path("/bin/sh"));
        assert!(!resolves_on_path("/nonexistent/plugin-binary"));
    }
}
