use crate::client::BuzzClient;
use crate::error::CliError;

pub async fn dispatch(cmd: crate::UploadCmd, client: &BuzzClient) -> Result<(), CliError> {
    match cmd {
        crate::UploadCmd::File { file } => {
            let desc = client.upload_file(&file).await?;
            println!(
                "{}",
                serde_json::to_string_pretty(&desc).map_err(|e| CliError::Other(e.to_string()))?
            );
            Ok(())
        }
    }
}

pub async fn dispatch_media(cmd: crate::MediaCmd, client: &BuzzClient) -> Result<(), CliError> {
    match cmd {
        crate::MediaCmd::Get { input, output } => {
            let bytes = client.download_media(&input).await?;
            match output.as_deref() {
                Some(path) if path != "-" => {
                    std::fs::write(path, &bytes)
                        .map_err(|e| CliError::Other(format!("could not write {path}: {e}")))?;
                }
                _ => {
                    use std::io::Write;
                    std::io::stdout()
                        .write_all(&bytes)
                        .map_err(|e| CliError::Other(format!("could not write stdout: {e}")))?;
                }
            }
            Ok(())
        }
    }
}
