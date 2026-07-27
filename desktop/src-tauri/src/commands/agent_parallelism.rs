/// Validate an optional managed-instance worker-count update before any
/// record is mutated or persisted.
pub(super) fn validate_parallelism_update(parallelism: Option<u32>) -> Result<(), String> {
    if parallelism.is_some_and(|count| !(1..=32).contains(&count)) {
        return Err("parallelism must be between 1 and 32".to_string());
    }
    Ok(())
}
