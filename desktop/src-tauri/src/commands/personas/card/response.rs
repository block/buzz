//! Parsing for the Responses API image-generation output.
//!
//! The API can return a successful top-level response even when every hosted
//! image tool call failed. Keep those failures readable without echoing the
//! full payload (which can contain large image results and model reasoning).

const MAX_FAILURE_DETAIL_CHARS: usize = 600;

/// Extract the generated image (base64) and any designer text from a
/// Responses API payload. Pure for testability.
pub(crate) fn extract_card_output(resp: &serde_json::Value) -> Result<(String, String), String> {
    let output = resp
        .get("output")
        .and_then(|o| o.as_array())
        .ok_or_else(|| "OpenAI returned a response without an output array.".to_string())?;

    let mut image_b64 = None;
    let mut notes = Vec::new();
    let mut call_statuses = Vec::new();
    let mut call_errors = Vec::new();

    for item in output {
        match item.get("type").and_then(|t| t.as_str()) {
            Some("image_generation_call") => {
                if let Some(status) = item.get("status").and_then(|s| s.as_str()) {
                    call_statuses.push(status.to_string());
                }
                if let Some(result) = item
                    .get("result")
                    .and_then(|r| r.as_str())
                    .filter(|r| !r.is_empty())
                {
                    image_b64 = Some(result.to_string());
                }
                if let Some(error) = item.get("error") {
                    if let Some(message) = error.get("message").and_then(|m| m.as_str()) {
                        call_errors.push(message.to_string());
                    } else if let Some(code) = error.get("code").and_then(|c| c.as_str()) {
                        call_errors.push(code.to_string());
                    }
                }
            }
            Some("message") => collect_message_text(item, &mut notes),
            _ => {}
        }
    }

    if let Some(image_b64) = image_b64 {
        return Ok((image_b64, notes.join("\n")));
    }

    let detail = if !notes.is_empty() {
        Some(notes.join(" "))
    } else if !call_errors.is_empty() {
        Some(call_errors.join(" "))
    } else {
        resp.get("incomplete_details")
            .and_then(|details| details.get("reason"))
            .and_then(|reason| reason.as_str())
            .map(str::to_string)
    };

    let mut message = "OpenAI did not return a card image".to_string();
    if let Some(detail) = detail {
        let detail = compact_and_truncate(&detail, MAX_FAILURE_DETAIL_CHARS);
        if !detail.is_empty() {
            message.push_str(": ");
            message.push_str(&detail);
        }
    } else if !call_statuses.is_empty() {
        message.push_str(&format!(
            " (image generation statuses: {})",
            call_statuses.join(", ")
        ));
    } else {
        message.push_str(". Try again, or revise the card description if the problem repeats");
    }
    if !message.ends_with(['.', '!', '?']) {
        message.push('.');
    }
    Err(message)
}

fn collect_message_text(item: &serde_json::Value, notes: &mut Vec<String>) {
    let Some(content) = item.get("content").and_then(|c| c.as_array()) else {
        return;
    };
    for part in content {
        match part.get("type").and_then(|t| t.as_str()) {
            Some("output_text") => {
                if let Some(text) = part.get("text").and_then(|t| t.as_str()) {
                    notes.push(text.to_string());
                }
            }
            Some("refusal") => {
                if let Some(text) = part.get("refusal").and_then(|t| t.as_str()) {
                    notes.push(text.to_string());
                }
            }
            _ => {}
        }
    }
}

fn compact_and_truncate(value: &str, max_chars: usize) -> String {
    let compact = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if compact.chars().count() <= max_chars {
        return compact;
    }
    let mut truncated = compact.chars().take(max_chars).collect::<String>();
    truncated.push('…');
    truncated
}
