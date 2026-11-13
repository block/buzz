use super::{
    agent_readiness, cli_login, make_cli_runtime, AgentReadiness, EffectiveAgentEnv, Requirement,
};
use crate::managed_agents::discovery::{known_acp_runtime, resolve_command};
use crate::managed_agents::AcpAvailabilityStatus;
use std::collections::BTreeMap;
use std::path::Path;

const FIXTURE_ENV: &str = "BUZZ_TEST_READINESS_AUTH_FIXTURE";

#[test]
fn codex_not_ready_copy_does_not_mention_openai_api_key() {
    // codex uses its own credential store via `codex login` (OAuth or API key).
    // The nudge copy must NOT say "set OPENAI_API_KEY".
    // Use a not-installed runtime so the requirement is always emitted
    // regardless of whether codex is on the test machine's PATH.
    let rt = make_cli_runtime(&["__buzz_nonexistent_adapter_xyz789__"], None);
    let reqs = cli_login::requirements(
        &["codex", "login", "status"],
        "run `codex login`",
        &rt,
        &BTreeMap::new(),
    );
    // Whether codex is installed or not, the copy (if any) must not mention OPENAI_API_KEY.
    for req in &reqs {
        if let Requirement::CliLogin { setup_copy, .. } = req {
            assert!(
                !setup_copy.contains("OPENAI_API_KEY"),
                "codex nudge copy must not mention OPENAI_API_KEY; got: {setup_copy:?}"
            );
            assert!(
                setup_copy.contains("codex login"),
                "codex nudge copy should mention `codex login`; got: {setup_copy:?}"
            );
        }
    }
}

