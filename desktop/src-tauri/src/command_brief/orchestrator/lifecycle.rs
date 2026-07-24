//! Durable terminal-state installation for the command-brief orchestrator.

use super::*;

impl CommandBriefOrchestrator {
    pub(super) async fn persist_closed_and_install(
        &self,
        run_id: &str,
        schedule_id: &str,
        snapshot_id: &str,
        failure_code: CommandBriefFailureCode,
        degraded: &[BriefSection],
        cancellation: CancellationToken,
    ) {
        let lifecycle_state = if failure_code == CommandBriefFailureCode::CancellationRequested {
            CommandBriefLifecycleState::Cancelled
        } else {
            CommandBriefLifecycleState::Failed
        };
        let input = match TerminalAuditInput::closed(
            run_id.to_string(),
            schedule_id.to_string(),
            timestamp(),
            snapshot_id.to_string(),
            lifecycle_state,
            failure_code,
        ) {
            Ok(input) => input,
            Err(_) => {
                self.terminal_local_failure(run_id, schedule_id, degraded);
                return;
            }
        };
        let display_error =
            (lifecycle_state == CommandBriefLifecycleState::Failed).then_some(failure_code);
        self.persist_and_install(
            run_id,
            schedule_id,
            degraded,
            input,
            display_error,
            cancellation,
        )
        .await;
    }

    pub(super) async fn persist_and_install(
        &self,
        run_id: &str,
        schedule_id: &str,
        degraded: &[BriefSection],
        input: TerminalAuditInput,
        display_error: Option<CommandBriefFailureCode>,
        cancellation: CancellationToken,
    ) {
        let persisted = self
            .inner
            .persistence
            .persist_terminal(input, cancellation)
            .await;
        let Ok(persisted) = persisted else {
            self.terminal_local_failure(run_id, schedule_id, degraded);
            return;
        };
        self.inner.finalization_gate.wait().await;
        let state = match persisted.lifecycle_state() {
            CommandBriefLifecycleState::Completed => BriefRunState::Completed,
            CommandBriefLifecycleState::Degraded => BriefRunState::Degraded,
            CommandBriefLifecycleState::Cancelled => BriefRunState::Cancelled,
            CommandBriefLifecycleState::Failed => BriefRunState::Failed,
        };
        let result = persisted.published_brief().cloned();
        self.terminal(
            run_id,
            schedule_id,
            state,
            degraded,
            display_error.map(CommandBriefFailureCode::as_str),
            result,
        );
    }

    fn terminal_local_failure(&self, run_id: &str, schedule_id: &str, degraded: &[BriefSection]) {
        self.terminal(
            run_id,
            schedule_id,
            BriefRunState::Failed,
            degraded,
            Some(CommandBriefFailureCode::BriefPersistenceFailed.as_str()),
            None,
        );
    }
}
