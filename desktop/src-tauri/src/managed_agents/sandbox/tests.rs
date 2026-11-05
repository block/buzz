use super::*;

use std::ffi::OsStr;
use std::fs;

fn sandbox() -> AgentSandbox {
    AgentSandbox {
        nono: PathBuf::from("/Applications/Buzz.app/Contents/MacOS/nono"),
        profile: "buzz-agent-local".into(),
    }
}

fn args(command: &Command) -> Vec<String> {
    command
        .get_args()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect()
}

fn env_value<'a>(command: &'a Command, key: &str) -> Option<Option<&'a OsStr>> {
    command
        .get_envs()
        .find(|(name, _)| *name == OsStr::new(key))
        .map(|(_, value)| value)
}

#[test]
fn sandbox_follows_the_global_setting() {
    let mut global = GlobalAgentConfig::default();
    assert_eq!(sandbox_requested(&global), cfg!(unix));
    global.disable_agent_sandbox = true;
    assert!(!sandbox_requested(&global));
    assert_eq!(prepare_agent_sandbox(&global), Ok(None));
}

#[test]
fn unsandboxed_command_runs_the_harness_directly() {
    let acp = Path::new("/opt/buzz/buzz-acp");
    let command = agent_command(None, acp);
    assert_eq!(command.get_program(), acp.as_os_str());
    assert!(args(&command).is_empty());
    assert_eq!(env_value(&command, OUTER_SANDBOX_ENV), Some(None));
}

#[test]
fn sandboxed_command_wraps_the_harness_with_the_local_profile() {
    let sandbox = sandbox();
    let acp = Path::new("/Applications/Buzz.app/Contents/MacOS/buzz-acp");
    let grants = SandboxGrants {
        read: vec![PathBuf::from("/Users/u/.bun/bin")],
        allow: vec![PathBuf::from("/Volumes/code")],
        read_files: vec![PathBuf::from("/Users/u/Library/agents/logs/a.log")],
    };

    let mut command = agent_command(Some(&sandbox), acp);
    finish_sandboxed_command(&mut command, &sandbox, &grants, acp);

    assert_eq!(command.get_program(), sandbox.nono.as_os_str());
    assert_eq!(
        env_value(&command, OUTER_SANDBOX_ENV),
        Some(Some(OsStr::new("nono")))
    );
    assert_eq!(
        args(&command),
        [
            "wrap",
            "--silent",
            "--allow-cwd",
            "--profile",
            "buzz-agent-local",
            "--read",
            "/Users/u/.bun/bin",
            "--allow",
            "/Volumes/code",
            "--read-file",
            "/Users/u/Library/agents/logs/a.log",
            "--",
            "/Applications/Buzz.app/Contents/MacOS/buzz-acp",
        ]
    );
}

#[test]
fn sandboxed_command_strips_every_nono_env_var() {
    let _env = super::super::lock_env_mutex();
    // The shared lock serializes process-env mutation across tests.
    std::env::set_var("NONO_PROFILE", "permissive");
    let sandbox = sandbox();
    let acp = Path::new("/opt/buzz/buzz-acp");
    let mut command = agent_command(Some(&sandbox), acp);
    command.env("NONO_ALLOW", "/").env("nono_block_net", "0");
    command.env("BUZZ_RELAY_URL", "wss://relay.example");

    finish_sandboxed_command(&mut command, &sandbox, &SandboxGrants::default(), acp);
    std::env::remove_var("NONO_PROFILE");

    for key in ["NONO_PROFILE", "NONO_ALLOW", "nono_block_net"] {
        assert_eq!(
            env_value(&command, key),
            Some(None),
            "{key} must be removed"
        );
    }
    assert_eq!(
        env_value(&command, "BUZZ_RELAY_URL"),
        Some(Some(OsStr::new("wss://relay.example")))
    );
}

struct GrantFixture {
    _dir: tempfile::TempDir,
    root: PathBuf,
    home: PathBuf,
    nest: PathBuf,
}

fn grant_fixture() -> GrantFixture {
    let dir = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(dir.path()).unwrap();
    let home = root.join("home");
    let nest = home.join(".buzz");
    for path in [
        &nest,
        &home.join(".bun/bin"),
        &root.join("repo/target/debug"),
    ] {
        fs::create_dir_all(path).unwrap();
    }
    GrantFixture {
        _dir: dir,
        root,
        home,
        nest,
    }
}

fn grants_for(fixture: &GrantFixture, binaries: &[&Path], path_env: &str) -> SandboxGrants {
    spawn_grants(&GrantInputs {
        binaries,
        path_env: Some(OsStr::new(path_env)),
        home: Some(&fixture.home),
        nest: Some(&fixture.nest),
        demo_config_home: None,
        data_reads: &[],
        log_file: None,
    })
}

#[test]
fn grants_read_binary_dirs_and_user_path_entries() {
    let f = grant_fixture();
    let acp = f.root.join("repo/target/debug/buzz-acp");
    let path_env = format!(
        "{}:{}:/usr/bin:{}",
        f.home.join(".bun/bin").display(),
        f.home.join("missing/bin").display(),
        f.home.display()
    );

    let grants = grants_for(&f, &[&acp, Path::new("claude-agent-acp")], &path_env);

    assert_eq!(
        grants.read,
        [f.root.join("repo/target/debug"), f.home.join(".bun/bin")]
    );
    assert!(grants.allow.is_empty());
}

