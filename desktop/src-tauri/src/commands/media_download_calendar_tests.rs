use super::validate_downloaded_file;

#[test]
fn accepts_valid_named_calendar() {
    let calendar = b"BEGIN:VCALENDAR\r\nVERSION:2.0\r\nEND:VCALENDAR\r\n";
    assert_eq!(
        validate_downloaded_file(calendar, "Planning.ics").unwrap(),
        "text/calendar"
    );
}

#[test]
fn rejects_malformed_or_active_calendar_payloads() {
    let malformed = b"BEGIN:VCALENDAR\r\nVERSION:2.0\r\n";
    let html = b"<!DOCTYPE html><html><script>alert(1)</script></html>";
    let executable = [b"\x7fELF".as_slice(), &[0u8; 60]].concat();

    for payload in [malformed.as_slice(), html.as_slice(), executable.as_slice()] {
        assert!(validate_downloaded_file(payload, "Planning.ics").is_err());
    }
}
