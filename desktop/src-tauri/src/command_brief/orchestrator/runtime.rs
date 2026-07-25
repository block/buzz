use super::*;

/// A redacted start error for invalid or capacity-exhausted run requests.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OrchestratorStartError {
    /// The request or protected run registry could not be admitted safely.
    Rejected,
    /// All bounded nonterminal command-run slots are occupied.
    AdmissionUnavailable,
}

impl fmt::Display for OrchestratorStartError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("command brief run rejected")
    }
}

impl std::error::Error for OrchestratorStartError {}

/// Redacted capacity state for admitting another top-level command run.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum OrchestratorAdmissionState {
    /// The run registry is readable and exposes only bounded counters.
    Available {
        /// Number of queued or running command runs.
        tracked_nonterminal: usize,
        /// Maximum number of tracked nonterminal command runs.
        capacity: usize,
    },
    /// The run registry cannot be inspected safely.
    Unavailable,
}

impl CommandBriefOrchestrator {
    /// Return the trusted, metadata-only top-level run admission state.
    pub(crate) fn admission_state(&self) -> OrchestratorAdmissionState {
        self.inner
            .runs
            .lock()
            .map_or(OrchestratorAdmissionState::Unavailable, |runs| {
                OrchestratorAdmissionState::Available {
                    tracked_nonterminal: runs
                        .values()
                        .filter(|record| !is_terminal(record.state))
                        .count(),
                    capacity: MAX_TRACKED_RUNS,
                }
            })
    }

    #[cfg(test)]
    pub(crate) fn poison_admission_lock_for_test(&self, detail: &str) {
        let inner = Arc::clone(&self.inner);
        let detail = detail.to_string();
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
            let _guard = inner.runs.lock().expect("test run registry");
            std::panic::panic_any(detail);
        }));
    }

    /// Starts a unique trusted run and returns immediately with its UUID.
    pub fn start(&self, request: CommandBriefRequest) -> Result<String, OrchestratorStartError> {
        let run_id = uuid::Uuid::new_v4().to_string();
        self.start_exact(&run_id, request)
    }

    /// Idempotently starts one native-owned exact run identity.
    pub fn start_exact(
        &self,
        run_id: &str,
        request: CommandBriefRequest,
    ) -> Result<String, OrchestratorStartError> {
        if !valid_bounded_text(run_id, 256) {
            return Err(OrchestratorStartError::Rejected);
        }
        let run_id = run_id.to_string();
        let cancellation = CancellationToken::new();
        let queued = status_value(
            &run_id,
            &request.schedule_id,
            BriefRunState::Queued,
            &[],
            None,
        )
        .map_err(|_| OrchestratorStartError::Rejected)?;
        {
            let mut runs = self
                .inner
                .runs
                .lock()
                .map_err(|_| OrchestratorStartError::Rejected)?;
            if runs.contains_key(&run_id) {
                return Ok(run_id);
            }
            if runs.len() >= MAX_TRACKED_RUNS {
                let removable = runs
                    .iter()
                    .find(|(_, record)| is_terminal(record.state))
                    .map(|(id, _)| id.clone());
                if let Some(removable) = removable {
                    runs.remove(&removable);
                } else {
                    return Err(OrchestratorStartError::AdmissionUnavailable);
                }
            }
            runs.insert(
                run_id.clone(),
                RunRecord {
                    cancellation: cancellation.clone(),
                    state: BriefRunState::Queued,
                    history: VecDeque::from([queued]),
                    result: None,
                },
            );
        }
        let orchestrator = self.clone();
        let spawned_run_id = run_id.clone();
        tokio::spawn(async move {
            orchestrator
                .run(spawned_run_id, request, cancellation)
                .await;
        });
        Ok(run_id)
    }
}
