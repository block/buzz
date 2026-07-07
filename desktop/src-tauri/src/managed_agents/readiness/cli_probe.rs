use std::path::Path;

use crate::managed_agents::runtime::build_augmented_path;

pub(super) fn augmented_path() -> Option<String> {
    build_augmented_path(
        dirs::home_dir(),
        std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(std::path::Path::to_path_buf)),
        crate::managed_agents::login_shell_path(),
    )
}

pub(super) fn login_probe_succeeds(
    binary_path: &Path,
    probe_args: &[&str],
    augmented_path: Option<&str>,
) -> bool {
    // Run the probe at the resolved absolute path so the GUI-PATH gap is
    // bypassed. Inject the same augmented PATH used for launched agents so
    // script shims with `/usr/bin/env <interpreter>` shebangs can find runtimes
    // such as node/python when the app was launched with a bare GUI PATH.
    let mut command = std::process::Command::new(binary_path);
    command.args(&probe_args[1..]);
    if let Some(path) = augmented_path {
        command.env("PATH", path);
    }
    command
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    #[test]
    fn login_probe_uses_augmented_path_for_env_shebang_interpreter() {
        use std::fs;
        use std::os::unix::fs::PermissionsExt;
        use std::process::Command;

        let temp = tempfile::tempdir().expect("temp dir");
        let script_dir = temp.path().join("script-bin");
        let interpreter_dir = temp.path().join("interpreter-bin");
        let empty_path_dir = temp.path().join("empty-bin");
        fs::create_dir_all(&script_dir).expect("script dir");
        fs::create_dir_all(&interpreter_dir).expect("interpreter dir");
        fs::create_dir_all(&empty_path_dir).expect("empty path dir");

        let interpreter_path = interpreter_dir.join("node");
        let marker_path = temp.path().join("fake-node-ran");
        fs::write(
            &interpreter_path,
            format!(
                "#!/bin/sh\nprintf 'fake node ran\\n' > '{}' || exit 1\nexit 0\n",
                marker_path.display()
            ),
        )
        .expect("write interpreter");
        fs::set_permissions(&interpreter_path, fs::Permissions::from_mode(0o755))
            .expect("chmod interpreter");

        let script_path = script_dir.join("fake-codex");
        fs::write(&script_path, "#!/usr/bin/env node\n").expect("write script");
        fs::set_permissions(&script_path, fs::Permissions::from_mode(0o755)).expect("chmod script");

        let scrubbed_path = std::env::join_paths([empty_path_dir.as_path()])
            .expect("join scrubbed PATH")
            .to_string_lossy()
            .into_owned();
        let without_augmented_path = Command::new(&script_path)
            .args(["login", "status"])
            .env("PATH", &scrubbed_path)
            .output()
            .expect("run script with scrubbed PATH");
        assert!(
            !without_augmented_path.status.success(),
            "with a scrubbed PATH, /usr/bin/env should not find node"
        );

        let augmented_path =
            std::env::join_paths([interpreter_dir.as_path()]).expect("join augmented PATH");
        let augmented_path = augmented_path.to_string_lossy().into_owned();
        assert!(
            super::login_probe_succeeds(
                &script_path,
                &["fake-codex", "login", "status"],
                Some(&augmented_path),
            ),
            "the injected augmented PATH should allow /usr/bin/env to find the interpreter"
        );
        assert!(
            marker_path.exists(),
            "the fake node from the injected PATH should have run"
        );
    }
}
