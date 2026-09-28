//! Local certificate verification for protected agents and their CLI helpers.
use std::path::Path;
/// Optional launcher-owned certificate snapshot path.
pub const ROOTS_ENV: &str = "BUZZ_SANDBOX_TLS_ROOTS";
/// Use ordinary platform trust unless an explicit launcher snapshot is configured.
pub fn http_builder() -> Result<reqwest::ClientBuilder, anyhow::Error> {
    let builder = reqwest::Client::builder();
    match std::env::var_os(ROOTS_ENV) {
        None => Ok(builder),
        Some(path) => builder_with_roots(builder, Path::new(&path)),
    }
}
/// Restrict verification to a nonempty DER certificate snapshot; invalid input fails closed.
pub fn builder_with_roots(
    builder: reqwest::ClientBuilder,
    path: &Path,
) -> Result<reqwest::ClientBuilder, anyhow::Error> {
    let fail = || anyhow::anyhow!("invalid sandbox TLS trust snapshot");
    let bytes = std::fs::read(path).map_err(|_| fail())?;
    if bytes.len() > 8 * 1024 * 1024 {
        return Err(fail());
    }
    let roots: Vec<Vec<u8>> = serde_json::from_slice(&bytes).map_err(|_| fail())?;
    if roots.is_empty() {
        return Err(fail());
    }
    let certs = roots
        .iter()
        .map(|der| reqwest::Certificate::from_der(der))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| fail())?;
    Ok(builder.tls_certs_only(certs))
}
