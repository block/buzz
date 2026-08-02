use std::path::Path;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use serde::Deserialize;

use super::{resolve_command, KnownAcpRuntime};

const PROBE_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_CONTRACT_BYTES: usize = 16 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RuntimeCompatibility {
    Compatible,
    Incompatible,
    Unknown,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RuntimeContract {
    schema_version: u32,
    protocol: String,
    transport: String,
    execution: String,
}

fn compatibility_cache(
) -> &'static Mutex<std::collections::HashMap<&'static str, RuntimeCompatibility>> {
    static CACHE: OnceLock<Mutex<std::collections::HashMap<&'static str, RuntimeCompatibility>>> =
        OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(std::collections::HashMap::new()))
}

pub(super) fn clear_compatibility_cache() {
    compatibility_cache()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .clear();
}

pub(super) fn cached_runtime_compatibility(
    runtime: &KnownAcpRuntime,
) -> Option<RuntimeCompatibility> {
    if runtime.compatibility_probe_args.is_none() {
        return Some(RuntimeCompatibility::Compatible);
    }
    compatibility_cache()
        .lock()
        .ok()
        .and_then(|cache| cache.get(runtime.id).copied())
}

pub(super) fn probe_runtime_compatibility(
    runtime: &'static KnownAcpRuntime,
) -> RuntimeCompatibility {
    let Some(probe_args) = runtime.compatibility_probe_args else {
        return RuntimeCompatibility::Compatible;
    };
    let Some(binary_path) = resolve_command(probe_args[0]) else {
        return RuntimeCompatibility::Unknown;
    };
    probe_runtime_compatibility_at(runtime, &binary_path)
}

pub(super) fn probe_runtime_compatibility_at(
    runtime: &'static KnownAcpRuntime,
    binary_path: &Path,
) -> RuntimeCompatibility {
    let Some(probe_args) = runtime.compatibility_probe_args else {
        return RuntimeCompatibility::Compatible;
    };
    let outcome = probe_command(
        binary_path,
        probe_args,
        crate::managed_agents::readiness::cli_probe::augmented_path().as_deref(),
        PROBE_TIMEOUT,
    );
    if let Ok(mut cache) = compatibility_cache().lock() {
        cache.insert(runtime.id, outcome);
    }
    outcome
}

fn probe_command(
    binary_path: &Path,
    probe_args: &[&str],
    augmented_path: Option<&str>,
    timeout: Duration,
) -> RuntimeCompatibility {
    let Ok(output) = crate::managed_agents::readiness::cli_probe::run_bounded_probe(
        binary_path,
        probe_args,
        augmented_path,
        timeout,
        crate::managed_agents::readiness::cli_probe::ProbeOutputStream::Stdout,
        MAX_CONTRACT_BYTES,
    ) else {
        return RuntimeCompatibility::Unknown;
    };
    if output.truncated {
        return RuntimeCompatibility::Incompatible;
    }
    if output.status.success() {
        classify_contract(&output.bytes)
    } else if output.status.code().is_some() {
        RuntimeCompatibility::Incompatible
    } else {
        RuntimeCompatibility::Unknown
    }
}

fn classify_contract(stdout: &[u8]) -> RuntimeCompatibility {
    let Ok(contract) = serde_json::from_slice::<RuntimeContract>(stdout) else {
        return RuntimeCompatibility::Incompatible;
    };
    if contract.schema_version == 1
        && contract.protocol == "acp"
        && contract.transport == "stdio"
        && contract.execution == "embedded"
    {
        RuntimeCompatibility::Compatible
    } else {
        RuntimeCompatibility::Incompatible
    }
}