#[test]
fn grants_never_expose_home_or_its_ancestors() {
    let f = grant_fixture();
    let in_home = f.home.join("my-agent");
    let at_root = f.root.join("agent");

    let grants = grants_for(&f, &[&in_home, &at_root], &f.home.display().to_string());

    assert!(grants.read.is_empty(), "{:?}", grants.read);
}

#[cfg(unix)]
#[test]
fn grants_allow_a_repos_dir_outside_the_nest() {
    let f = grant_fixture();
    let repos = f.root.join("code");
    fs::create_dir_all(&repos).unwrap();
    std::os::unix::fs::symlink(&repos, f.nest.join("REPOS")).unwrap();

    assert_eq!(grants_for(&f, &[], "").allow, [repos]);
}

#[test]
fn grants_skip_a_repos_dir_inside_the_nest() {
    let f = grant_fixture();
    fs::create_dir_all(f.nest.join("REPOS")).unwrap();

    assert!(grants_for(&f, &[], "").allow.is_empty());
}

#[test]
fn grants_allow_the_demo_config_home() {
    let f = grant_fixture();
    let demo = f.root.join("config/buzz-demo-acme");
    fs::create_dir_all(&demo).unwrap();

    let grants = spawn_grants(&GrantInputs {
        binaries: &[],
        path_env: None,
        home: Some(&f.home),
        nest: Some(&f.nest),
        demo_config_home: Some(&demo),
        data_reads: &[],
        log_file: None,
    });

    assert_eq!(grants.allow, [demo]);
}

#[cfg(unix)]
fn fake_nono(dir: &Path, script: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = dir.join("nono");
    fs::write(&path, format!("#!/bin/sh\n{script}\n")).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    path
}

#[cfg(unix)]
#[test]
fn valid_profile_passes_validation() {
    let dir = tempfile::tempdir().unwrap();
    let nono = fake_nono(
        dir.path(),
        r#"[ "$1 $2 $3" = "profile validate buzz-agent-local" ]"#,
    );
    assert_eq!(validate_profile(&nono, "buzz-agent-local"), Ok(()));
}

#[cfg(unix)]
#[test]
fn invalid_profile_refuses_with_the_validator_errors() {
    let dir = tempfile::tempdir().unwrap();
    let nono = fake_nono(
        dir.path(),
        "printf '  \\033[31m[err]\\033[0m  JSON syntax error: line 3\\n  Result: invalid\\n'\nexit 1",
    );

    let error = validate_profile(&nono, "buzz-agent-local").unwrap_err();

    assert!(error.contains("`buzz-agent-local` is invalid"), "{error}");
    assert!(
        error.contains("[err]  JSON syntax error: line 3"),
        "{error}"
    );
    assert!(error.contains("buzz-agent-local.json"), "{error}");
}

#[test]
fn grants_read_only_buzz_data_dirs() {
    let f = grant_fixture();
    let templates = f.root.join("app-data/xyz.block.buzz.app/templates");
    let node_tools = f.root.join("app-data/Buzz/node-tools");
    fs::create_dir_all(&templates).unwrap();
    fs::create_dir_all(&node_tools).unwrap();
    let missing = f.root.join("app-data/Buzz/runtimes/node");

    let grants = spawn_grants(&GrantInputs {
        binaries: &[],
        path_env: None,
        home: Some(&f.home),
        nest: Some(&f.nest),
        demo_config_home: None,
        data_reads: &[templates.clone(), node_tools.clone(), missing],
        log_file: None,
    });

    assert_eq!(grants.read, [templates, node_tools]);
    assert!(grants.allow.is_empty());
}

/// Buzz's npm tools are `bin/` links into `lib/node_modules`; granting only
/// the binary's directory left `claude-agent-acp` unable to load its code.
#[test]
fn buzz_data_reads_cover_managed_node_tools() {
    let reads = buzz_data_reads();
    for dir in [
        crate::managed_agents::buzz_managed_npm_prefix(),
        crate::managed_agents::buzz_managed_node_root(),
        cli_templates_dir(),
    ] {
        let dir = dir.expect("data dir");
        assert!(reads.contains(&dir), "{} missing", dir.display());
    }
}

/// Node aborts at startup when it cannot `fstat` its inherited stderr, the
/// agent log in the desktop's app-data directory.
#[test]
fn grants_read_only_the_agent_log() {
    let f = grant_fixture();
    let logs = f.root.join("app-data/agents/logs");
    fs::create_dir_all(&logs).unwrap();
    let log = logs.join("agent.log");
    fs::write(&log, "").unwrap();

    let grants = spawn_grants(&GrantInputs {
        binaries: &[],
        path_env: None,
        home: Some(&f.home),
        nest: Some(&f.nest),
        demo_config_home: None,
        data_reads: &[],
        log_file: Some(&log),
    });

    assert_eq!(grants.read_files, [log]);
    assert!(grants.read.is_empty() && grants.allow.is_empty());
}
