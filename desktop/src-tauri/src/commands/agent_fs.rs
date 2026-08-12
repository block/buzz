use crate::managed_agents::{
    BackendKind, CreateManagedAgentRequest, FilesystemIsolationProfile, ManagedAgentRecord,
};

pub(super) fn validate_create(input: &CreateManagedAgentRequest) -> Result<(), String> {
    if let Some(parallelism) = input.parallelism {
        if !(1..=32).contains(&parallelism) {
            return Err("parallelism must be between 1 and 32".to_string());
        }
    }
    crate::managed_agents::validate_user_env_keys(&input.env_vars)?;
    if let Some(profile) = &input.filesystem_isolation {
        crate::managed_agents::validate_filesystem_isolation_profile(profile)?;
        if input.backend != BackendKind::Local {
            return Err(
                "filesystem isolation is available only for local managed agents".to_string(),
            );
        }
    }
    Ok(())
}

pub(super) fn apply_update(
    record: &mut ManagedAgentRecord,
    filesystem_isolation: Option<Option<FilesystemIsolationProfile>>,
) -> Result<(), String> {
    let Some(filesystem_isolation) = filesystem_isolation else {
        return Ok(());
    };
    if let Some(profile) = &filesystem_isolation {
        if record.backend != BackendKind::Local {
            return Err(
                "filesystem isolation is available only for local managed agents".to_string(),
            );
        }
        crate::managed_agents::validate_filesystem_isolation_profile(profile)?;
    }
    record.filesystem_isolation = filesystem_isolation;
    Ok(())
}
