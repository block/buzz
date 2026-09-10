//! Machine-local admission control for commands executed inside ACP adapters.
//!
//! Adapters execute tools themselves, so `session/update` arrives too late to
//! gate a command. When the operator has installed the Buzz heavy-run lease,
//! prepend shims to the adapter's PATH before it starts.

use crate::acp::AcpError;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use tokio::process::Command;
use uuid::Uuid;

const SHIM_NAMES: &[&str] = &["npm", "pnpm", "bun", "npx", "node", "next", "playwright"];

pub(crate) struct HeavyRunGuard {
    directory: PathBuf,
}

impl HeavyRunGuard {
    pub(crate) fn install(
        cmd: &mut Command,
        agent_command: &str,
    ) -> Result<Option<Self>, AcpError> {
        let Some(home) = std::env::var_os("HOME") else {
            return Ok(None);
        };
        let lease = std::env::var_os("BUZZ_HEAVY_RUN_LEASE")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(&home).join(".buzz/scripts/heavy-run-lease.sh"));
        if !lease.is_file() {
            return Ok(None);
        }

        let original_path = std::env::var_os("PATH").unwrap_or_default();
        let root = PathBuf::from(home).join(".buzz/.scratch/acp-heavy-command-shims");
        fs::create_dir_all(&root)?;
        let guard_id = Uuid::new_v4();
        let directory = root.join(format!("{}-{guard_id}", std::process::id()));
        fs::create_dir(&directory)?;
        let script = shim_script();
        for name in SHIM_NAMES {
            let path = directory.join(name);
            fs::write(&path, script)?;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700))?;
        }

        let mut paths = vec![directory.clone()];
        paths.extend(std::env::split_paths(&original_path));
        let guarded_path = std::env::join_paths(paths).map_err(|error| {
            AcpError::Protocol(format!("invalid PATH for heavy-run guard: {error}"))
        })?;
        let nonce = std::env::var("BUZZ_MANAGED_AGENT_START_NONCE")
            .unwrap_or_else(|_| std::process::id().to_string());
        let identity = Path::new(agent_command)
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("agent");
        cmd.env("PATH", guarded_path)
            .env("BUZZ_HEAVY_RUN_LEASE", &lease)
            .env("BUZZ_HEAVY_RUN_ORIGINAL_PATH", original_path)
            .env(
                "BUZZ_HEAVY_RUN_LABEL",
                format!("{identity}-{nonce}-{guard_id}"),
            );
        Ok(Some(Self { directory }))
    }
}

impl Drop for HeavyRunGuard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.directory);
    }
}

fn shim_script() -> &'static str {
    r#"#!/bin/sh
set -eu
name=${0##*/}
original_path=${BUZZ_HEAVY_RUN_ORIGINAL_PATH:?}
lease=${BUZZ_HEAVY_RUN_LEASE:?}
label=${BUZZ_HEAVY_RUN_LABEL:?}
real=$(PATH=$original_path command -v "$name" || true)
if [ -z "$real" ]; then
  echo "buzz heavy-run guard: unwrapped $name not found" >&2
  exit 127
fi
heavy=0
case "$name" in
  npm|pnpm|bun)
    case " ${*:-} " in
      *" run build "*|*" test "*|*" run test:all "*|*" run test:full "*|*" run test:suite "*|*" run test:render "*|*" run "*browser*|*" run "*screenshot*|*" run "*capture*) heavy=1 ;;
    esac ;;
  npx)
    case " ${*:-} " in
      *" next build "*|*" playwright "*|*" browser"*|*" screenshot"*|*" capture"*) heavy=1 ;;
    esac ;;
  next) [ "${1:-}" = build ] && heavy=1 ;;
  node)
    for arg in "$@"; do [ "$arg" = --test ] && heavy=1; done ;;
  playwright) heavy=1 ;;
esac
if [ "$heavy" -eq 1 ]; then
  exec "$lease" "$label-$name" -- "$real" "$@"
fi
exec "$real" "$@"
"#
}

#[cfg(test)]
mod tests {
    use super::shim_script;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;
    use std::process::Command;
    use uuid::Uuid;

    fn executable(path: &Path, body: &str) {
        fs::write(path, body).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
    }

    #[test]
    fn classifier_contains_required_heavy_and_nonheavy_fallthrough_paths() {
        let script = shim_script();
        for required in [
            "run build",
            "run test:all",
            "run test:render",
            "next build",
            "--test",
            "playwright",
        ] {
            assert!(
                script.contains(required),
                "missing classification for {required}"
            );
        }
        assert!(
            script.contains("exec \"$real\" \"$@\""),
            "non-heavy commands must bypass the lease"
        );
    }

    #[test]
    fn scratch_heavy_command_blocks_then_admits_while_lint_bypasses() {
        let root = std::env::temp_dir().join(format!("buzz-heavy-guard-{}", Uuid::new_v4()));
        let shim_dir = root.join("shims");
        let real_dir = root.join("real");
        let scratch = root.join("disposable-scratch-checkout");
        fs::create_dir_all(&shim_dir).unwrap();
        fs::create_dir_all(&real_dir).unwrap();
        fs::create_dir_all(&scratch).unwrap();

        let shim = shim_dir.join("npm");
        let real = real_dir.join("npm");
        let lease = root.join("lease.sh");
        let held = root.join("held");
        let ran = root.join("ran");
        let leased = root.join("leased");
        executable(&shim, shim_script());
        executable(
            &real,
            &format!("#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\n", ran.display()),
        );
        executable(
            &lease,
            &format!(
                "#!/bin/sh\nprintf called >> '{}'\n[ -e '{}' ] && exit 75\nshift\n[ \"$1\" = -- ] && shift\nexec \"$@\"\n",
                leased.display(),
                held.display()
            ),
        );

        let invoke = |args: &[&str]| {
            Command::new(&shim)
                .args(args)
                .current_dir(&scratch)
                .env("BUZZ_HEAVY_RUN_ORIGINAL_PATH", &real_dir)
                .env("BUZZ_HEAVY_RUN_LEASE", &lease)
                .env("BUZZ_HEAVY_RUN_LABEL", "agent-session")
                .status()
                .unwrap()
        };

        fs::write(&held, "held").unwrap();
        assert_eq!(invoke(&["run", "test:render"]).code(), Some(75));
        assert!(!ran.exists(), "blocked command must not execute");

        fs::remove_file(&held).unwrap();
        assert!(invoke(&["run", "test:render"]).success());
        assert!(fs::read_to_string(&ran)
            .unwrap()
            .contains("run test:render"));

        let lease_calls = fs::read_to_string(&leased).unwrap();
        assert!(invoke(&["run", "lint"]).success());
        assert_eq!(fs::read_to_string(&leased).unwrap(), lease_calls);

        fs::remove_dir_all(root).unwrap();
    }
}
