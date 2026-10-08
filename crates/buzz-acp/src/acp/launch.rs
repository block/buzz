//! Host-controlled wrapping at the shared ACP subprocess boundary.
use super::AcpError;
use tokio::process::Command;

pub(super) const PREFIX_ENV: &str = "BUZZ_ACP_LAUNCH_PREFIX";

pub(super) fn command(worker: &str, args: &[String]) -> Result<Command, AcpError> {
    command_with_prefix(worker, args, std::env::var_os(PREFIX_ENV))
}

fn command_with_prefix(
    worker: &str,
    args: &[String],
    prefix: Option<std::ffi::OsString>,
) -> Result<Command, AcpError> {
    // Non-Unix cleanup only owns the immediate child, not a supervised worker.
    if prefix.is_some() && !cfg!(unix) {
        return Err(AcpError::Protocol(format!(
            "{PREFIX_ENV} is supported only on Unix; refusing to launch an uncontained worker"
        )));
    }
    let mut command = match prefix {
        None => Command::new(worker),
        Some(value) => {
            let invalid = || {
                AcpError::Protocol(format!(
                "{PREFIX_ENV} must be a nonempty JSON string array with an absolute executable path"
            ))
            };
            let prefix: Vec<String> =
                serde_json::from_str(&value.into_string().map_err(|_| invalid())?)
                    .map_err(|_| invalid())?;
            let executable = prefix
                .first()
                .filter(|path| std::path::Path::new(path).is_absolute())
                .ok_or_else(invalid)?;
            if prefix.iter().any(|arg| arg.contains('\0')) {
                return Err(invalid());
            }
            let mut command = Command::new(executable);
            command.args(&prefix[1..]).arg(worker);
            command
        }
    };
    command.args(args);
    Ok(command)
}

#[cfg(all(test, unix))]
mod tests;

#[cfg(test)]
mod platform_tests {
    use super::*;

    #[test]
    fn unset_prefix_keeps_direct_launch_on_every_platform() {
        let command = command_with_prefix("worker", &["arg".into()], None).unwrap();
        assert_eq!(command.as_std().get_program(), "worker");
        assert_eq!(command.as_std().get_args().collect::<Vec<_>>(), ["arg"]);
    }

    #[test]
    fn configured_prefix_requires_unix_process_containment() {
        // A real absolute executable prevents a bad path from hiding the guard.
        let executable = std::env::current_exe().unwrap();
        let prefix = serde_json::to_string(&[&executable]).unwrap();
        let result = command_with_prefix("worker", &[], Some(prefix.into()));
        if cfg!(unix) {
            assert_eq!(
                result.unwrap().as_std().get_program(),
                executable.as_os_str()
            );
        } else {
            assert!(result
                .unwrap_err()
                .to_string()
                .contains("supported only on Unix"));
            // Even an empty setting must refuse rather than downgrade to direct launch.
            assert!(command_with_prefix("worker", &[], Some("".into())).is_err());
        }
    }
}
