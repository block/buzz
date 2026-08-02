use super::manifest::Artifact;
use futures_util::StreamExt;
use sha2::{Digest, Sha256};
#[cfg(test)]
use std::io::Read;
use std::io::Write;
use std::time::Duration;

const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(60);

pub(crate) async fn fetch_verified_artifact_to<W: Write>(
    artifact: &Artifact,
    output: W,
) -> Result<(), String> {
    validate_authorized_url(artifact)?;
    fetch_verified_to(
        &artifact.url,
        output,
        artifact.archive_size,
        &artifact.archive_sha256,
    )
    .await
}

fn validate_authorized_url(artifact: &Artifact) -> Result<(), String> {
    let url = reqwest::Url::parse(&artifact.url).map_err(|_| "Guardian artifact URL is invalid")?;
    let expected_path = format!(
        "/perplexityai/numbat/releases/download/v0.1.2/{}",
        artifact.asset_name
    );
    if url.scheme() != "https"
        || url.host_str() != Some("github.com")
        || url.port().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path() != expected_path
    {
        return Err("Guardian artifact URL is outside the compiled allowlist".into());
    }
    Ok(())
}

async fn fetch_verified_to<W: Write>(
    url: &str,
    output: W,
    expected_size: u64,
    expected_sha256: &str,
) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            let allowed = attempt.url().scheme() == "https"
                && matches!(
                    attempt.url().host_str(),
                    Some("github.com")
                        | Some("objects.githubusercontent.com")
                        | Some("release-assets.githubusercontent.com")
                );
            if attempt.previous().len() >= 5 {
                attempt.error("Guardian download exceeded redirect limit")
            } else if allowed {
                attempt.follow()
            } else {
                attempt.error("Guardian download redirect left the allowlist")
            }
        }))
        .timeout(DOWNLOAD_TIMEOUT)
        .build()
        .map_err(|e| format!("failed to build Guardian download client: {e}"))?;
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("Guardian download failed: {e}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "Guardian download returned HTTP {}",
            response.status()
        ));
    }
    if response
        .content_length()
        .is_some_and(|length| length != expected_size)
    {
        return Err("Guardian download Content-Length mismatch".into());
    }
    verify_response_stream(response, output, expected_size, expected_sha256).await
}

async fn verify_response_stream<W: Write>(
    response: reqwest::Response,
    mut output: W,
    expected_size: u64,
    expected_sha256: &str,
) -> Result<(), String> {
    let mut stream = response.bytes_stream();
    let mut hash = Sha256::new();
    let mut total = 0u64;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("download read failed: {e}"))?;
        total = total
            .checked_add(chunk.len() as u64)
            .ok_or("download size overflow")?;
        if total > expected_size {
            return Err("download exceeded authorized size".into());
        }
        hash.update(&chunk);
        output
            .write_all(&chunk)
            .map_err(|e| format!("staging write failed: {e}"))?;
    }
    finish_verification(output, hash, total, expected_size, expected_sha256)
}

#[cfg(test)]
pub(crate) fn verify_stream<R: Read, W: Write>(
    mut input: R,
    mut output: W,
    expected_size: u64,
    expected_sha256: &str,
) -> Result<(), String> {
    let mut hash = Sha256::new();
    let mut total = 0u64;
    let mut buf = [0u8; 64 * 1024];
    loop {
        let count = input
            .read(&mut buf)
            .map_err(|e| format!("download read failed: {e}"))?;
        if count == 0 {
            break;
        }
        total = total
            .checked_add(count as u64)
            .ok_or("download size overflow")?;
        if total > expected_size {
            return Err("download exceeded authorized size".into());
        }
        hash.update(&buf[..count]);
        output
            .write_all(&buf[..count])
            .map_err(|e| format!("staging write failed: {e}"))?;
    }
    finish_verification(output, hash, total, expected_size, expected_sha256)
}