#[cfg(all(test, unix))]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::*;
    use crate::managed_agents::AcpAvailabilityStatus;

    #[test]
    fn compatibility_unknown_blocks_an_otherwise_available_runtime() {
        assert_eq!(
            super::super::availability_with_compatibility(
                AcpAvailabilityStatus::Available,
                RuntimeCompatibility::Unknown,
            ),
            AcpAvailabilityStatus::CompatibilityUnknown
        );
        assert_eq!(
            super::super::availability_with_compatibility(
                AcpAvailabilityStatus::Available,
                RuntimeCompatibility::Incompatible,
            ),
            AcpAvailabilityStatus::CliOutdated
        );
        assert_eq!(
            super::super::availability_with_compatibility(
                AcpAvailabilityStatus::Available,
                RuntimeCompatibility::Compatible,
            ),
            AcpAvailabilityStatus::Available
        );
    }

    #[test]
    fn compatibility_probe_requires_successful_runtime_contract() {
        let dir = tempfile::tempdir().expect("temp dir");
        let bin = dir.path().join("runtime");
        std::fs::write(
            &bin,
            r#"#!/bin/sh
if [ "$1" = acp ] && [ "$2" = info ]; then
  printf '%s\n' '{"schemaVersion":1,"protocol":"acp","transport":"stdio","execution":"embedded"}'
  exit 0
fi
exit 1
"#,
        )
        .expect("write probe runtime");
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755))
            .expect("chmod probe runtime");

        assert_eq!(
            probe_command(
                &bin,
                &["runtime", "acp", "info"],
                None,
                Duration::from_secs(1),
            ),
            RuntimeCompatibility::Compatible
        );
        assert_eq!(
            probe_command(
                &bin,
                &["runtime", "acp", "legacy"],
                None,
                Duration::from_secs(1),
            ),
            RuntimeCompatibility::Incompatible
        );
    }

    #[test]
    fn exit_zero_requires_the_exact_self_contained_contract() {
        assert_eq!(
            classify_contract(
                br#"{"schemaVersion":1,"protocol":"acp","transport":"stdio","execution":"embedded"}"#,
            ),
            RuntimeCompatibility::Compatible
        );
        assert_eq!(
            classify_contract(
                br#"{"schemaVersion":1,"protocol":"acp","transport":"stdio","execution":"gateway"}"#,
            ),
            RuntimeCompatibility::Incompatible
        );
        assert_eq!(
            classify_contract(br#"{"status":"ok"}"#),
            RuntimeCompatibility::Incompatible
        );
        assert_eq!(
            classify_contract(b"not json"),
            RuntimeCompatibility::Incompatible
        );
    }

    #[test]
    fn probe_bounds_output_without_waiting_for_stdout_owners() {
        let dir = tempfile::tempdir().expect("temp dir");
        let oversized = dir.path().join("oversized-runtime");
        std::fs::write(&oversized, "#!/bin/sh\nhead -c 20000 /dev/zero\n")
            .expect("write oversized runtime");
        std::fs::set_permissions(&oversized, std::fs::Permissions::from_mode(0o755))
            .expect("chmod oversized runtime");
        assert_eq!(
            probe_command(
                &oversized,
                &["runtime", "acp", "info"],
                None,
                Duration::from_secs(1),
            ),
            RuntimeCompatibility::Incompatible
        );

        let inherited = dir.path().join("inherited-runtime");
        std::fs::write(
            &inherited,
            r#"#!/bin/sh
(sleep 2) &
printf '%s\n' '{"schemaVersion":1,"protocol":"acp","transport":"stdio","execution":"embedded"}'
"#,
        )
        .expect("write inherited runtime");
        std::fs::set_permissions(&inherited, std::fs::Permissions::from_mode(0o755))
            .expect("chmod inherited runtime");
        assert_eq!(
            probe_command(
                &inherited,
                &["runtime", "acp", "info"],
                None,
                Duration::from_secs(1),
            ),
            RuntimeCompatibility::Compatible
        );
    }

    #[test]
    fn operational_probe_failures_are_unknown() {
        let dir = tempfile::tempdir().expect("temp dir");
        let missing = dir.path().join("missing-runtime");
        assert_eq!(
            probe_command(
                &missing,
                &["runtime", "acp", "info"],
                None,
                Duration::from_millis(10),
            ),
            RuntimeCompatibility::Unknown
        );

        let slow = dir.path().join("slow-runtime");
        std::fs::write(&slow, "#!/bin/sh\nwhile :; do :; done\n").expect("write slow runtime");
        std::fs::set_permissions(&slow, std::fs::Permissions::from_mode(0o755))
            .expect("chmod slow runtime");
        assert_eq!(
            probe_command(
                &slow,
                &["runtime", "acp", "info"],
                None,
                Duration::from_millis(10),
            ),
            RuntimeCompatibility::Unknown
        );

        let signaled = dir.path().join("signaled-runtime");
        std::fs::write(&signaled, "#!/bin/sh\nkill -TERM $$\n").expect("write signaled runtime");
        std::fs::set_permissions(&signaled, std::fs::Permissions::from_mode(0o755))
            .expect("chmod signaled runtime");
        assert_eq!(
            probe_command(
                &signaled,
                &["runtime", "acp", "info"],
                None,
                Duration::from_secs(1),
            ),
            RuntimeCompatibility::Unknown
        );
    }
}
