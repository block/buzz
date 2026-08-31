//! Stamp the actual local launcher, never saved provider routing or ambient env.
pub(super) fn apply(
    command: &mut std::process::Command,
    owner: Option<&str>,
) -> Result<(), String> {
    command
        .env_remove("BUZZ_ACP_HOST_PUBKEY")
        .env_remove("BUZZ_ACP_HOST_LABEL");
    let Some(owner) = owner else {
        return Ok(());
    };
    let location = crate::commands::local_launch_location(owner)?;
    command
        .env("BUZZ_ACP_HOST_PUBKEY", location.host)
        .env("BUZZ_ACP_HOST_LABEL", location.label);
    Ok(())
}
