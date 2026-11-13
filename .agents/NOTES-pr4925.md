# PR 4925 working notes

## Long-running operations: always run DETACHED, then sleep

`cargo check -p buzz-cli --all-targets` (and any `cargo check`/`clippy` over the
full workspace, incl. `desktop/src-tauri`) regularly takes **2.5–6+ minutes** on
this host. Running it inline overruns the turn wall-clock (a 10-minute turn was
killed mid-`cargo check`).

Correct pattern:

    nohup cargo check -p buzz-cli --all-targets > /tmp/buzz4925-check.log 2>&1 &
    # then end the turn with sleep(minutes=N) and resume from the log

Do NOT re-run the same compile inline to "wait it out". Read the log file on
wake.

## Result already established (do not redo)

- Merge `upstream/main` (1adde0e6b) into 30795daa5 => conflicts in exactly
  `crates/buzz-cli/src/lib.rs` (2), `crates/buzz-admin/src/main.rs` (1),
  `Cargo.lock` (9).
- Taking upstream's side => `error[E0609]: no field private_key on type Cli`.
- Taking the branch's side => 26 errors (48x `cannot find value client`,
  `Keys`, `command`).
- => Semantic conflict. No textual resolution compiles. Branch untouched.
- `darrenoakey/daz-secrets` is archived; crates.io 404; successor
  `boringstackai/daz-agentic` is Go-only, so the pin cannot be repointed.
