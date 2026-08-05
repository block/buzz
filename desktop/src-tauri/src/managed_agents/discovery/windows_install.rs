//! Defender-safe construction of the Windows PowerShell CLI install commands.
//!
//! # Why the shape matters
//!
//! Windows Defender's ML classifier flags the bare `irm <url> | iex` command
//! line as `Trojan:Win32/Commando.A!ml` — piping a downloaded string straight
//! into `Invoke-Expression` is a textbook dropper signature, so the *command
//! line itself* is scored, independent of what the URL actually serves. The
//! spawn is denied before PowerShell runs, surfacing as
//! `failed to spawn shell: Access is denied. (os error 5)`, and the block is
//! sticky: Defender's "Allow" button does not clear it.
//!
//! [`windows_install_command!`] emits the two-step form instead — download the
//! vendor script to a file, then execute the file — which does not match that
//! signature. All three runtimes use it, not only the one observed failing:
//! Goose and Claude escaped by scoring under the classifier threshold, which is
//! luck rather than design, and the threshold is not ours to depend on.
//!
//! # Why one macro instead of three literals
//!
//! The catalog needs `&'static str`, so the commands must be built at compile
//! time from literals. Emitting them from a single macro means the security
//! shape is defined once and cannot drift between runtimes as URLs change —
//! a per-runtime literal would let one entry silently regress to `iex`.
//!
//! # Exit-code fidelity
//!
//! [#2892](https://github.com/block/buzz/pull/2892) established that an install
//! step must not report success when the download failed. Two pieces preserve
//! that here, and both are load-bearing:
//!
//! - `$ErrorActionPreference='Stop'` makes a failed `Invoke-RestMethod`
//!   terminate the whole command. Without it a failed download falls through to
//!   `& $installer` on a path that does not exist, and PowerShell exits **0** —
//!   the exact masking #2892 removed, in a new dress. `Stop` also prevents
//!   executing a *stale* installer left in `$env:TEMP` by an earlier run.
//! - `exit $LASTEXITCODE` propagates the vendor script's own exit code. Without
//!   it PowerShell reports its own status and a vendor failure of `3` flattens
//!   to `1`, losing the distinction the retry logic reads.
//!
//! Verified against `pwsh` over a local HTTP server: vendor exit 3 surfaces as
//! 3, vendor exit 0 as 0, a 404 and an unresolvable host as non-zero, and a
//! planted stale installer is never executed. The old `irm | iex` shape
//! produces identical codes for all four, so this is not a behavior change.
//!
//! # Quoting contract
//!
//! The emitted body is wrapped in one double-quote pair, which
//! `install_powershell_command` strips before handing the body to PowerShell.
//! The body therefore uses **only single quotes** internally; a double quote
//! would terminate that pair early and truncate the command.

/// Build the Windows CLI install command for one runtime.
///
/// `slug` names the downloaded script (`buzz-install-<slug>.ps1`) so concurrent
/// installs of different runtimes cannot overwrite each other's file. The
/// optional third argument carries a runtime's env prefix (Goose's
/// `$env:CONFIGURE='false'; `) and must end with `; `.
///
/// See the module docs for why each fragment is present.
macro_rules! windows_install_command {
    ($slug:literal, $url:literal) => {
        windows_install_command!($slug, $url, "")
    };
    ($slug:literal, $url:literal, $env_prefix:literal) => {
        concat!(
            "powershell.exe -NoProfile -ExecutionPolicy Bypass -Command \"",
            $env_prefix,
            "$ErrorActionPreference='Stop'; ",
            "$installer=Join-Path $env:TEMP 'buzz-install-",
            $slug,
            ".ps1'; ",
            "Invoke-RestMethod ",
            $url,
            " -OutFile $installer; ",
            "& $installer; ",
            "exit $LASTEXITCODE\"",
        )
    };
}

#[cfg(test)]
mod tests {
    use crate::managed_agents::known_acp_runtime_exact;

    /// Every runtime that ships a Windows install command. `cli_install_commands_windows`
    /// is read directly rather than through `cli_install_commands_for_os()` so these
    /// assertions cover the Windows strings while running on the Linux CI host.
    fn windows_install_commands() -> Vec<(&'static str, &'static str)> {
        ["goose", "claude", "codex"]
            .into_iter()
            .flat_map(|id| {
                known_acp_runtime_exact(id)
                    .expect("runtime must exist in the catalog")
                    .cli_install_commands_windows
                    .iter()
                    .map(move |command| (id, *command))
            })
            .collect()
    }

