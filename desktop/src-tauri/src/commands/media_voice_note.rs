use tokio_util::sync::CancellationToken;

use super::media_transcode::transcode_voice_note_to_mp4_with_cancellation;

const VOICE_NOTE_MAX_INPUT_BYTES: usize = 128 * 1024 * 1024;

pub(super) fn is_voice_note_filename(filename: Option<&str>) -> bool {
    filename.is_some_and(|name| {
        let lower = name.to_ascii_lowercase();
        lower.starts_with("voice-note-") && lower.ends_with(".wav")
    })
}

pub(super) fn voice_note_mp4_filename(filename: &str) -> String {
    filename
        .strip_suffix(".wav")
        .or_else(|| filename.strip_suffix(".WAV"))
        .map_or_else(|| format!("{filename}.mp4"), |stem| format!("{stem}.mp4"))
}

pub(super) async fn prepare_voice_note_for_upload(
    data: Vec<u8>,
    cancellation: Option<&CancellationToken>,
) -> Result<(Vec<u8>, Option<Vec<u8>>), String> {
    validate_voice_note_input_size(data.len())?;
    let cancellation = cancellation.cloned();
    tokio::task::spawn_blocking(move || {
        let detected = infer::get(&data)
            .ok_or_else(|| "Voice note has an unrecognized audio format.".to_string())?;
        if !detected.mime_type().starts_with("audio/") {
            return Err("Voice note upload did not contain audio.".to_string());
        }

        let tmp_input =
            std::env::temp_dir().join(format!("buzz-voice-input-{}", uuid::Uuid::new_v4()));
        let result = (|| {
            std::fs::write(&tmp_input, &data)
                .map_err(|error| format!("failed to prepare voice note: {error}"))?;
            let output =
                transcode_voice_note_to_mp4_with_cancellation(&tmp_input, cancellation.as_ref())?;
            let bytes = std::fs::read(&output)
                .map_err(|error| format!("failed to read prepared voice note: {error}"));
            let _ = std::fs::remove_file(&output);
            bytes.map(|bytes| (bytes, None))
        })();
        let _ = std::fs::remove_file(&tmp_input);
        result
    })
    .await
    .map_err(|error| format!("voice note task failed: {error}"))?
}

fn validate_voice_note_input_size(size: usize) -> Result<(), String> {
    if size > VOICE_NOTE_MAX_INPUT_BYTES {
        return Err(format!(
            "Voice note exceeds the maximum input size of {VOICE_NOTE_MAX_INPUT_BYTES} bytes."
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        is_voice_note_filename, validate_voice_note_input_size, voice_note_mp4_filename,
        VOICE_NOTE_MAX_INPUT_BYTES,
    };

    #[test]
    fn voice_note_filenames_are_scoped_and_rewritten_for_video_upload() {
        assert!(is_voice_note_filename(Some("voice-note-123.wav")));
        assert!(!is_voice_note_filename(Some("meeting.wav")));
        assert!(!is_voice_note_filename(Some("voice-note-123.mp4")));
        assert_eq!(
            voice_note_mp4_filename("voice-note-123.wav"),
            "voice-note-123.mp4"
        );
    }

    #[test]
    fn voice_note_input_size_is_bounded_before_transcoding() {
        assert!(validate_voice_note_input_size(VOICE_NOTE_MAX_INPUT_BYTES).is_ok());
        assert!(validate_voice_note_input_size(VOICE_NOTE_MAX_INPUT_BYTES + 1).is_err());
    }
}
