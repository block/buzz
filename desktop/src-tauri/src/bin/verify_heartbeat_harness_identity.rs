#[path = "../managed_agents/binary_identity.rs"]
mod binary_identity;

fn main() -> Result<(), String> {
    let path = std::env::args_os()
        .nth(1)
        .ok_or_else(|| "usage: verify-heartbeat-harness-identity <signed-buzz-acp>".to_string())?;
    let expected = option_env!("BUZZ_DESKTOP_BUNDLED_BUZZ_ACP_SHA256")
        .ok_or_else(|| "this build has no bundled buzz-acp identity pin".to_string())?;
    let bytes = std::fs::read(&path)
        .map_err(|error| format!("cannot read signed heartbeat harness: {error}"))?;
    let actual = binary_identity::executable_identity_sha256(&bytes)?;
    if actual != expected {
        return Err(format!(
            "signed heartbeat harness identity mismatch: expected {expected}, got {actual}"
        ));
    }
    println!("signed heartbeat harness matches the Desktop build pin");
    Ok(())
}
