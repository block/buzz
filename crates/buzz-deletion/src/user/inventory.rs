//! Inventory preview and shared subject parsing for preview and submission.
pub(super) fn parse_subject(input: &str) -> std::result::Result<String, String> {
    if input == "demo" {
        return Ok(input.to_owned());
    }
    buzz_db::user_deletion::normalize_user_deletion_pubkey(input).map_err(|error| error.to_string())
}