#[test]
fn cli_login_readiness_uses_launch_environment() {
    if let Some(root) = std::env::var_os(FIXTURE_ENV) {
        check_readiness(Path::new(&root));
        return;
    }

    let fixture = tempfile::tempdir().expect("synthetic fixture directory");
    let root = fixture.path();
    let bin = root.join("bin");
    let effective_path = root.join("effective path");
    std::fs::create_dir(&bin).expect("fixture bin directory");
    std::fs::create_dir(&effective_path).expect("fixture effective PATH directory");
    for (account, status) in [
        ("default account", "logged-in"),
        ("signed in account", "logged-in"),
        ("signed out account", "logged-out"),
        ("invalid account", "invalid"),
    ] {
        let directory = root.join(account);
        std::fs::create_dir(&directory).expect("fixture account directory");
        std::fs::write(directory.join("status"), format!("{status}\n"))
            .expect("fixture account status");
    }
    for (command, selector, args) in [
        ("claude", "CLAUDE_CONFIG_DIR", "auth status"),
        ("claude-agent-acp", "CLAUDE_CONFIG_DIR", "auth status"),
        ("codex", "CODEX_HOME", "login status"),
        ("codex-acp", "CODEX_HOME", "login status"),
    ] {
        write_probe(&bin, command, selector, args, &effective_path);
    }

    let output = std::process::Command::new(std::env::current_exe().expect("test executable"))
        .args([
            "--exact",
            "managed_agents::readiness::tests::auth_env_tests::cli_login_readiness_uses_launch_environment",
            "--nocapture",
        ])
        .env(FIXTURE_ENV, root)
        .env("PATH", &bin)
        .env("HOME", root)
        .env("USERPROFILE", root)
        .env("APPDATA", root)
        .env("LOCALAPPDATA", root)
        .env("BUZZ_TEST_DEFAULT_ACCOUNT", root.join("default account"))
        .env_remove("CODEX_HOME")
        .env_remove("CLAUDE_CONFIG_DIR")
        .env_remove("BUZZ_TEST_AUTH_CONTEXT")
        .output()
        .expect("run confined readiness test process");
    assert!(
        output.status.success()
            && String::from_utf8_lossy(&output.stdout).contains("test result: ok. 1 passed;"),
        "confined readiness assertions failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn check_readiness(root: &Path) {
    for (adapter, selector) in [
        ("claude-agent-acp", "CLAUDE_CONFIG_DIR"),
        ("codex-acp", "CODEX_HOME"),
    ] {
        let runtime = known_acp_runtime(adapter).expect("known CLI-login runtime");
        for command in [adapter, runtime.underlying_cli.expect("underlying CLI")] {
            assert_eq!(
                resolve_command(command),
                Some(root.join("bin").join(probe_filename(command))),
                "readiness must resolve only the synthetic command"
            );
        }
        for (account, expected) in [
            (Some("signed in account"), "ready"),
            (Some("signed out account"), "login"),
            (Some("invalid account"), "config"),
            (Some("missing account"), "login"),
            (None, "ready"),
        ] {
            let mut env = BTreeMap::from([
                ("BUZZ_TEST_AUTH_CONTEXT".into(), "launch-equivalent".into()),
                (
                    "PATH".into(),
                    root.join("effective path").to_string_lossy().into_owned(),
                ),
            ]);
            if let Some(account) = account {
                env.insert(
                    selector.into(),
                    root.join(account).to_string_lossy().into_owned(),
                );
            }
            let effective = EffectiveAgentEnv {
                env,
                config_file_path: None,
                effective_command: adapter.into(),
            };
            let result = agent_readiness(&effective);
            let observed = match result {
                AgentReadiness::Ready => "ready",
                AgentReadiness::NotReady { requirements } => match requirements.as_slice() {
                    [Requirement::CliLogin {
                        availability: AcpAvailabilityStatus::Available,
                        ..
                    }] => "login",
                    [Requirement::CliConfigInvalid { .. }] => "config",
                    _ => "unexpected requirement",
                },
            };
            assert_eq!(
                observed, expected,
                "runtime {adapter}, synthetic account {account:?}"
            );
        }
    }
}

fn probe_filename(command: &str) -> String {
    if cfg!(windows) {
        format!("{command}.cmd")
    } else {
        command.into()
    }
}

fn write_probe(bin: &Path, command: &str, selector: &str, args: &str, path: &Path) {
    #[cfg(windows)]
    let script = format!(
        "@echo off\r\n\
         if \"%~1\"==\"--version\" (\r\n  echo codex-acp 1.99.0\r\n  exit /b 0\r\n)\r\n\
         if not \"%~1 %~2\"==\"{args}\" exit /b 87\r\n\
         if not \"%BUZZ_TEST_AUTH_CONTEXT%\"==\"launch-equivalent\" exit /b 88\r\n\
         if not \"%PATH%\"==\"{}\" exit /b 89\r\n\
         set \"account=%{selector}%\"\r\n\
         if not defined account set \"account=%BUZZ_TEST_DEFAULT_ACCOUNT%\"\r\n\
         if not exist \"%account%\\status\" exit /b 1\r\n\
         set /p status=<\"%account%\\status\"\r\n\
         if \"%status%\"==\"invalid\" (\r\n  echo Error loading configuration: synthetic unknown variant 1>&2\r\n  exit /b 1\r\n)\r\n\
         if \"%status%\"==\"logged-in\" (exit /b 0) else (exit /b 1)\r\n",
        path.display()
    );
    #[cfg(unix)]
    let script = format!(
        "#!/bin/sh\n\
         if [ \"$1\" = '--version' ]; then printf 'codex-acp 1.99.0\\n'; exit 0; fi\n\
         [ \"$1 $2\" = '{args}' ] || exit 87\n\
         [ \"$BUZZ_TEST_AUTH_CONTEXT\" = 'launch-equivalent' ] || exit 88\n\
         [ \"$PATH\" = '{}' ] || exit 89\n\
         account=\"${{{selector}:-$BUZZ_TEST_DEFAULT_ACCOUNT}}\"\n\
         [ -f \"$account/status\" ] || exit 1\n\
         IFS= read -r status < \"$account/status\" || exit 1\n\
         case \"$status\" in\n\
           logged-in) exit 0;;\n\
           invalid) printf 'Error loading configuration: synthetic unknown variant\\n' >&2; exit 1;;\n\
           *) exit 1;;\n\
         esac\n",
        path.display()
    );
    let probe = bin.join(probe_filename(command));
    std::fs::write(&probe, script).expect("write synthetic CLI probe");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(probe, std::fs::Permissions::from_mode(0o755))
            .expect("executable synthetic CLI probe");
    }
}
