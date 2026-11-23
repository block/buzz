//! The current workflow stage selects one handler. Its mode controls source changes.
use super::{media, summary, worker::RunOptions};
use anyhow::{bail, ensure, Context, Result};
use buzz_db::entity_deletion::{EntityLease, EntityRequest, EntityStage};
use buzz_db::user_deletion::{
    account_batches::AccountBatchOptions, content_batches::ContentBatchOptions,
    profile_batches::ProfileBatchOptions, ExecutionMode, UserDeletionInventory,
};
use buzz_db::Db;
use serde_json::{json, Value};
use uuid::Uuid;

/// A persisted invocation has a leased stage. Stateless previews have no checkpoint.
pub(super) struct Handler<'a> {
    pub db: &'a Db,
    pub request: &'a EntityRequest,
    pub mode: ExecutionMode,
    pub options: &'a RunOptions,
    pub current: Option<(&'a EntityLease, &'a EntityStage)>,
}

impl Handler<'_> {
    pub async fn run(&self, stage: &str, community: Option<Uuid>) -> Result<Value> {
        let subject = UserDeletionInventory::subject(self.request)?;
        let mode = self.mode;
        let options = self.options;
        let delete = mode == ExecutionMode::Delete;
        let saved_preview = !delete && self.current.is_some();
        // Only destructive handlers need lease checks inside source transactions.
        let checkpoint = self
            .current
            .filter(|_| delete)
            .map(|(lease, row)| (lease, row.id));
        let users = self.db.user_deletion_store();
        let mut stderr = std::io::stderr();
        let mut sink = std::io::sink();
        let mut output: &mut (dyn std::io::Write + Send) = if saved_preview {
            &mut sink
        } else {
            &mut stderr
        };
        let mut value = match stage {
            "gate_user" | "global_cleanup" if !delete => json!({
                "simulated":true,"stage":stage,"source_data_unchanged":true,
                "detail":"dummy rehearsal stage; no gate or global cleanup performed"
            }),
            "purge_media" => {
                let configured;
                let storage = match options.media.as_deref() {
                    Some(storage) => storage,
                    None => {
                        configured = media::configured_storage()?;
                        &configured
                    }
                };
                if saved_preview {
                    media::stage_preview(self.db, storage, subject).await?
                } else {
                    media::handle_with_storage(self.db, storage, subject, delete, &mut output)
                        .await?
                }
            }
            "purge_content" => {
                let summary = users
                    .run_message_batches(
                        &ContentBatchOptions {
                            community: community.context("missing content community")?,
                            pubkey: subject.into(),
                            relay_pubkey: options.relay_pubkey.clone(),
                            batch_size: options.batch_size,
                            max_items: options.max_items,
                        },
                        mode,
                        checkpoint,
                        &mut output,
                    )
                    .await?;
                json!({"content": summary, "account_erasure_complete": false})
            }
            "purge_account" => {
                let summary = super::account_batches::store(self.db, mode)?
                    .run_account_batches(
                        &AccountBatchOptions {
                            community: community.context("missing account community")?,
                            pubkey: subject.into(),
                            batch_size: options.batch_size,
                            max_items: options.max_items,
                        },
                        mode,
                        &mut output,
                        checkpoint,
                    )
                    .await?;
                ensure!(
                    summary.supported_scope_exhausted,
                    "account batch limit reached; increase --max-items and retry"
                );
                serde_json::to_value(summary)?
            }
            "purge_profile" if !delete => {
                let summary = users
                    .preview_profile_plan(
                        &ProfileBatchOptions {
                            community: community.context("missing profile community")?,
                            pubkey: subject.into(),
                            relay_pubkeys: options.relay_pubkey.iter().cloned().collect(),
                            batch_size: options.batch_size,
                            max_items: options.max_items,
                        },
                        &mut output,
                    )
                    .await?;
                serde_json::to_value(summary)?
            }
            "delete_owned_community" => {
                if let Some((_, row)) = self.current {
                    let storage;
                    let media = match options.media.as_deref() {
                        Some(media) => media,
                        None => {
                            storage = media::configured_storage()?;
                            &storage
                        }
                    };
                    let communities = options
                        .community_store
                        .as_ref()
                        .context("missing child journal")?;
                    super::owned_community::run_stage(self.request, row, communities, media).await?
                } else {
                    json!({"community":community, "would_submit_or_adopt_child":true, "child_submitted":false})
                }
            }
            "remaining_cleanup" if !delete => {
                summary::report(&self.db.entity_deletion_store(), self.request).await?
            }
            _ => bail!("real handler {stage} is not implemented for this execution mode"),
        };
        if saved_preview {
            value["execution_mode"] = "rehearsal".into();
            value["account_erasure_complete"] = false.into();
        }
        Ok(value)
    }
}

pub(super) async fn demo(row: &EntityStage, options: &RunOptions) -> Result<serde_json::Value> {
    let message = match row.stage.as_str() {
        "gate_user" => "would gate user",
        "purge_media" => "would purge user media",
        "purge_content" => "would purge user content",
        "purge_agents" => "would purge user agents",
        name => bail!("unknown demo stage {name}"),
    };
    let inputs = &row.metadata_json["inputs"];
    let target = inputs["target"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("missing target input"))?;
    eprintln!(
        "DUMMY request={} stage={} community={:?} target={target}: {message}",
        row.request_id, row.stage, row.target_community_id
    );
    tokio::time::sleep(options.stage_delay).await;
    if options.fail_stage.as_deref() == Some(&row.stage) {
        bail!("injected dummy failure for {}", row.stage);
    }
    let mut result = serde_json::json!({"dummy": true, "message": message});
    if row.stage == "purge_content" {
        let community = inputs["community"]["community_id"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing community input"))?
            .parse::<Uuid>()?;
        if Some(community) != row.target_community_id {
            bail!("community input does not match stage");
        }
        // A simulated observation, persisted before routing. Restart uses this
        // result even if the next invocation supplies different demo options.
        result["has_agents"] = (!options.no_agents_in.contains(&community)).into();
    }
    Ok(result)
}