fn finish_verification<W: Write>(
    mut output: W,
    hash: Sha256,
    total: u64,
    expected_size: u64,
    expected_sha256: &str,
) -> Result<(), String> {
    if total != expected_size {
        return Err(format!(
            "download size mismatch: expected {expected_size}, got {total}"
        ));
    }
    let actual = hex::encode(hash.finalize());
    if actual != expected_sha256 {
        return Err("download digest mismatch".into());
    }
    output
        .flush()
        .map_err(|e| format!("staging flush failed: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::guardian_distribution::manifest::ArchiveKind;
    use axum::{body::Body, response::Redirect, routing::get, Router};

    async fn fixture_server(router: Router) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        format!("http://{address}")
    }

    fn artifact_with_url(url: &str) -> Artifact {
        Artifact {
            os: "linux".into(),
            arch: "amd64".into(),
            archive_kind: ArchiveKind::TarGz,
            asset_name: "numbat_0.1.2_linux_amd64.tar.gz".into(),
            url: url.into(),
            archive_sha256: "0".repeat(64),
            archive_size: 1,
            expanded_size: 1,
            binary_path: "numbat".into(),
            binary_sha256: "0".repeat(64),
            binary_size: 1,
        }
    }

    #[test]
    fn artifact_download_url_is_compiled_allowlist_only() {
        let allowed = artifact_with_url(
            "https://github.com/perplexityai/numbat/releases/download/v0.1.2/numbat_0.1.2_linux_amd64.tar.gz",
        );
        assert!(validate_authorized_url(&allowed).is_ok());
        for url in [
            "http://github.com/perplexityai/numbat/releases/download/v0.1.2/numbat_0.1.2_linux_amd64.tar.gz",
            "https://example.com/perplexityai/numbat/releases/download/v0.1.2/numbat_0.1.2_linux_amd64.tar.gz",
            "https://github.com/perplexityai/numbat/releases/download/v0.1.2/other.tar.gz",
            "https://github.com/perplexityai/numbat/releases/download/v0.1.2/numbat_0.1.2_linux_amd64.tar.gz?override=1",
        ] {
            assert!(validate_authorized_url(&artifact_with_url(url)).is_err(), "accepted {url}");
        }
    }
    #[test]
    fn streams_and_checks_size_and_digest() {
        let mut out = Vec::new();
        verify_stream(
            &b"fixture"[..],
            &mut out,
            7,
            "f16d05ec6b29248d2c61adb1e9263f78e4f7bace1b955014a2d17872cfe4064d",
        )
        .unwrap();
        assert_eq!(out, b"fixture");
        assert!(verify_stream(&b"fixture"[..], Vec::new(), 6, "00").is_err());
    }

    #[tokio::test]
    async fn local_fixture_streams_exact_authorized_bytes() {
        let base =
            fixture_server(Router::new().route("/asset", get(|| async { Body::from("fixture") })))
                .await;
        let mut output = Vec::new();
        fetch_verified_to(
            &format!("{base}/asset"),
            &mut output,
            7,
            "f16d05ec6b29248d2c61adb1e9263f78e4f7bace1b955014a2d17872cfe4064d",
        )
        .await
        .unwrap();
        assert_eq!(output, b"fixture");
    }

    #[tokio::test]
    async fn local_fixture_rejects_redirects_and_false_lengths() {
        let base = fixture_server(
            Router::new()
                .route("/redirect", get(|| async { Redirect::temporary("/asset") }))
                .route("/wrong-length", get(|| async { Body::from("fixture!") })),
        )
        .await;
        let digest = "f16d05ec6b29248d2c61adb1e9263f78e4f7bace1b955014a2d17872cfe4064d";
        assert!(
            fetch_verified_to(&format!("{base}/redirect"), Vec::new(), 7, digest)
                .await
                .unwrap_err()
                .contains("redirect")
        );
        assert!(
            fetch_verified_to(&format!("{base}/wrong-length"), Vec::new(), 7, digest)
                .await
                .unwrap_err()
                .contains("Content-Length")
        );
    }
}
