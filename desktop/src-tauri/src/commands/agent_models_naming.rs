//! OpenAI model-id → display-name helpers for `get_agent_models`, split out
//! of `agent_models.rs` to keep it under the desktop file-size ratchet. Wired
//! via `#[path = "agent_models_naming.rs"] mod naming;` and used through it.

pub(super) fn is_agent_text_model_id(id: &str) -> bool {
    let lower = id.to_ascii_lowercase();
    if [
        "audio",
        "dall-e",
        "embedding",
        "image",
        "moderation",
        "realtime",
        "speech",
        "transcribe",
        "tts",
        "whisper",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
    {
        return false;
    }

    lower.starts_with("gpt-") || lower.starts_with('o') || lower.starts_with("chatgpt-")
}

pub(super) fn openai_dated_snapshot_alias(id: &str) -> Option<String> {
    let (base, date) = id.rsplit_once('-')?;
    if date.len() != 2 || !date.chars().all(|character| character.is_ascii_digit()) {
        return None;
    }
    let (base, month) = base.rsplit_once('-')?;
    if month.len() != 2 || !month.chars().all(|character| character.is_ascii_digit()) {
        return None;
    }
    let (base, year) = base.rsplit_once('-')?;
    if year.len() != 4 || !year.chars().all(|character| character.is_ascii_digit()) {
        return None;
    }

    Some(base.to_string())
}

pub(super) fn openai_model_display_name(id: &str) -> String {
    let canonical = openai_dated_snapshot_alias(id).unwrap_or_else(|| id.to_string());
    if let Some(rest) = canonical.strip_prefix("chatgpt-") {
        return format!("ChatGPT {}", title_case_model_suffix(rest));
    }
    if let Some(rest) = canonical.strip_prefix("gpt-") {
        return format!("GPT-{}", title_case_model_suffix(rest));
    }

    canonical
}

pub(super) fn title_case_model_suffix(value: &str) -> String {
    value
        .split('-')
        .enumerate()
        .map(|(index, part)| {
            let part = if part.eq_ignore_ascii_case("pro") {
                "Pro".to_string()
            } else if part.eq_ignore_ascii_case("mini") {
                "mini".to_string()
            } else if part.eq_ignore_ascii_case("nano") {
                "nano".to_string()
            } else {
                part.to_string()
            };

            if index == 0 {
                part
            } else {
                format!(" {part}")
            }
        })
        .collect::<String>()
}
