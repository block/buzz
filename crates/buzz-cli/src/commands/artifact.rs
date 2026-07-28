use std::path::PathBuf;

use crate::client::{normalize_artifact_sha256, BuzzClient};
use crate::error::CliError;

pub(crate) async fn dispatch(cmd: crate::ArtifactCmd, client: &BuzzClient) -> Result<(), CliError> {
    match cmd {
        crate::ArtifactCmd::Put { file } => {
            let metadata = std::fs::metadata(&file)
                .map_err(|e| CliError::Usage(format!("cannot access {}: {e}", file.display())))?;
            if !metadata.is_file() {
                return Err(CliError::Usage(format!("{} is not a file", file.display())));
            }
            let body = std::fs::read(&file)
                .map_err(|e| CliError::Usage(format!("failed to read {}: {e}", file.display())))?;
            let receipt = client.put_artifact(bytes::Bytes::from(body)).await?;
            println!(
                "{}",
                serde_json::to_string_pretty(&receipt)
                    .map_err(|e| CliError::Other(format!("receipt serialization failed: {e}")))?
            );
            Ok(())
        }
        crate::ArtifactCmd::Get { sha256, out } => {
            let sha256 = normalize_artifact_sha256(&sha256)?;
            let out = out.unwrap_or_else(|| PathBuf::from(&sha256));
            let bytes = client.get_artifact(&sha256).await?;
            std::fs::write(&out, &bytes)
                .map_err(|e| CliError::Other(format!("could not write {}: {e}", out.display())))?;
            println!(
                "saved artifact {sha256} to {} ({} bytes)",
                out.display(),
                bytes.len()
            );
            Ok(())
        }
        crate::ArtifactCmd::Head { sha256 } => {
            let sha256 = normalize_artifact_sha256(&sha256)?;
            if client.head_artifact(&sha256).await? {
                println!("present {sha256}");
                Ok(())
            } else {
                println!("absent {sha256}");
                Err(CliError::NotFound(format!("artifact {sha256} is absent")))
            }
        }
    }
}
