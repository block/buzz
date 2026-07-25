use super::*;

impl CommandBriefOrchestrator {
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
            return Err(OrchestratorStartError);
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
        .map_err(|_| OrchestratorStartError)?;
        {
            let mut runs = self.inner.runs.lock().map_err(|_| OrchestratorStartError)?;
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
                    return Err(OrchestratorStartError);
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
