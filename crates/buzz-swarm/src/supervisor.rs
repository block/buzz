//! Run one harness per connection and stop its process group before reaping it.

use std::io;
use std::process::Stdio;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use rustix::process::{kill_process_group, waitid, Pid, Signal, WaitId, WaitIdOptions};
use tokio::process::{Child, Command};
use tokio::sync::watch;
use tokio::task::JoinSet;
use tokio::time::sleep;

use crate::config::Restart;
use crate::plan::{AgentPlan, Plan};

const HEALTHY_UPTIME: Duration = Duration::from_secs(60);
const BACKOFF_CAP: Duration = Duration::from_secs(60);
const RUN: u8 = 0;
const DRAIN: u8 = 1;
const KILL: u8 = 2;

/// Final result for one agent/relay connection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Completed,
    Stopped,
    GaveUp { status: String },
    SpawnFailed { error: String },
}

/// Outcomes after all connections have ended.
#[derive(Debug)]
pub struct Summary {
    pub outcomes: Vec<(String, Outcome)>,
}

impl Summary {
    /// Clean exits and requested shutdowns are successful.
    pub fn failed(&self) -> bool {
        self.outcomes.iter().any(|(_, outcome)| {
            matches!(
                outcome,
                Outcome::GaveUp { .. } | Outcome::SpawnFailed { .. }
            )
        })
    }
}

/// Process shutdown settings shared by all connections.
#[derive(Debug, Clone, Copy)]
pub struct Options {
    /// Time allowed after SIGTERM before SIGKILL.
    pub grace: Duration,
}

/// First signal requests a graceful stop; a second forces every group to exit.
pub async fn run(plan: Plan, options: Options) -> Result<Summary> {
    // Install handlers before spawning anything.
    let mut interrupt = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let (tx, rx) = watch::channel(RUN);
    let mut tasks = JoinSet::new();
    for agent in plan.agents {
        tasks.spawn(supervise(agent, rx.clone(), options));
    }
    let mut outcomes = Vec::with_capacity(tasks.len());
    while !tasks.is_empty() {
        tokio::select! {
            task = tasks.join_next() => {
                if let Some(task) = task {
                    outcomes.push(task.context("agent supervisor task failed")?);
                }
            }
            _ = interrupt.recv() => { tx.send_modify(|state| *state = (*state + 1).min(KILL)); }
            _ = terminate.recv() => { tx.send_modify(|state| *state = (*state + 1).min(KILL)); }
        }
    }
    Ok(Summary { outcomes })
}

async fn supervise(
    plan: AgentPlan,
    mut shutdown: watch::Receiver<u8>,
    options: Options,
) -> (String, Outcome) {
    let mut strikes = 0;
    loop {
        if *shutdown.borrow() != RUN {
            return (plan.name, Outcome::Stopped);
        }
        let started = Instant::now();
        let mut child = match spawn(&plan) {
            Ok(child) => child,
            Err(error) => {
                return (
                    plan.name,
                    Outcome::SpawnFailed {
                        error: error.to_string(),
                    },
                );
            }
        };
        let Some(pid) = child
            .id()
            .and_then(|pid| i32::try_from(pid).ok())
            .and_then(Pid::from_raw)
        else {
            return (
                plan.name,
                Outcome::SpawnFailed {
                    error: "spawned child has no PID".into(),
                },
            );
        };
        let mut group = ProcessGroup(Some(pid));
        let mut interrupted = false;
        let exited = tokio::select! {
            status = exited(pid) => status,
            _ = wait_for(&mut shutdown, DRAIN) => {
                interrupted = true;
                terminate(pid, &mut shutdown, options.grace).await
            }
        };
        // A waitable leader reserves the PID until cleanup, so it cannot be
        // recycled into an unrelated group between exit and kill.
        let cleanup = group.kill();
        let status = match exited.and(cleanup) {
            Ok(()) => child.wait().await,
            Err(error) => Err(error),
        };
        let status = match status {
            Ok(_) if interrupted || *shutdown.borrow() != RUN => {
                return (plan.name, Outcome::Stopped);
            }
            Ok(status) => status,
            Err(error) => {
                return (
                    plan.name,
                    Outcome::GaveUp {
                        status: error.to_string(),
                    },
                );
            }
        };
        if started.elapsed() >= HEALTHY_UPTIME {
            strikes = 0;
        }
        match decide(plan.restart, status.success(), strikes, plan.max_restarts) {
            Next::Stop => {
                let outcome = if status.success() {
                    Outcome::Completed
                } else {
                    Outcome::GaveUp {
                        status: status.to_string(),
                    }
                };
                return (plan.name, outcome);
            }
            Next::Restart(delay) => {
                strikes += 1;
                tokio::select! {
                    _ = sleep(delay) => {}
                    _ = wait_for(&mut shutdown, DRAIN) => return (plan.name, Outcome::Stopped),
                }
            }
        }
    }
}

