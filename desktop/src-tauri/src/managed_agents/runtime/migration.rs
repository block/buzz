use super::*;

/// Clear migration-only scalar PIDs loaded from pre-schema-v2 records.
///
/// Authenticated runtime adoption and status probes happen outside state locks.
/// This helper performs no liveness inference and emits no process signal.
pub fn clear_legacy_runtime_pids(records: &mut [ManagedAgentRecord]) -> bool {
    let mut changed = false;
    for record in records.iter_mut() {
        if record.runtime_pid.take().is_some() {
            record.updated_at = now_iso();
            changed = true;
        }
    }
    changed
}
