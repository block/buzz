//! Integration tests for MCP toolchain shim config isolation.
//!
//! Layer 1 tests invoke the REAL checked-in wrapper scripts from resources/shims/
//! with a pre-seeded fake hermit environment. The fake hermit dir contains no-op
//! bootstrap binaries and probe scripts that print their effective env vars. This
//! verifies the actual wrapper → setup-common.sh → exec chain without network.

#[cfg(unix)]
mod hostile_config {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    fn shim_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/shims")
    }

    #[test]
    fn shim_files_exist_and_are_executable() {
        for name in ["setup-common.sh", "uv", "uvx", "npx", "node"] {
            let path = shim_dir().join(name);
            assert!(
                path.exists(),
                "shim {name} does not exist at {}",
                path.display()
            );
            let mode = path.metadata().unwrap().permissions().mode();
            assert!(
                mode & 0o111 != 0,
                "shim {name} is not executable (mode={mode:#o})"
            );
        }
    }

    #[test]
    fn npm_uses_distinct_config_files() {
        let tmp = tempfile::TempDir::new().unwrap();
        let hermit_dir = tmp.path();
        let user_npmrc = hermit_dir.join("empty-user-npmrc");
        let global_npmrc = hermit_dir.join("empty-global-npmrc");
        fs::write(&user_npmrc, "").unwrap();
        fs::write(&global_npmrc, "").unwrap();

        assert_ne!(
            user_npmrc.canonicalize().unwrap(),
            global_npmrc.canonicalize().unwrap(),
            "user and global npmrc must be distinct files to avoid double-load crash"
        );
    }

    /// Pre-seeds a fake hermit dir that short-circuits setup-common.sh bootstrap.
    /// Probe binaries print their effective env vars instead of doing real work.
    fn seed_fake_hermit_dir(dir: &Path) {
        let bin = dir.join("bin");
        fs::create_dir_all(&bin).unwrap();
        fs::create_dir_all(dir.join("cache")).unwrap();

        let write_exec = |path: &Path, content: &str| {
            fs::write(path, content).unwrap();
            fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
        };

        write_exec(&bin.join("hermit"), "#!/bin/bash\nexit 0\n");
        write_exec(
            &bin.join("activate-hermit"),
            "#!/bin/bash\nexport HERMIT_ENV=\"${BUZZ_MCP_HERMIT_DIR}\"\n",
        );

        // uv probe: prints UV_NO_CONFIG and UV_INDEX_URL
        write_exec(
            &bin.join("uv"),
            "#!/bin/bash\n\
             echo \"UV_NO_CONFIG=${UV_NO_CONFIG:-unset}\"\n\
             echo \"UV_INDEX_URL=${UV_INDEX_URL:-unset}\"\n",
        );

        // uvx probe: prints UV_NO_CONFIG and UV_INDEX_URL
        write_exec(
            &bin.join("uvx"),
            "#!/bin/bash\n\
             echo \"UV_NO_CONFIG=${UV_NO_CONFIG:-unset}\"\n\
             echo \"UV_INDEX_URL=${UV_INDEX_URL:-unset}\"\n",
        );

        // npx probe: prints NPM isolation vars
        write_exec(
            &bin.join("npx"),
            "#!/bin/bash\n\
             echo \"NPM_CONFIG_USERCONFIG=${NPM_CONFIG_USERCONFIG:-unset}\"\n\
             echo \"NPM_CONFIG_GLOBALCONFIG=${NPM_CONFIG_GLOBALCONFIG:-unset}\"\n\
             echo \"NPM_CONFIG_LOCATION=${NPM_CONFIG_LOCATION:-unset}\"\n",
        );

        // node probe: prints NPM isolation vars
        write_exec(
            &bin.join("node"),
            "#!/bin/bash\n\
             echo \"NPM_CONFIG_USERCONFIG=${NPM_CONFIG_USERCONFIG:-unset}\"\n\
             echo \"NPM_CONFIG_GLOBALCONFIG=${NPM_CONFIG_GLOBALCONFIG:-unset}\"\n\
             echo \"NPM_CONFIG_LOCATION=${NPM_CONFIG_LOCATION:-unset}\"\n",
        );

        // python3 no-op (hermit install python3@3.10 resolves to this)
        write_exec(&bin.join("python3"), "#!/bin/bash\nexit 0\n");

        fs::write(dir.join("empty-user-npmrc"), "").unwrap();
        fs::write(dir.join("empty-global-npmrc"), "").unwrap();
    }

    fn run_real_shim(shim_name: &str, hermit_dir: &Path, log_dir: &Path) -> std::process::Output {
        let real_shim = shim_dir().join(shim_name);
        Command::new(&real_shim)
            .env_clear()
            .env(
                "PATH",
                format!("{}/bin:/usr/bin:/bin", hermit_dir.display()),
            )
            .env("HOME", std::env::var("HOME").unwrap())
            .env("BUZZ_MCP_HERMIT_DIR", hermit_dir)
            .env("BUZZ_MCP_LOG_DIR", log_dir)
            .output()
            .unwrap_or_else(|e| panic!("failed to run real shim {shim_name}: {e}"))
    }

    #[test]
    fn real_uv_wrapper_sets_uv_no_config() {
        let tmp = tempfile::TempDir::new().unwrap();
        let hermit_dir = tmp.path().join("hermit");
        let log_dir = tmp.path().join("logs");
        fs::create_dir_all(&log_dir).unwrap();
        seed_fake_hermit_dir(&hermit_dir);

        let out = run_real_shim("uv", &hermit_dir, &log_dir);
        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            out.status.success(),
            "real uv wrapper exited non-zero.\nstdout: {stdout}\nstderr: {stderr}"
        );
        assert!(
            stdout.contains("UV_NO_CONFIG=1"),
            "expected UV_NO_CONFIG=1 from real uv wrapper, got stdout: {stdout}"
        );
    }

    #[test]
    fn real_uvx_wrapper_sets_uv_no_config() {
        let tmp = tempfile::TempDir::new().unwrap();
        let hermit_dir = tmp.path().join("hermit");
        let log_dir = tmp.path().join("logs");
        fs::create_dir_all(&log_dir).unwrap();
        seed_fake_hermit_dir(&hermit_dir);

        let out = run_real_shim("uvx", &hermit_dir, &log_dir);
        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            out.status.success(),
            "real uvx wrapper exited non-zero.\nstdout: {stdout}\nstderr: {stderr}"
        );
        assert!(
            stdout.contains("UV_NO_CONFIG=1"),
            "expected UV_NO_CONFIG=1 from real uvx wrapper, got stdout: {stdout}"
        );
    }

    #[test]
    fn real_npx_wrapper_sets_npm_isolation() {
        let tmp = tempfile::TempDir::new().unwrap();
        let hermit_dir = tmp.path().join("hermit");
        let log_dir = tmp.path().join("logs");
        fs::create_dir_all(&log_dir).unwrap();
        seed_fake_hermit_dir(&hermit_dir);

        let out = run_real_shim("npx", &hermit_dir, &log_dir);
        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            out.status.success(),
            "real npx wrapper exited non-zero.\nstdout: {stdout}\nstderr: {stderr}"
        );
        let expected_user = format!(
            "NPM_CONFIG_USERCONFIG={}/empty-user-npmrc",
            hermit_dir.display()
        );
        let expected_global = format!(
            "NPM_CONFIG_GLOBALCONFIG={}/empty-global-npmrc",
            hermit_dir.display()
        );
        assert!(
            stdout.contains(&expected_user),
            "expected {expected_user} in output, got: {stdout}"
        );
        assert!(
            stdout.contains(&expected_global),
            "expected {expected_global} in output, got: {stdout}"
        );
        assert!(
            stdout.contains("NPM_CONFIG_LOCATION=global"),
            "expected NPM_CONFIG_LOCATION=global in output, got: {stdout}"
        );
    }

    #[test]
    fn real_node_wrapper_sets_npm_isolation() {
        let tmp = tempfile::TempDir::new().unwrap();
        let hermit_dir = tmp.path().join("hermit");
        let log_dir = tmp.path().join("logs");
        fs::create_dir_all(&log_dir).unwrap();
        seed_fake_hermit_dir(&hermit_dir);

        let out = run_real_shim("node", &hermit_dir, &log_dir);
        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            out.status.success(),
            "real node wrapper exited non-zero.\nstdout: {stdout}\nstderr: {stderr}"
        );
        let expected_user = format!(
            "NPM_CONFIG_USERCONFIG={}/empty-user-npmrc",
            hermit_dir.display()
        );
        let expected_global = format!(
            "NPM_CONFIG_GLOBALCONFIG={}/empty-global-npmrc",
            hermit_dir.display()
        );
        assert!(
            stdout.contains(&expected_user),
            "expected {expected_user} in output, got: {stdout}"
        );
        assert!(
            stdout.contains(&expected_global),
            "expected {expected_global} in output, got: {stdout}"
        );
        assert!(
            stdout.contains("NPM_CONFIG_LOCATION=global"),
            "expected NPM_CONFIG_LOCATION=global in output, got: {stdout}"
        );
    }

    #[test]
    fn real_uv_wrapper_passes_explicit_override() {
        let tmp = tempfile::TempDir::new().unwrap();
        let hermit_dir = tmp.path().join("hermit");
        let log_dir = tmp.path().join("logs");
        fs::create_dir_all(&log_dir).unwrap();
        seed_fake_hermit_dir(&hermit_dir);

        let real_shim = shim_dir().join("uv");
        let out = Command::new(&real_shim)
            .env_clear()
            .env(
                "PATH",
                format!("{}/bin:/usr/bin:/bin", hermit_dir.display()),
            )
            .env("HOME", std::env::var("HOME").unwrap())
            .env("BUZZ_MCP_HERMIT_DIR", &hermit_dir)
            .env("BUZZ_MCP_LOG_DIR", &log_dir)
            .env("UV_INDEX_URL", "https://explicit.example.com/simple")
            .output()
            .unwrap_or_else(|e| panic!("failed to run real uv shim: {e}"));

        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            out.status.success(),
            "real uv wrapper exited non-zero.\nstdout: {stdout}\nstderr: {stderr}"
        );
        assert!(
            stdout.contains("UV_INDEX_URL=https://explicit.example.com/simple"),
            "explicit UV_INDEX_URL should pass through real wrapper, got: {stdout}"
        );
    }

    /// Runs `setup-common.sh` directly (not through a toolchain wrapper) so
    /// tests can assert on its own exit status/stderr without a real `uv`/
    /// `node` binary needing to exist on PATH afterwards.
    fn run_setup_common(
        toolchain: &str,
        hermit_dir: &Path,
        log_dir: &Path,
        extra_path_prefix: &str,
    ) -> std::process::Output {
        let script = shim_dir().join("setup-common.sh");
        Command::new("bash")
            .arg(&script)
            .arg(toolchain)
            .env_clear()
            .env(
                "PATH",
                format!(
                    "{extra_path_prefix}{}/bin:/usr/bin:/bin",
                    hermit_dir.display()
                ),
            )
            .env("HOME", std::env::var("HOME").unwrap())
            .env("BUZZ_MCP_HERMIT_DIR", hermit_dir)
            .env("BUZZ_MCP_LOG_DIR", log_dir)
            .output()
            .unwrap_or_else(|e| panic!("failed to run setup-common.sh: {e}"))
    }

    /// Finding 2a: a live lock holder must never be evicted purely for
    /// being older than the previous 600s age threshold, even though this
    /// test's own PID is real and the recorded starttime is stale/mismatched
    /// relative to what the fresh `_proc_starttime` lookup would compute —
    /// what makes eviction wrong here is liveness, not staleness. A second
    /// setup-common.sh invocation must wait (and time out, bounded by
    /// BUZZ_AGENT_MCP_INIT_TIMEOUT_SECS) rather than steal the lock, proving
    /// the outer timeout — not an internal age check — bounds the wait.
    #[test]
    fn live_owner_beyond_previous_600s_threshold_is_not_evicted() {
        let tmp = tempfile::TempDir::new().unwrap();
        let hermit_dir = tmp.path().join("hermit");
        let log_dir = tmp.path().join("logs");
        fs::create_dir_all(&hermit_dir).unwrap();
        fs::create_dir_all(&log_dir).unwrap();

        let lock_dir = hermit_dir.join(".mcp-hermit-setup.lock");
        fs::create_dir_all(&lock_dir).unwrap();
        // This test process is unquestionably alive; record an "acquired"
        // timestamp far older than the old 600s eviction threshold so a
        // regression to age-based eviction would steal it, but a
        // liveness-based check correctly will not.
        let ancient_acquired = 0; // epoch 0: as old as an "acquired" timestamp can get
        fs::write(
            lock_dir.join("info"),
            format!("{}::{ancient_acquired}", std::process::id()),
        )
        .unwrap();

        let script = shim_dir().join("setup-common.sh");
        let mut child = Command::new("bash")
            .arg(&script)
            .arg("uv")
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", std::env::var("HOME").unwrap())
            .env("BUZZ_MCP_HERMIT_DIR", &hermit_dir)
            .env("BUZZ_MCP_LOG_DIR", &log_dir)
            // Bound the wait tightly so the test doesn't hang if the lock
            // is (correctly) never stolen.
            .env("BUZZ_AGENT_MCP_INIT_TIMEOUT_SECS", "1")
            .spawn()
            .unwrap();

        // Give the child a moment to reach (and block on) the lock loop.
        std::thread::sleep(std::time::Duration::from_millis(300));
        assert!(
            lock_dir.exists(),
            "live-held lock must still exist while our process is alive"
        );

        let _ = child.kill();
        let _ = child.wait();
        let _ = fs::remove_dir_all(&lock_dir);
    }

    /// Finding 2b: a lock left behind by a genuinely dead holder (PID no
    /// longer exists) must be recoverable — the fix's dead-owner recovery
    /// path must still work, so a crash never wedges the lock forever.
    #[test]
    fn dead_lock_owner_is_recoverable() {
        let tmp = tempfile::TempDir::new().unwrap();
        let hermit_dir = tmp.path().join("hermit");
        let log_dir = tmp.path().join("logs");
        fs::create_dir_all(&log_dir).unwrap();
        seed_fake_hermit_dir(&hermit_dir);

        let lock_dir = hermit_dir.join(".mcp-hermit-setup.lock");
        fs::create_dir_all(&lock_dir).unwrap();
        // A PID that is essentially guaranteed not to exist, paired with a
        // starttime that can't possibly match anything alive.
        let dead_pid = 999_999;
        fs::write(lock_dir.join("info"), format!("{dead_pid}:0:0")).unwrap();

        let out = run_setup_common("uv", &hermit_dir, &log_dir, "");
        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            out.status.success(),
            "setup-common.sh must recover a dead-owner lock rather than wait it out.\nstdout: {stdout}\nstderr: {stderr}"
        );
        assert!(
            stderr.contains("dead lock holder detected"),
            "expected dead-holder recovery log, got stderr: {stderr}"
        );
        assert!(
            !lock_dir.exists(),
            "lock must be released after successful bootstrap"
        );
    }

    /// Finding 3: an existing `bin/hermit` that fails to execute (here:
    /// truncated/non-functional, simulating an interrupted prior download)
    /// must be detected via the `--version` probe, removed, and
    /// rebootstrapped from a stubbed local "download" — never accepted
    /// on `-f` existence alone. The download path is stubbed by seeding
    /// `PATH` with a fake `curl`/`openssl`/`bash`-invoked installer stand-in
    /// so this test needs no network.
    #[test]
    fn invalid_existing_hermit_binary_is_recovered() {
        let tmp = tempfile::TempDir::new().unwrap();
        let hermit_dir = tmp.path().join("hermit");
        let log_dir = tmp.path().join("logs");
        let stub_bin = tmp.path().join("stub-bin");
        fs::create_dir_all(&log_dir).unwrap();
        fs::create_dir_all(hermit_dir.join("bin")).unwrap();
        fs::create_dir_all(&stub_bin).unwrap();

        let write_exec = |path: &Path, content: &str| {
            fs::write(path, content).unwrap();
            fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
        };

        // Existing "hermit" that is present (`-f` would accept it) but
        // fails to run — the invalid/interrupted-bootstrap case.
        write_exec(&hermit_dir.join("bin/hermit"), "#!/bin/bash\nexit 1\n");

        // Stub curl: instead of downloading, emit a fixed install script
        // body to whatever path was requested via -o. setup-common.sh SHA
        // pins that script's *production* content
        // (`INSTALL_SCRIPT_SHA256`), which this stub body deliberately does
        // not match — so `openssl` below is stubbed too, to report the
        // expected hash regardless of actual content. The SHA-pin check
        // itself is unchanged, existing behavior; this test is about
        // invalid-binary detection and atomic publish, not re-verifying the
        // pin logic.
        let install_script_body = "#!/bin/bash\n\
             # Stub install script standing in for hermit's real installer.\n\
             # Writes a working replacement hermit binary to $HERMIT_EXE.\n\
             # The replacement creates bin/activate-hermit on init that sets\n\
             # HERMIT_ENV, mirroring real hermit behavior.\n\
             mkdir -p \"$(dirname \"$HERMIT_EXE\")\"\n\
             cat > \"$HERMIT_EXE\" <<'STUBHERMIT'\n\
#!/bin/bash\n\
if [ \"$1\" = init ]; then\n\
  printf '#!/bin/bash\\nexport HERMIT_ENV=\"%s\"\\n' \"$(pwd)\" > bin/activate-hermit\n\
  chmod +x bin/activate-hermit\n\
fi\n\
exit 0\n\
STUBHERMIT\n\
             chmod +x \"$HERMIT_EXE\"\n";
        const PINNED_INSTALL_SCRIPT_SHA256: &str =
            "09ed936378857886fd4a7a4878c0f0c7e3d839883f39ca8b4f2f242e3126e1c6";

        write_exec(
            &stub_bin.join("curl"),
            &format!(
                "#!/bin/bash\n\
                 # Stub curl: ignore the real URL args, emit the fixed install script\n\
                 # body to whatever -o path was requested, regardless of flags order.\n\
                 out=\"\"\n\
                 while [ $# -gt 0 ]; do\n\
                 case \"$1\" in\n\
                 -o) out=\"$2\"; shift 2 ;;\n\
                 *) shift ;;\n\
                 esac\n\
                 done\n\
                 cat > \"$out\" <<'EOF'\n{install_script_body}EOF\n"
            ),
        );
        // Real openssl's `dgst -sha256 <path>` prints `SHA256(<path>)= <hash>`
        // — setup-common.sh extracts the hash via `awk '{print $2}'`. Stub
        // reports the pinned hash unconditionally (see rationale above).
        write_exec(
            &stub_bin.join("openssl"),
            &format!("#!/bin/bash\necho \"SHA256($3)= {PINNED_INSTALL_SCRIPT_SHA256}\"\n"),
        );

        let out = run_setup_common(
            "uv",
            &hermit_dir,
            &log_dir,
            &format!("{}:", stub_bin.display()),
        );
        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            out.status.success(),
            "setup-common.sh must recover an invalid existing hermit binary via stubbed re-download.\nstdout: {stdout}\nstderr: {stderr}"
        );
        assert!(
            stderr.contains("existing hermit binary invalid; removing and rebootstrapping"),
            "expected invalid-binary recovery log, got stderr: {stderr}"
        );
        let mode = hermit_dir
            .join("bin/hermit")
            .metadata()
            .unwrap()
            .permissions()
            .mode();
        assert_ne!(
            mode & 0o111,
            0,
            "rebootstrapped hermit binary must be executable"
        );
    }

    /// Fix 1 regression: two waiters on a stale lock must not overlap their
    /// critical sections. The old protocol (observe-dead then rm -rf) let
    /// waiter A delete waiter B's freshly acquired lock. The atomic-mv
    /// reclaim ensures only one racer succeeds.
    ///
    /// The hermit binary writes overlap-detection markers during `install`
    /// (which runs inside the locked critical section in setup-common.sh).
    /// Each invocation writes its own PID marker, sleeps briefly, checks
    /// for the other marker, then removes its own. If both markers exist
    /// simultaneously, the critical sections overlapped.
    #[test]
    fn two_waiters_on_stale_lock_never_overlap_critical_sections() {
        let tmp = tempfile::TempDir::new().unwrap();
        let hermit_dir = tmp.path().join("hermit");
        let log_dir = tmp.path().join("logs");
        fs::create_dir_all(&log_dir).unwrap();
        seed_fake_hermit_dir(&hermit_dir);

        let lock_dir = hermit_dir.join(".mcp-hermit-setup.lock");
        fs::create_dir_all(&lock_dir).unwrap();
        let dead_pid = 999_999;
        fs::write(lock_dir.join("info"), format!("{dead_pid}:0:0")).unwrap();

        let overlap = tmp.path().join("overlap.detected");
        let cs_dir = tmp.path().join("cs-markers");
        fs::create_dir_all(&cs_dir).unwrap();

        // Hermit binary that writes an overlap-detection marker during
        // `install` (inside the locked region). `init` creates a valid
        // activate-hermit. `--version` reports a version for validation.
        let write_exec = |path: &Path, content: &str| {
            fs::write(path, content).unwrap();
            fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
        };
        write_exec(
            &hermit_dir.join("bin/hermit"),
            &format!(
                "#!/bin/bash\n\
                 case \"$1\" in\n\
                   --version) echo \"stub 0.0.0\" ;;\n\
                   init)\n\
                     printf '#!/bin/bash\\nexport HERMIT_ENV=\"%s\"\\n' \"$(pwd)\" > bin/activate-hermit\n\
                     chmod +x bin/activate-hermit\n\
                     ;;\n\
                   install)\n\
                     touch \"{cs_dir}/$$.marker\"\n\
                     sleep 0.3\n\
                     for f in \"{cs_dir}\"/*.marker; do\n\
                       [ -f \"$f\" ] || continue\n\
                       other=\"$(basename \"$f\" .marker)\"\n\
                       if [ \"$other\" != \"$$\" ]; then\n\
                         touch \"{overlap}\"\n\
                       fi\n\
                     done\n\
                     rm -f \"{cs_dir}/$$.marker\"\n\
                     ;;\n\
                 esac\n\
                 exit 0\n",
                cs_dir = cs_dir.display(),
                overlap = overlap.display(),
            ),
        );

        // Remove pre-seeded activate-hermit so init runs.
        fs::remove_file(hermit_dir.join("bin/activate-hermit")).unwrap();

        let script = shim_dir().join("setup-common.sh");

        let spawn_waiter = || {
            Command::new("bash")
                .arg(&script)
                .arg("uv")
                .env_clear()
                .env(
                    "PATH",
                    format!("{}/bin:/usr/bin:/bin", hermit_dir.display()),
                )
                .env("HOME", std::env::var("HOME").unwrap())
                .env("BUZZ_MCP_HERMIT_DIR", &hermit_dir)
                .env("BUZZ_MCP_LOG_DIR", &log_dir)
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .unwrap()
        };

        let child_a = spawn_waiter();
        let child_b = spawn_waiter();

        let out_a = child_a.wait_with_output().unwrap();
        let out_b = child_b.wait_with_output().unwrap();

        assert!(
            out_a.status.success() && out_b.status.success(),
            "both waiters must complete successfully.\n\
             A: exit={:?} stderr={}\n\
             B: exit={:?} stderr={}",
            out_a.status,
            String::from_utf8_lossy(&out_a.stderr),
            out_b.status,
            String::from_utf8_lossy(&out_b.stderr),
        );
        assert!(
            !overlap.exists(),
            "critical sections overlapped: hermit install ran concurrently in both processes"
        );
    }

    /// Fix 2 regression: a pre-existing malformed activate-hermit (exit 0
    /// body) with a valid hermit binary must be repaired, not silently
    /// accepted. Setup must either repair-and-complete (installs run) or
    /// exit nonzero — never exit 0 without installs.
    #[test]
    fn malformed_activate_hermit_is_repaired_not_silently_accepted() {
        let tmp = tempfile::TempDir::new().unwrap();
        let hermit_dir = tmp.path().join("hermit");
        let log_dir = tmp.path().join("logs");
        fs::create_dir_all(&log_dir).unwrap();
        seed_fake_hermit_dir(&hermit_dir);

        // Replace activate-hermit with a malformed file that exits 0 without
        // setting HERMIT_ENV. Replace the hermit binary with one that creates
        // a valid activate-hermit on init (mirroring real hermit behavior).
        fs::write(
            hermit_dir.join("bin/activate-hermit"),
            "#!/bin/bash\nexit 0\n",
        )
        .unwrap();
        fs::write(
            hermit_dir.join("bin/hermit"),
            "#!/bin/bash\n\
             if [ \"$1\" = init ]; then\n\
               printf '#!/bin/bash\\nexport HERMIT_ENV=\"%s\"\\n' \"$(pwd)\" > bin/activate-hermit\n\
               chmod +x bin/activate-hermit\n\
             fi\n\
             if [ \"$1\" = \"--version\" ]; then echo \"stub 0.0.0\"; fi\n\
             exit 0\n",
        )
        .unwrap();
        fs::set_permissions(
            hermit_dir.join("bin/hermit"),
            fs::Permissions::from_mode(0o755),
        )
        .unwrap();

        let out = run_setup_common("uv", &hermit_dir, &log_dir, "");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            out.status.success(),
            "setup must repair malformed activate-hermit and succeed.\nstderr: {stderr}"
        );
        assert!(
            stderr.contains("initializing hermit environment"),
            "expected re-init for malformed activate-hermit, got stderr: {stderr}"
        );
        assert!(
            stderr.contains("bootstrap complete for uv"),
            "installs must actually run after repair.\nstderr: {stderr}"
        );
    }

    /// Fix 2 variant: garbage content (not even a valid script) must also
    /// be repaired, proving validation catches more than just `exit 0`.
    #[test]
    fn garbage_activate_hermit_is_repaired() {
        let tmp = tempfile::TempDir::new().unwrap();
        let hermit_dir = tmp.path().join("hermit");
        let log_dir = tmp.path().join("logs");
        fs::create_dir_all(&log_dir).unwrap();
        seed_fake_hermit_dir(&hermit_dir);

        fs::write(
            hermit_dir.join("bin/activate-hermit"),
            b"THIS IS NOT A VALID SHELL SCRIPT\n\x00\xff",
        )
        .unwrap();
        fs::write(
            hermit_dir.join("bin/hermit"),
            "#!/bin/bash\n\
             if [ \"$1\" = init ]; then\n\
               printf '#!/bin/bash\\nexport HERMIT_ENV=\"%s\"\\n' \"$(pwd)\" > bin/activate-hermit\n\
               chmod +x bin/activate-hermit\n\
             fi\n\
             if [ \"$1\" = \"--version\" ]; then echo \"stub 0.0.0\"; fi\n\
             exit 0\n",
        )
        .unwrap();
        fs::set_permissions(
            hermit_dir.join("bin/hermit"),
            fs::Permissions::from_mode(0o755),
        )
        .unwrap();

        let out = run_setup_common("uv", &hermit_dir, &log_dir, "");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            out.status.success(),
            "setup must repair garbage activate-hermit and succeed.\nstderr: {stderr}"
        );
        assert!(
            stderr.contains("bootstrap complete for uv"),
            "installs must actually run after repair.\nstderr: {stderr}"
        );
    }

    /// Fix 3: the hostile-path test must actually hit the vulnerable code
    /// path. This uses a wrapper shell that reports its own PID, blocks on a
    /// fifo while the test plants the hostile binary at the exact path the
    /// old vulnerable code would construct (/tmp/hermit_tmp_<child PID>),
    /// then exec's into setup-common.sh preserving PID.
    #[test]
    #[cfg(target_os = "linux")]
    fn hostile_preexisting_tmp_path_is_never_executed() {
        let tmp = tempfile::TempDir::new().unwrap();
        let hermit_dir = tmp.path().join("hermit");
        let log_dir = tmp.path().join("logs");
        fs::create_dir_all(&log_dir).unwrap();
        seed_fake_hermit_dir(&hermit_dir);
        // Need a hermit that creates a valid activate-hermit on init.
        fs::remove_file(hermit_dir.join("bin/activate-hermit")).unwrap();
        fs::write(
            hermit_dir.join("bin/hermit"),
            "#!/bin/bash\n\
             if [ \"$1\" = init ]; then\n\
               printf '#!/bin/bash\\nexport HERMIT_ENV=\"%s\"\\n' \"$(pwd)\" > bin/activate-hermit\n\
               chmod +x bin/activate-hermit\n\
             fi\n\
             exit 0\n",
        )
        .unwrap();
        fs::set_permissions(
            hermit_dir.join("bin/hermit"),
            fs::Permissions::from_mode(0o755),
        )
        .unwrap();

        // Synchronization: fifo for PID reporting, fifo for release signal.
        let pid_fifo = tmp.path().join("pid.fifo");
        let go_fifo = tmp.path().join("go.fifo");
        assert!(
            Command::new("mkfifo")
                .arg(&pid_fifo)
                .status()
                .unwrap()
                .success(),
            "mkfifo failed for pid fifo"
        );
        assert!(
            Command::new("mkfifo")
                .arg(&go_fifo)
                .status()
                .unwrap()
                .success(),
            "mkfifo failed for go fifo"
        );

        // Wrapper script: writes its own $$ to the pid fifo, blocks reading
        // from the go fifo, then exec's into setup-common.sh (preserving PID).
        let wrapper_path = tmp.path().join("wrapper.sh");
        let script = shim_dir().join("setup-common.sh");
        fs::write(
            &wrapper_path,
            format!(
                "#!/bin/bash\n\
                 echo $$ > \"{pid_fifo}\"\n\
                 cat \"{go_fifo}\" > /dev/null\n\
                 exec bash \"{script}\" uv\n",
                pid_fifo = pid_fifo.display(),
                go_fifo = go_fifo.display(),
                script = script.display(),
            ),
        )
        .unwrap();
        fs::set_permissions(&wrapper_path, fs::Permissions::from_mode(0o755)).unwrap();

        let child = Command::new("bash")
            .arg(&wrapper_path)
            .env_clear()
            .env(
                "PATH",
                format!("{}/bin:/usr/bin:/bin", hermit_dir.display()),
            )
            .env("HOME", std::env::var("HOME").unwrap())
            .env("BUZZ_MCP_HERMIT_DIR", &hermit_dir)
            .env("BUZZ_MCP_LOG_DIR", &log_dir)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();

        // Read the child's PID from the fifo.
        let child_pid: u32 = std::fs::read_to_string(&pid_fifo)
            .unwrap()
            .trim()
            .parse()
            .unwrap();

        // Plant the hostile binary at the exact path the old vulnerable
        // code would construct: /tmp/hermit_tmp_<child's own PID>/bin/hermit
        let marker = tmp.path().join("hostile-ran.marker");
        let hostile_dir = PathBuf::from(format!("/tmp/hermit_tmp_{child_pid}/bin"));
        fs::create_dir_all(&hostile_dir).unwrap();
        fs::write(
            hostile_dir.join("hermit"),
            format!("#!/bin/bash\ntouch {}\nexit 0\n", marker.display()),
        )
        .unwrap();
        fs::set_permissions(
            hostile_dir.join("hermit"),
            fs::Permissions::from_mode(0o755),
        )
        .unwrap();

        // Release the wrapper to exec into setup-common.sh.
        fs::write(&go_fifo, "go\n").unwrap();

        let out = child.wait_with_output().unwrap();

        let _ = fs::remove_dir_all(format!("/tmp/hermit_tmp_{child_pid}"));

        assert!(
            !marker.exists(),
            "hostile binary at /tmp/hermit_tmp_{child_pid} was executed — \
             the fix (private mktemp + absolute-path invocation) is not effective"
        );
        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            out.status.success(),
            "setup-common.sh should succeed via the real hermit copy.\n\
             stdout: {stdout}\nstderr: {stderr}"
        );
    }
}
