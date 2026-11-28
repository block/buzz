/// Sanitize a filename for use as a display label in the imeta `filename` field.
///
/// Strips directory components, removes control characters, and bounds length
/// to 255 UTF-8 bytes (the relay's limit, not 255 characters) so the resulting
/// name always passes relay ingest validation.
pub(crate) fn sanitize_filename(name: &str) -> String {
    const MAX_BYTES: usize = 255;
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name).trim();
    let mut cleaned = String::new();
    for c in base.chars().filter(|c| !c.is_control()) {
        if cleaned.len() + c.len_utf8() > MAX_BYTES {
            break;
        }
        cleaned.push(c);
    }
    if cleaned.is_empty() {
        "file".to_string()
    } else {
        cleaned
    }
}

#[cfg(test)]
mod tests {
    use super::sanitize_filename;

    #[test]
    fn strips_paths_controls_and_empty_names() {
        assert_eq!(sanitize_filename("report.pdf"), "report.pdf");
        assert_eq!(sanitize_filename("../../etc/passwd"), "passwd");
        assert_eq!(sanitize_filename("/abs/path/notes.txt"), "notes.txt");
        assert_eq!(sanitize_filename(r"C:\Users\me\doc.docx"), "doc.docx");
        assert_eq!(sanitize_filename(""), "file");
        assert_eq!(sanitize_filename("/"), "file");
        assert_eq!(sanitize_filename("a\nb\tc.txt"), "abc.txt");
    }

    /// The relay rejects an imeta `filename` over 255 bytes, so the bound is
    /// in UTF-8 bytes, not characters: 106 CJK characters are 310 bytes.
    #[test]
    fn bounds_non_ascii_names_to_255_bytes() {
        let name = format!("{}.pdf", "日本語".repeat(34));
        let sanitized = sanitize_filename(&name);
        assert!(sanitized.len() <= 255, "{} bytes", sanitized.len());
        assert_eq!(sanitized, "日本語".repeat(28) + "日");

        assert_eq!(sanitize_filename(&"a".repeat(300)), "a".repeat(255));
        let short = "отчёт за квартал.pdf";
        assert_eq!(sanitize_filename(short), short);
    }
}
