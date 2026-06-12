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
