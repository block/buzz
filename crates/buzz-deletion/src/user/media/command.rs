//! Standalone preview: read-only source sessions, separate journal connection.
use super::*;
use buzz_db::{Db, DbConfig};
use serde_json::Value;

/// Direct command arguments. The mode is bound to the run and cannot change on retry.
#[derive(Debug, clap::Args)]
pub struct Options {
    /// Human public key (hex or npub); demo input is not accepted.
    #[arg(long,value_parser=buzz_db::user_deletion::normalize_user_deletion_pubkey)]
    pub pubkey: String,
    /// Stable UUID for retries. Omit to begin a fresh observation.
    #[arg(long)]
    pub run_id: Option<Uuid>,
    /// Include agents currently owned by this person, separately in each community.
    #[arg(long)]
    pub include_owned_agents: bool,
    /// Current direct execution supports print-only. Other modes require a protected workflow caller.
    #[arg(long, value_enum, default_value = "print-only")]
    pub mode: Mode,
    /// Additional canonical media base URLs (including /media) for deployment aliases.
    #[arg(long)]
    pub media_base: Vec<String>,
    /// Maximum source records to inspect; exceeding it publishes no inventory.
    #[arg(long, default_value_t = 1_000_000)]
    pub max_source_rows: usize,
    /// Per-statement timeout for the read-only source scan.
    #[arg(long, default_value_t = 120)]
    pub scan_timeout_secs: u64,
}

/// Run the direct command. It never migrates either DB, fences subjects, mutates
/// source tables, removes uploader proofs, or calls storage deletion.
pub async fn run(options: Options) -> Result<i32> {
    if options.mode != Mode::PrintOnly {
        bail!("direct execution requires print-only; destructive modes require a workflow ProtectedMedia provider with durable writer reservations");
    }
    if options.max_source_rows == 0 || options.scan_timeout_secs == 0 {
        bail!("media scan budgets must be positive");
    }
    let budget = buzz_db::media_deletion::MediaScanBudget {
        max_source_rows: options.max_source_rows,
        statement_timeout_secs: options.scan_timeout_secs,
    };
    let source_url = crate::required_env("BUZZ_MEDIA_SOURCE_DATABASE_URL")?;
    let journal_url = crate::required_env("DATABASE_URL")?;
    let endpoint = crate::required_env("BUZZ_S3_ENDPOINT")?;
    let bucket = crate::required_env("BUZZ_S3_BUCKET")?;
    let namespace =
        buzz_db::entity_deletion::inventory_digest(&json!({"endpoint":endpoint,"bucket":bucket}))?;
    let source_identity = buzz_db::entity_deletion::inventory_digest(&json!(source_url))?;
    let run_id = options.run_id.unwrap_or_else(Uuid::new_v4);
    eprintln!("media inventory run {run_id}; retry with --run-id {run_id}");
    let journal = Db::connect_cold_start(
        DbConfig {
            database_url: journal_url,
            ..DbConfig::default()
        },
        "media_journal",
    )
    .await?;
    let store = journal.media_deletion_store();
    let binding = json!({"version":1,"mode":"print-only","human":options.pubkey,"include_owned_agents":options.include_owned_agents,"source":source_identity,"namespace":namespace,"media_bases":options.media_base,"scan_budget":budget});
    let run = store.open_preview(run_id, binding).await?;
    if store.load(&run).await?.is_none() {
        let source = Db::connect_cold_start(
            DbConfig {
                database_url: source_url,
                max_connections: 1,
                min_connections: 0,
                default_transaction_read_only: true,
                ..DbConfig::default()
            },
            "media_source",
        )
        .await?;
        let mut scan = source
            .scan_user_media_with_budget(
                &options.pubkey,
                options.include_owned_agents,
                &namespace,
                &options.media_base,
                budget,
            )
            .await?;
        let (s3_access_key, s3_secret_key) = crate::s3_key_pair_from_env();
        let media = MediaStorage::new(&buzz_media::MediaConfig {
            s3_endpoint: endpoint,
            s3_bucket: bucket,
            s3_access_key,
            s3_secret_key,
            s3_region: crate::s3_region_from_env(),
            s3_addressing_style: std::env::var("BUZZ_S3_ADDRESSING_STYLE")
                .unwrap_or_else(|_| "path".into())
                .parse()
                .map_err(anyhow::Error::msg)?,
            max_image_bytes: 1,
            max_gif_bytes: 1,
            max_video_bytes: 1,
            max_file_bytes: 1,
            public_base_url: "https://example.org/media".into(),
            upload_records_enabled: false,
            upload_ip_header: None,
            upload_port_header: None,
        })?;
        storage::supplement(&mut scan, &media, &namespace).await?;
        store
            .publish(
                &run,
                &scan.files,
                json!({"tables":scan.scanned,"scope":scan.scope,"unresolved":scan.unresolved,"database_snapshot_at":scan.database_snapshot_at,"storage_listing_finished_at":chrono::Utc::now()}),
            )
            .await?;
    }
    let files = store
        .load(&run)
        .await?
        .context("discovery did not publish")?;
    for file in files.into_iter().filter(|f| f.status == "unknown") {
        store
            .save_preview_decision(&run, &classify_preview(file)?)
            .await?;
    }
    let files = store.load(&run).await?.context("discovery missing")?;
    let unknown = files.iter().filter(|f| f.status == "unknown").count();
    let discovery = store.receipt(&run).await?;
    let output: Value = json!({"discovery":discovery,"run_id":run_id,"mode":"print-only","discovery_complete":true,"candidate_count":files.len(),"decision_complete":unknown==0,"execution_authorized":false,"unknown_count":unknown,"files":files});
    crate::print_json(&output)?;
    Ok(if unknown == 0 { 0 } else { 2 })
}
