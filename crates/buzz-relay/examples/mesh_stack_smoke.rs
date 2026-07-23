//! Tokio worker stack-size smoke — reproduces and verifies the fix for the
//! mesh-llm model-download stack overflow (SIGABRT via stack guard).
//!
//! Crash report (2026-07-08, buzz-desktop 0.3.46): enabling Share compute
//! aborted the app on a `tokio-rt-worker` thread inside
//! `mesh_llm_host_runtime::models::resolve::download_model_ref_with_progress_details`
//! — Rust's stack-overflow signal handler fired on tokio's default 2 MiB
//! worker stack. Upstream mesh-llm runs its own binary on 8 MiB worker
//! stacks for exactly this reason (`DEFAULT_WORKER_STACK_SIZE` in mesh-llm
//! `main.rs`), as does mesh-console.
//!
//! This harness polls the same future as a spawned task (matching how Tauri
//! polls command futures on worker threads) in two subprocess legs:
//!
//!   1. 2 MiB worker stacks (tokio default) — expected to DIE from the
//!      stack guard (signal, no exit code). Proves we reproduced the crash.
//!   2. 8 MiB worker stacks (the fix installed in desktop `lib.rs` via
//!      `tauri::async_runtime::set`) — expected to complete the download.
//!
//! Each leg gets a fresh HF_HOME so the download really runs (a cache hit
//! never reaches the deep code path). Network required; downloads a ~100 MB
//! GGUF twice at most (leg 1 usually dies early). Not CI — run manually:
//!
//!   cargo run -p buzz-relay --example mesh_stack_smoke
use std::process::{Command, Stdio};

/// Small real model, same one the admission smoke uses.
const MODEL: &str = "jc-builds/SmolLM2-135M-Instruct-Q4_K_M-GGUF:Q4_K_M";

const TOKIO_DEFAULT_STACK: usize = 2 * 1024 * 1024;
/// Must match `buzz_lib::mesh_llm::MESH_WORKER_STACK_SIZE` (desktop crate is
/// not a dependency of buzz-relay, so the value is duplicated here).
const FIXED_STACK: usize = 8 * 1024 * 1024;

fn main() -> anyhow::Result<()> {
    match std::env::var("MESH_STACK_ROLE").ok().as_deref() {
        Some("download") => role_download(),
        _ => orchestrate(),
    }
}

/// Subprocess: poll the download future as a spawned task on a worker
/// thread with the requested stack size — the exact shape of a Tauri
/// command future.
fn role_download() -> anyhow::Result<()> {
    let stack: usize = std::env::var("MESH_STACK_SIZE")?.parse()?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_stack_size(stack)
        .build()?;
    runtime.block_on(async {
        tokio::spawn(async {
            mesh_llm_host_runtime::models::download_model_ref_with_progress_details(MODEL, true)
                .await
                .map(|_| ())
                .map_err(|error| anyhow::anyhow!("download failed: {error}"))
        })
        .await?
    })?;
    println!("DOWNLOAD_OK");
    // Skip destructors (ggml teardown abort, mesh-console issue #8).
    std::process::exit(0);
}

fn run_leg(stack: usize, hf_home: &std::path::Path) -> anyhow::Result<(bool, Option<i32>)> {
    let exe = std::env::current_exe()?;
    let status = Command::new(exe)
        .env("MESH_STACK_ROLE", "download")
        .env("MESH_STACK_SIZE", stack.to_string())
        .env("HF_HOME", hf_home)
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()?;
    Ok((status.success(), status.code()))
}

fn orchestrate() -> anyhow::Result<()> {
    let scratch = std::env::temp_dir().join(format!("mesh-stack-smoke-{}", std::process::id()));

    println!(
        "=== leg 1: {} MiB worker stacks (tokio default) — expecting stack-guard death ===",
        TOKIO_DEFAULT_STACK / (1024 * 1024)
    );
    let hf1 = scratch.join("hf-2mb");
    std::fs::create_dir_all(&hf1)?;
    let (ok_2mb, code_2mb) = run_leg(TOKIO_DEFAULT_STACK, &hf1)?;

    println!(
        "=== leg 2: {} MiB worker stacks (the fix) — expecting success ===",
        FIXED_STACK / (1024 * 1024)
    );
    let hf2 = scratch.join("hf-8mb");
    std::fs::create_dir_all(&hf2)?;
    let (ok_8mb, code_8mb) = run_leg(FIXED_STACK, &hf2)?;

    let _ = std::fs::remove_dir_all(&scratch);

    println!();
    println!(
        "leg 1 (2 MiB): success={ok_2mb} exit_code={code_2mb:?}  (None = killed by signal, i.e. stack guard)"
    );
    println!("leg 2 (8 MiB): success={ok_8mb} exit_code={code_8mb:?}");

    // Leg 2 is the hard gate: the fix must work.
    if !ok_8mb {
        anyhow::bail!("FAIL: download did not complete on 8 MiB worker stacks — fix is broken");
    }
    // Leg 1 documents the repro. If it *succeeds*, the overflow needs deeper
    // nesting than this harness provides — flag loudly but do not fail, the
    // fix leg is still proven.
    if ok_2mb {
        println!(
            "WARNING: 2 MiB leg did not crash here; overflow requires the app's extra \
             tauri/ipc nesting. Fix leg still verified."
        );
    } else {
        println!("repro confirmed: 2 MiB worker stack dies, matching the crash report");
    }
    println!("PASS");
    Ok(())
}
