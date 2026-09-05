use crate::managed_agents::types::{ManagedAgentRecord, RespondTo};

#[cfg(unix)]
const MARKED_CHILD_FIXTURE_ENV: &str = "BUZZ_TEST_MARKED_CHILD_FIXTURE";
#[cfg(unix)]
const MARKED_CHILD_READY_ENV: &str = "BUZZ_TEST_MARKED_CHILD_READY";

/// Test-executable child whose environment is stable and directly observable
/// through the production process-marker reader.
#[cfg(unix)]
pub(in crate::managed_agents) struct MarkedTestChild {
    child: Option<std::process::Child>,
    _ready_dir: tempfile::TempDir,
}

#[cfg(unix)]
impl MarkedTestChild {
    pub(in crate::managed_agents) fn spawn(instance_id: &str) -> Result<Self, String> {
        use std::os::unix::process::CommandExt as _;
        use std::process::{Command, Stdio};

        let ready_dir = tempfile::tempdir().map_err(|error| error.to_string())?;
        let ready_path = ready_dir.path().join("ready");
        let executable = std::env::current_exe().map_err(|error| error.to_string())?;
        let mut child = Command::new(executable)
            .args([
                "--exact",
                "managed_agents::runtime::test_fixtures::marked_child_process_fixture",
                "--nocapture",
            ])
            .env_clear()
            .env(MARKED_CHILD_FIXTURE_ENV, "1")
            .env(MARKED_CHILD_READY_ENV, &ready_path)
            .env("BUZZ_MANAGED_AGENT", instance_id)
            .process_group(0)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| error.to_string())?;

        for _ in 0..100 {
            if ready_path.is_file() {
                return Ok(Self {
                    child: Some(child),
                    _ready_dir: ready_dir,
                });
            }
            match child.try_wait() {
                Ok(Some(status)) => {
                    return Err(format!(
                        "marked child fixture exited before readiness: {status}"
                    ));
                }
                Ok(None) => {}
                Err(error) => {
                    let _ = super::terminate_process(child.id());
                    let _ = child.wait();
                    return Err(format!("failed to inspect marked child fixture: {error}"));
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let _ = super::terminate_process(child.id());
        let _ = child.wait();
        Err("marked child fixture did not become ready".into())
    }

    pub(in crate::managed_agents) fn id(&self) -> u32 {
        self.child.as_ref().expect("child is present").id()
    }

    pub(in crate::managed_agents) fn child_mut(&mut self) -> &mut std::process::Child {
        self.child.as_mut().expect("child is present")
    }

    pub(in crate::managed_agents) fn into_child(mut self) -> std::process::Child {
        self.child.take().expect("child is present")
    }
}

#[cfg(unix)]
impl Drop for MarkedTestChild {
    fn drop(&mut self) {
        if let Some(child) = self.child.as_mut() {
            let _ = super::terminate_process(child.id());
            let _ = child.wait();
        }
    }
}

/// Backstop for children whose owned `Child` handle is moved into production
/// runtime state. A failed assertion still terminates the complete process
/// group; successful tests explicitly wait through the owned handle.
#[cfg(unix)]
pub(in crate::managed_agents) struct MarkedProcessGuard(u32);

#[cfg(unix)]
impl MarkedProcessGuard {
    pub(in crate::managed_agents) fn new(pid: u32) -> Self {
        Self(pid)
    }
}

#[cfg(unix)]
impl Drop for MarkedProcessGuard {
    fn drop(&mut self) {
        let _ = super::terminate_process(self.0);
    }
}

#[cfg(unix)]
#[test]
fn marked_child_process_fixture() {
    if std::env::var_os(MARKED_CHILD_FIXTURE_ENV).is_none() {
        return;
    }
    let ready_path = std::env::var_os(MARKED_CHILD_READY_ENV)
        .expect("marked child fixture requires a readiness path");
    std::fs::write(ready_path, b"ready").expect("write marked child readiness handshake");
    loop {
        std::thread::park_timeout(std::time::Duration::from_secs(60));
    }
}

pub(super) const EXPECTED_ACCESS_ENV: &str = "BUZZ_TEST_EXPECTED_AGENT_ACCESS_OWNER_ONLY";

pub(super) fn expected_owner_only() -> bool {
    match std::env::var(EXPECTED_ACCESS_ENV) {
        Ok(value) => value
            .parse::<bool>()
            .unwrap_or_else(|_| panic!("{EXPECTED_ACCESS_ENV} must be true or false")),
        Err(std::env::VarError::NotPresent)
            if !crate::managed_agents::owner_only_access_build() =>
        {
            false
        }
        Err(std::env::VarError::NotPresent) => {
            panic!("{EXPECTED_ACCESS_ENV} must be set for owner-only-access-build tests")
        }
        Err(std::env::VarError::NotUnicode(_)) => {
            panic!("{EXPECTED_ACCESS_ENV} must be valid UTF-8")
        }
    }
}

pub(super) fn expected_mode(oss_mode: &'static str) -> &'static str {
    if expected_owner_only() {
        "owner-only"
    } else {
        oss_mode
    }
}

/// Construct a minimal record fixture for runtime tests.
pub(in crate::managed_agents) fn fixture(
    respond_to: RespondTo,
    allowlist: Vec<String>,
    auth_tag: Option<String>,
) -> ManagedAgentRecord {
    ManagedAgentRecord {
        description: None,
        pubkey: "p".into(),
        name: "n".into(),
        persona_id: None,
        private_key_nsec: "nsec1fake".into(),
        auth_tag,
        relay_url: "ws://localhost:3000".into(),
        avatar_url: None,
        acp_command: "buzz-acp".into(),
        agent_command: "goose".into(),
        agent_command_override: None,
        agent_args: vec![],
        mcp_command: String::new(),
        turn_timeout_seconds: 320,
        idle_timeout_seconds: None,
        max_turn_duration_seconds: None,
        parallelism: 1,
        system_prompt: None,
        model: None,
        provider: None,
        persona_source_version: None,
        env_vars: std::collections::BTreeMap::new(),
        start_on_app_launch: false,
        auto_restart_on_config_change: true,
        runtime_pid: None,
        backend: Default::default(),
        backend_agent_id: None,
        provider_policy_pending: false,
        provider_binary_path: None,
        team_id: None,
        persona_team_dir: None,
        persona_name_in_team: None,
        created_at: "now".into(),
        updated_at: "now".into(),
        last_started_at: None,
        last_stopped_at: None,
        last_exit_code: None,
        last_error: None,
        last_error_code: None,
        respond_to,
        respond_to_allowlist: allowlist,
        display_name: None,
        slug: None,
        runtime: None,
        name_pool: Vec::new(),
        is_builtin: false,
        is_active: true,
        shared: false,
        source_team: None,
        source_team_persona_slug: None,
        catalog_source: None,
        team_catalog_source: None,
        definition_respond_to: None,
        definition_respond_to_allowlist: Vec::new(),
        definition_parallelism: None,
        relay_mesh: None,
        effort_level: None,
    }
}
