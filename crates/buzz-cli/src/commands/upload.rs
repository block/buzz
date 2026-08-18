use crate::client::BuzzClient;
use crate::error::CliError;

fn standalone_upload_output(
    descriptor: &crate::client::BlobDescriptor,
) -> Result<serde_json::Value, CliError> {
    let mut output =
        serde_json::to_value(descriptor).map_err(|error| CliError::Other(error.to_string()))?;
    let object = output
        .as_object_mut()
        .ok_or_else(|| CliError::Other("upload result was not an object".to_string()))?;
    let raw_label = descriptor.filename.as_deref().unwrap_or("file");
    let mut label = String::with_capacity(raw_label.len());
    for character in raw_label.chars() {
        if matches!(character, '\\' | '[' | ']') {
            label.push('\\');
        }
        label.push(character);
    }
    object.insert(
        "attachment_markdown".to_string(),
        serde_json::Value::String(format!("[{label}]({})", descriptor.url)),
    );
    object.insert(
        "delivery_hint".to_string(),
        serde_json::Value::String(
            "Do not send the bare url to a Buzz channel. Retry with `buzz messages send --channel <UUID> --content \"attached\" --file <path>` so the original filename and preview metadata are included."
                .to_string(),
        ),
    );
    Ok(output)
}

pub async fn dispatch(cmd: crate::UploadCmd, client: &BuzzClient) -> Result<(), CliError> {
    match cmd {
        crate::UploadCmd::File { file } => {
            let desc = client.upload_file(&file).await?;
            println!(
                "{}",
                serde_json::to_string_pretty(&standalone_upload_output(&desc)?)
                    .map_err(|e| CliError::Other(e.to_string()))?
            );
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::standalone_upload_output;

    #[test]
    fn standalone_upload_warns_against_bare_urls_and_preserves_filename_link() {
        let descriptor = crate::client::BlobDescriptor {
            url: "https://relay.test/media/abc.bin".to_string(),
            sha256: "a".repeat(64),
            size: 12,
            mime_type: "application/octet-stream".to_string(),
            uploaded: 0,
            dim: None,
            blurhash: None,
            thumb: None,
            duration: None,
            filename: Some("notes[final].md".to_string()),
        };

        let output = standalone_upload_output(&descriptor).expect("serialize upload output");
        assert_eq!(
            output["attachment_markdown"],
            "[notes\\[final\\].md](https://relay.test/media/abc.bin)"
        );
        assert!(output["delivery_hint"]
            .as_str()
            .is_some_and(|hint| hint.contains("messages send") && hint.contains("--file")));
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