fn spawn(plan: &AgentPlan) -> io::Result<Child> {
    let mut command = Command::new(&plan.program);
    command
        .args(&plan.args)
        .current_dir(&plan.workdir)
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .kill_on_drop(true)
        .process_group(0);
    for key in &plan.env_remove {
        command.env_remove(key);
    }
    command.envs(&plan.env).spawn()
}

/// Observe exit without reaping; ProcessGroup owns the cleanup boundary.
async fn exited(pid: Pid) -> io::Result<()> {
    let mut signal = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::child())?;
    loop {
        match waitid(
            WaitId::Pid(pid),
            WaitIdOptions::EXITED | WaitIdOptions::NOHANG | WaitIdOptions::NOWAIT,
        ) {
            Ok(Some(_)) => return Ok(()),
            Ok(None) => {
                signal.recv().await;
            }
            Err(rustix::io::Errno::INTR) => continue,
            Err(error) => return Err(error.into()),
        }
    }
}

async fn terminate(
    pid: Pid,
    shutdown: &mut watch::Receiver<u8>,
    grace: Duration,
) -> io::Result<()> {
    signal_group(pid, Signal::TERM)?;
    tokio::select! {
        status = exited(pid) => status,
        _ = sleep(grace) => Ok(()),
        _ = wait_for(shutdown, KILL) => Ok(()),
    }
}

struct ProcessGroup(Option<Pid>);

impl ProcessGroup {
    fn kill(&mut self) -> io::Result<()> {
        if let Some(pid) = self.0 {
            signal_group(pid, Signal::KILL)?;
            self.0 = None;
        }
        Ok(())
    }
}

impl Drop for ProcessGroup {
    fn drop(&mut self) {
        // Task cancellation and unwinding must also kill the group, before
        // dropping the Child lets Tokio reap its leader.
        let _ = self.kill();
    }
}

fn signal_group(pid: Pid, signal: Signal) -> io::Result<()> {
    match kill_process_group(pid, signal) {
        Ok(()) | Err(rustix::io::Errno::SRCH) => Ok(()),
        // Darwin reports EPERM for a group containing only its zombie leader.
        #[cfg(target_os = "macos")]
        Err(rustix::io::Errno::PERM)
            if waitid(
                WaitId::Pid(pid),
                WaitIdOptions::EXITED | WaitIdOptions::NOHANG | WaitIdOptions::NOWAIT,
            )
            .is_ok_and(|status| status.is_some()) =>
        {
            Ok(())
        }
        Err(error) => Err(error.into()),
    }
}

async fn wait_for(rx: &mut watch::Receiver<u8>, level: u8) {
    loop {
        if *rx.borrow() >= level {
            return;
        }
        if rx.changed().await.is_err() {
            std::future::pending::<()>().await;
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Next {
    Restart(Duration),
    Stop,
}

fn decide(policy: Restart, success: bool, strikes: u32, max_restarts: u32) -> Next {
    match policy {
        Restart::Never => Next::Stop,
        Restart::OnFailure if success => Next::Stop,
        _ if strikes >= max_restarts => Next::Stop,
        _ => Next::Restart(backoff(strikes)),
    }
}

fn backoff(strikes: u32) -> Duration {
    BACKOFF_CAP.min(Duration::from_secs(
        1u64.checked_shl(strikes).unwrap_or(u64::MAX),
    ))
}

#[cfg(test)]
mod tests;
