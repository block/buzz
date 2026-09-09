//! Shared ownership of a plugin process group through signaling and reaping.

use super::PluginError;
use std::sync::Mutex;
use tokio::process::Child;

#[cfg(target_os = "macos")]
#[path = "zombie_group.rs"]
mod zombie_group;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum GroupSignal {
    Term,
    Kill,
}

pub(super) struct ProcessOwner {
    state: Mutex<ProcessState>,
}

struct ProcessState {
    child: Child,
    process_group_id: Option<u32>,
    group_finished: bool,
    reaped: bool,
    #[cfg(test)]
    fail_signals: bool,
    #[cfg(target_os = "macos")]
    probe: zombie_group::Probe,
}

impl ProcessOwner {
    pub(super) fn new(child: Child, process_group_id: Option<u32>) -> Self {
        Self {
            state: Mutex::new(ProcessState {
                child,
                process_group_id,
                group_finished: false,
                reaped: false,
                #[cfg(test)]
                fail_signals: false,
                #[cfg(target_os = "macos")]
                probe: zombie_group::Probe::default(),
            }),
        }
    }

    pub(super) fn is_terminated(&self) -> bool {
        self.state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .reaped
    }

    #[cfg(test)]
    pub(super) fn fail_signals_for_test(&self, fail: bool) {
        self.state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .fail_signals = fail;
    }

    pub(super) fn signal(&self, signal: GroupSignal) -> Result<(), PluginError> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if state.group_finished {
            return Ok(());
        }
        #[cfg(test)]
        if state.fail_signals {
            return Err(PluginError::PluginProtocol(
                "injected termination failure for test".into(),
            ));
        }
        let process_group_id = state.process_group_id.ok_or(PluginError::Unsupported)?;
        #[cfg(unix)]
        {
            use nix::sys::signal::{killpg, Signal};
            use nix::unistd::Pid;
            let native_signal = match signal {
                GroupSignal::Term => Signal::SIGTERM,
                GroupSignal::Kill => Signal::SIGKILL,
            };
            match killpg(Pid::from_raw(process_group_id as i32), native_signal) {
                Ok(()) => {
                    if signal == GroupSignal::Kill {
                        state.group_finished = true;
                    }
                    Ok(())
                }
                Err(nix::errno::Errno::ESRCH) => {
                    state.group_finished = true;
                    Ok(())
                }
                Err(error) => {
                    // The unreaped leader prevents group-id reuse throughout the
                    // query. A leader's exit alone says nothing about descendants.
                    #[cfg(target_os = "macos")]
                    if error == nix::errno::Errno::EPERM
                        && state.probe.all_zombies(process_group_id)
                    {
                        state.group_finished = true;
                        return Ok(());
                    }
                    Err(PluginError::PluginProtocol(format!(
                        "signal process group {process_group_id}: {error}"
                    )))
                }
            }
        }
        #[cfg(not(unix))]
        {
            let _ = (process_group_id, signal);
            Err(PluginError::Unsupported)
        }
    }

    pub(super) fn try_reap(&self) -> Option<Result<(), PluginError>> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if state.reaped {
            return Some(Ok(()));
        }
        if !state.group_finished {
            return None;
        }
        #[cfg(target_os = "macos")]
        if !state.probe.reap() {
            return None;
        }
        // Never reap before the final group decision: the leader's zombie
        // retains the group id while any signal or complete-group query runs.
        match state.child.try_wait() {
            Ok(Some(_)) => {
                state.reaped = true;
                Some(Ok(()))
            }
            Ok(None) => None,
            Err(error) => Some(Err(PluginError::PluginProtocol(format!(
                "reap plugin process: {error}"
            )))),
        }
    }
}