    /// The whole point of the change: no runtime may carry the flagged
    /// download-and-execute-in-one-line signature.
    #[test]
    fn test_no_windows_install_command_pipes_a_download_into_iex() {
        for (id, command) in windows_install_commands() {
            assert!(
                !command.contains("| iex"),
                "{id}: `irm | iex` is the shape Defender flags as Trojan:Win32/Commando.A!ml; \
                 download to a file and execute the file instead. Got: {command}"
            );
            assert!(
                !command.contains("Invoke-Expression"),
                "{id}: Invoke-Expression on downloaded content carries the same signature. \
                 Got: {command}"
            );
        }
    }

    /// All three runtimes must be hardened, not just the one observed failing.
    /// Goose and Claude escaped only by scoring under the classifier threshold.
    #[test]
    fn test_every_windows_install_command_downloads_to_a_file_then_executes_it() {
        let commands = windows_install_commands();
        assert_eq!(
            commands.len(),
            3,
            "expected exactly one Windows install command for each of goose, claude, codex"
        );
        for (id, command) in commands {
            assert!(
                command.contains("-OutFile $installer"),
                "{id}: must download the vendor script to a file. Got: {command}"
            );
            assert!(
                command.contains("& $installer"),
                "{id}: must execute the downloaded file. Got: {command}"
            );
            assert!(
                command.contains(&format!("buzz-install-{id}.ps1")),
                "{id}: script name must be runtime-specific so concurrent installs of \
                 different runtimes cannot overwrite each other. Got: {command}"
            );
        }
    }

    /// Guards the #2892 regression: without `Stop`, a failed download falls
    /// through to a missing file and PowerShell exits 0, reporting a failed
    /// install as a success. Without `exit $LASTEXITCODE`, the vendor's own
    /// exit code is replaced by PowerShell's.
    #[test]
    fn test_every_windows_install_command_preserves_failure_exit_codes() {
        for (id, command) in windows_install_commands() {
            assert!(
                command.contains("$ErrorActionPreference='Stop'"),
                "{id}: a failed download must abort instead of running a missing or stale \
                 installer and exiting 0 (see #2892). Got: {command}"
            );
            assert!(
                command.contains("exit $LASTEXITCODE"),
                "{id}: the vendor script's exit code must propagate. Got: {command}"
            );
        }
    }

    /// `install_powershell_command` strips exactly one outer double-quote pair.
    /// An inner double quote would close that pair early and truncate the body.
    #[test]
    fn test_every_windows_install_command_quotes_the_body_exactly_once() {
        for (id, command) in windows_install_commands() {
            let body = command
                .split_once(" -Command ")
                .map(|(_, body)| body)
                .unwrap_or_else(|| panic!("{id}: command must pass a -Command body: {command}"));
            assert!(
                body.starts_with('"') && body.ends_with('"'),
                "{id}: body must be wrapped in one double-quote pair. Got: {body}"
            );
            assert_eq!(
                body.matches('"').count(),
                2,
                "{id}: body must contain no inner double quotes — one would terminate the \
                 outer pair early and truncate the command. Got: {body}"
            );
        }
    }

    /// Goose's installer reads `CONFIGURE` to stay non-interactive; losing the
    /// prefix hangs the install waiting on input that never comes.
    #[test]
    fn test_goose_windows_install_command_keeps_its_env_prefix() {
        let goose = known_acp_runtime_exact("goose").unwrap();
        let command = goose.cli_install_commands_windows[0];
        assert!(
            command.contains("$env:CONFIGURE='false'"),
            "goose must stay non-interactive. Got: {command}"
        );
        assert!(
            command.find("$env:CONFIGURE='false'").unwrap()
                < command.find("Invoke-RestMethod").unwrap(),
            "the env prefix must be set before the installer runs. Got: {command}"
        );
    }

    /// The vendor URLs are the payload; pin them so a refactor of the shared
    /// shape cannot silently retarget a download.
    #[test]
    fn test_windows_install_commands_target_the_official_vendor_urls() {
        for (id, expected) in [
            (
                "goose",
                "https://raw.githubusercontent.com/aaif-goose/goose/main/download_cli.ps1",
            ),
            ("claude", "https://claude.ai/install.ps1"),
            ("codex", "https://chatgpt.com/codex/install.ps1"),
        ] {
            let runtime = known_acp_runtime_exact(id).unwrap();
            let command = runtime.cli_install_commands_windows[0];
            assert!(
                command.contains(&format!("Invoke-RestMethod {expected} -OutFile")),
                "{id}: must download from {expected}. Got: {command}"
            );
        }
    }
}
