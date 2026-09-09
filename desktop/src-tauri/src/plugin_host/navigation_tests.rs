use super::*;

#[test]
fn admits_ordinary_https() {
    assert!(is_admitted("https://example.com/docs"));
}

#[test]
fn admits_ordinary_http() {
    assert!(is_admitted("http://example.com/"));
}

#[test]
fn admits_explicit_port() {
    assert!(is_admitted("https://example.com:8443/"));
}

#[test]
fn admits_userinfo() {
    assert!(is_admitted("https://user:pass@example.com/"));
}

#[test]
fn admits_ipv6_literal() {
    assert!(is_admitted("https://[2001:db8::1]/"));
}

#[test]
fn admits_percent_encoded_host() {
    assert!(is_admitted("https://xn--nxasmq6b.example/"));
}

#[test]
fn admits_localhost_on_an_ordinary_port() {
    assert!(is_admitted("http://localhost:3000/"));
}

#[test]
fn denies_localhost_on_the_tauri_dev_port() {
    assert!(!is_admitted("http://localhost:1420/"));
}

#[test]
fn denies_ipc_localhost_at_any_port() {
    assert!(!is_admitted("http://ipc.localhost/"));
    assert!(!is_admitted("http://ipc.localhost:9999/"));
}

#[test]
fn denies_buzz_media_localhost() {
    assert!(!is_admitted("https://buzz-media.localhost/"));
}

#[test]
fn denies_tauri_localhost() {
    assert!(!is_admitted("https://tauri.localhost/"));
}

#[test]
fn denies_about_scheme() {
    assert!(!is_admitted("about:blank"));
}

#[test]
fn denies_file_scheme() {
    assert!(!is_admitted("file:///etc/passwd"));
}

#[test]
fn denies_data_scheme() {
    assert!(!is_admitted("data:text/plain,hello"));
}

#[test]
fn denies_javascript_scheme() {
    assert!(!is_admitted("javascript:alert(1)"));
}

#[test]
fn denies_tauri_scheme() {
    assert!(!is_admitted("tauri://localhost/"));
}

#[test]
fn denies_buzz_media_scheme() {
    assert!(!is_admitted("buzz-media://asset"));
}

#[test]
fn denies_asset_scheme() {
    assert!(!is_admitted("asset://local"));
}

#[test]
fn denies_empty_and_whitespace_addresses() {
    assert!(!is_admitted(""));
    assert!(!is_admitted("   "));
}

#[test]
fn denies_unparseable_addresses() {
    assert!(!is_admitted("not a url"));
}

#[test]
fn admits_an_address_at_exactly_the_length_cap() {
    let padding = "a".repeat(2048 - "https://example.com/".len());
    let address = format!("https://example.com/{padding}");
    assert_eq!(address.len(), 2048);
    assert!(is_admitted(&address));
}

#[test]
fn denies_an_address_one_over_the_length_cap() {
    let padding = "a".repeat(2048 - "https://example.com/".len() + 1);
    let address = format!("https://example.com/{padding}");
    assert_eq!(address.len(), 2049);
    assert!(!is_admitted(&address));
}
