use super::*;
use std::{io, time::Duration};

pub(super) struct RecordWriter<F> {
    line: Vec<u8>,
    pub callback: F,
}

impl<F> RecordWriter<F> {
    pub fn new(callback: F) -> Self {
        Self {
            line: Vec::new(),
            callback,
        }
    }
}

impl<F: FnMut(&Value) -> io::Result<()>> Write for RecordWriter<F> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.line.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        let record: Value = serde_json::from_slice(&self.line)?;
        self.line.clear();
        (self.callback)(&record)
    }
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn output_failure_before_mutation_rolls_back_but_after_commit_is_durable() {
    let f = Fixture::new().await;
    let before = f.snapshot().await;
    for record_type in ["account_batches_start", "proposed_batch"] {
        let mut writer = RecordWriter::new(|r: &Value| {
            if r["record"] == record_type {
                return Err(io::Error::other("injected output failure"));
            }
            Ok(())
        });
        let err = f.run(ExecutionMode::Delete, &mut writer).await.unwrap_err();
        assert!(err.to_string().contains("injected output failure"));
        assert_eq!(f.snapshot().await, before);
    }
    let mut writer = RecordWriter::new(|r: &Value| {
        if r["record"] == "batch_committed" {
            return Err(io::Error::other("lost receipt"));
        }
        Ok(())
    });
    assert!(f
        .run(ExecutionMode::Delete, &mut writer)
        .await
        .unwrap_err()
        .to_string()
        .contains("lost receipt"));
    let after = f.snapshot().await;
    assert_eq!(
        after["api_tokens"].as_array().unwrap().len(),
        before["api_tokens"].as_array().unwrap().len() - 2
    );
    for table in TABLES.iter().filter(|t| **t != "api_tokens") {
        assert_eq!(after[*table], before[*table], "{table}");
    }
    assert_eq!(
        f.run(ExecutionMode::Delete, &mut Vec::new())
            .await
            .unwrap()
            .items,
        33
    );
    f.assert_cleaned(&before, &[f.options.community]).await;
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn cancellation_after_commit_preserves_batch_and_can_retry() {
    let f = Fixture::new().await;
    let before = f.snapshot().await;
    let mut blocker = f.pool.begin().await.unwrap();
    // Hold the third token, so batch one commits and batch two cannot complete.
    // OFFSET with FOR UPDATE also locks skipped rows; identify the third row
    // first so the barrier cannot block the first batch.
    let third: Uuid = sqlx::query_scalar("SELECT id FROM api_tokens WHERE community_id=$1 AND owner_pubkey=decode($2,'hex') ORDER BY id OFFSET 2 LIMIT 1")
        .bind(f.options.community).bind(TARGET).fetch_one(&f.pool).await.unwrap();
    sqlx::query("SELECT id FROM api_tokens WHERE community_id=$1 AND id=$2 FOR UPDATE")
        .bind(f.options.community)
        .bind(third)
        .fetch_one(&mut *blocker)
        .await
        .unwrap();
    let (committed_tx, committed_rx) = tokio::sync::oneshot::channel();
    let store = f.store();
    let community = f.options.community;
    let task = tokio::spawn(async move {
        let mut sent = Some(committed_tx);
        let mut writer = RecordWriter::new(move |r: &Value| {
            if r["record"] == "batch_committed" {
                if let Some(tx) = sent.take() {
                    let _ = tx.send(());
                }
            }
            Ok(())
        });
        let options = AccountBatchOptions {
            community,
            pubkey: TARGET.into(),
            batch_size: 2,
            max_items: 1000,
        };
        store
            .run_account_batches(&options, ExecutionMode::Delete, &mut writer, None)
            .await
    });
    let reached = tokio::time::timeout(Duration::from_secs(10), committed_rx).await;
    task.abort();
    let stopped = task.await;
    blocker.rollback().await.unwrap();
    reached.unwrap().unwrap();
    assert!(stopped.unwrap_err().is_cancelled());
    let after = f.snapshot().await;
    assert_eq!(
        after["api_tokens"].as_array().unwrap().len(),
        before["api_tokens"].as_array().unwrap().len() - 2
    );
    assert_eq!(
        f.run(ExecutionMode::Delete, &mut Vec::new())
            .await
            .unwrap()
            .items,
        33
    );
    f.assert_cleaned(&before, &[f.options.community]).await;
}
