const ALLOWED_PREVIEW_MIME: &[&str] = &[
    "image/jpeg",
    "image/png",
    "image/gif",
    "image/webp",
    "video/mp4",
];
const MAX_DOCUMENT_BYTES: usize = 10 * 1024 * 1024;

/// Sanitize a filename for use as a display label in the imeta `filename` field.
///
/// Strips any directory components (keeps only the final path segment), removes
/// control characters, and bounds length to 255. Mirrors the relay's filename
/// validation so a sanitized name always passes ingest. Returns a fallback when
/// the result would be empty.
pub(crate) fn sanitize_filename(name: &str) -> String {
    // Keep only the final path segment — defend against `../` and absolute paths
    // regardless of separator style.
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name).trim();
    let preserve_calendar_extension = base.to_ascii_lowercase().ends_with(".ics");
    let source = if preserve_calendar_extension {
        &base[..base.len() - ".ics".len()]
    } else {
        base
    };
    let byte_limit = if preserve_calendar_extension {
        255 - ".ics".len()
    } else {
        255
    };
    let mut cleaned = String::new();
    for character in source.chars().filter(|character| !character.is_control()) {
        if cleaned.len() + character.len_utf8() > byte_limit {
            break;
        }
        cleaned.push(character);
    }
    let cleaned = cleaned.trim();
    if preserve_calendar_extension {
        format!(
            "{}.ics",
            if cleaned.is_empty() {
                "calendar"
            } else {
                cleaned
            }
        )
    } else if cleaned.is_empty() {
        "file".to_string()
    } else {
        cleaned.to_string()
    }
}

pub(crate) fn detect_and_validate_mime(
    body: &[u8],
    filename: Option<&str>,
) -> Result<String, String> {
    let is_calendar = filename.is_some_and(|name| {
        std::path::Path::new(name)
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("ics"))
    });
    if is_calendar {
        if body.len() > MAX_DOCUMENT_BYTES {
            return Err(format!(
                "calendar file is too large: {} bytes (max {MAX_DOCUMENT_BYTES})",
                body.len()
            ));
        }
        let text = std::str::from_utf8(body)
            .map_err(|_| "invalid calendar file: expected UTF-8 text".to_string())?;
        if text.as_bytes().contains(&0) {
            return Err("invalid calendar file: NUL bytes are not allowed".to_string());
        }
        let mut lines = text.lines().map(str::trim).filter(|line| !line.is_empty());
        let first = lines.next().unwrap_or_default();
        let last = lines.last().unwrap_or(first);
        if !first.eq_ignore_ascii_case("BEGIN:VCALENDAR")
            || !last.eq_ignore_ascii_case("END:VCALENDAR")
        {
            return Err("invalid calendar file: missing VCALENDAR envelope".to_string());
        }
        return Ok("text/calendar".to_string());
    }

    let mime = infer::get(body)
        .map(|kind| kind.mime_type().to_string())
        .unwrap_or_else(|| "application/octet-stream".to_string());
    if !ALLOWED_PREVIEW_MIME.contains(&mime.as_str()) {
        return Err(format!("unsupported file type: {mime}"));
    }
    Ok(mime)
}

#[cfg(test)]
mod tests {
    use super::detect_and_validate_mime;

    #[test]
    fn detects_jpeg() {
        let jpeg = [0xFF, 0xD8, 0xFF, 0xE0];
        assert_eq!(detect_and_validate_mime(&jpeg, None).unwrap(), "image/jpeg");
    }

    #[test]
    fn rejects_arbitrary_text() {
        assert!(detect_and_validate_mime(b"hello world", None).is_err());
    }

    #[test]
    fn accepts_calendar_by_extension_and_envelope() {
        let calendar = b"BEGIN:VCALENDAR\r\nVERSION:2.0\r\nEND:VCALENDAR\r\n";
        assert_eq!(
            detect_and_validate_mime(calendar, Some("Planning.ics")).unwrap(),
            "text/calendar"
        );
    }

    #[test]
    fn rejects_html() {
        let html = b"<!DOCTYPE html><html><body><script>alert(1)</script></body></html>";
        assert!(detect_and_validate_mime(html, Some("calendar.ics")).is_err());
        assert!(detect_and_validate_mime(html, Some("page.html")).is_err());
    }

    #[test]
    fn rejects_executable() {
        let elf = [b"\x7fELF".as_slice(), &[0u8; 60]].concat();
        assert!(detect_and_validate_mime(&elf, None).is_err());
    }
}
