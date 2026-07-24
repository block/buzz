use std::collections::BTreeSet;
use std::sync::Mutex;

use serde_json::Value;
use tokio_util::sync::CancellationToken;

use super::audit::{PersistedTerminal, TerminalAuditInput};
use super::orchestrator::{BriefFuture, BriefPersistence, BriefPersistenceError};
use super::types::{PublicationState, PublishedCommandBrief};

#[derive(Default)]
pub(super) struct FakePersistence {
    pub(super) values: Mutex<Vec<Value>>,
    pub(super) tokens: Mutex<Vec<CancellationToken>>,
    committed: Mutex<BTreeSet<String>>,
    pub(super) terminals: Mutex<
        Vec<(
            buzz_core_pkg::command_brief::CommandBriefLifecycleState,
            Option<buzz_core_pkg::command_brief::CommandBriefFailureCode>,
        )>,
    >,
    pub(super) wait_for_cancel: bool,
    pub(super) fail: bool,
}

impl FakePersistence {
    pub(super) fn waiting_for_cancel() -> Self {
        Self {
            wait_for_cancel: true,
            ..Self::default()
        }
    }

    pub(super) fn failing() -> Self {
        Self {
            fail: true,
            ..Self::default()
        }
    }

    pub(super) fn assert_one_terminal(
        &self,
        state: buzz_core_pkg::command_brief::CommandBriefLifecycleState,
        code: Option<buzz_core_pkg::command_brief::CommandBriefFailureCode>,
    ) {
        assert_eq!(
            self.terminals.lock().expect("terminals").as_slice(),
            &[(state, code)]
        );
    }
}

impl BriefPersistence for FakePersistence {
    fn persist_terminal<'a>(
        &'a self,
        input: TerminalAuditInput,
        cancellation: CancellationToken,
    ) -> BriefFuture<'a, Result<PersistedTerminal, BriefPersistenceError>> {
        Box::pin(async move {
            self.tokens
                .lock()
                .expect("persistence token lock")
                .push(cancellation.clone());
            if self.wait_for_cancel {
                cancellation.cancelled().await;
            }
            if self.fail {
                return Err(BriefPersistenceError::Failed);
            }
            let input = if cancellation.is_cancelled() {
                input.into_cancelled()
            } else {
                input
            };
            if let Some(brief) = input.final_brief() {
                self.values
                    .lock()
                    .expect("persistence values lock")
                    .push(serde_json::to_value(brief).expect("serialize brief"));
            }
            self.terminals
                .lock()
                .expect("terminal lock")
                .push((input.lifecycle_state(), input.failure_code()));
            self.committed
                .lock()
                .expect("committed lock")
                .insert(input.run_id().to_string());
            let published = input.final_brief().map(|brief| {
                PublishedCommandBrief::new(brief.clone(), "a".repeat(64), PublicationState::Queued)
            });
            Ok(PersistedTerminal::new(
                input.lifecycle_state(),
                "a".repeat(64),
                PublicationState::Queued,
                published,
            ))
        })
    }

    fn request_cancel(&self, run_id: &str, cancellation: &CancellationToken) -> bool {
        if self
            .committed
            .lock()
            .expect("committed lock")
            .contains(run_id)
        {
            return false;
        }
        cancellation.cancel();
        true
    }
}
