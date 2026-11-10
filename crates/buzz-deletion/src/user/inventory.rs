//! Inventory preview and shared subject parsing for preview and submission.
use anyhow::Result;

pub(super) async fn generate(db: &buzz_db::Db, pubkey: &str) -> Result<serde_json::Value> {
    Ok(db.user_deletion_store().generate_inventory(pubkey).await?)
}

pub(super) fn parse_subject(input: &str) -> std::result::Result<String, String> {
    if input == "demo" {
        return Ok(input.to_owned());
    }
    buzz_db::user_deletion::normalize_user_deletion_pubkey(input).map_err(|error| error.to_string())
}
