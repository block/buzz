use super::*;
use serde_json::{json, Value};

fn valid_manifest_json() -> Value {
    json!({
        "packageFormatVersion": "0.1.0-alpha",
        "contractVersion": "0.1.0-alpha",
        "id": "dev.example.browser",
        "name": "Example Browser",
        "version": "0.1.0",
        "publisher": "Example Inc.",
        "license": "Apache-2.0",
        "runtime": {
            "type": "stdio",
            "targets": {
                "aarch64-apple-darwin": {
                    "path": "bin/aarch64-apple-darwin",
                    "sha256": "a".repeat(64),
                    "bytes": 1024,
                }
            }
        },
        "grants": ["browser.browse"],
        "contributions": [{
            "kind": "browser",
            "id": "web",
            "title": "Web",
        }],
    })
}

fn parse(value: Value) -> Result<Manifest, InstallReject> {
    parse_and_validate(&serde_json::to_vec(&value).expect("serialize manifest fixture"))
}

#[test]
fn accepts_a_valid_manifest() {
    let result = parse(valid_manifest_json());
    assert!(result.is_ok(), "valid manifest should parse: {result:?}");
}

#[test]
fn rejects_unknown_top_level_field() {
    let mut manifest = valid_manifest_json();
    manifest
        .as_object_mut()
        .unwrap()
        .insert("unknownField".into(), json!(true));
    assert!(matches!(
        parse(manifest),
        Err(InstallReject::InvalidManifest(_))
    ));
}

#[test]
fn rejects_unknown_nested_field() {
    let mut manifest = valid_manifest_json();
    manifest["runtime"]
        .as_object_mut()
        .unwrap()
        .insert("unknownNested".into(), json!(true));
    assert!(matches!(
        parse(manifest),
        Err(InstallReject::InvalidManifest(_))
    ));
}

#[test]
fn rejects_wrong_package_format_version() {
    let mut manifest = valid_manifest_json();
    manifest["packageFormatVersion"] = json!("0.2.0");
    assert!(matches!(
        parse(manifest),
        Err(InstallReject::InvalidManifest(_))
    ));
}

#[test]
fn rejects_wrong_contract_version() {
    let mut manifest = valid_manifest_json();
    manifest["contractVersion"] = json!("2026-01-01");
    assert!(matches!(
        parse(manifest),
        Err(InstallReject::InvalidManifest(_))
    ));
}

#[test]
fn rejects_empty_id() {
    let mut manifest = valid_manifest_json();
    manifest["id"] = json!("");
    assert!(matches!(
        parse(manifest),
        Err(InstallReject::InvalidManifest(_))
    ));
}

#[test]
fn rejects_single_label_id() {
    let mut manifest = valid_manifest_json();
    manifest["id"] = json!("noreversedomain");
    assert!(matches!(
        parse(manifest),
        Err(InstallReject::InvalidManifest(_))
    ));
}

#[test]
fn rejects_id_with_path_traversal_characters() {
    let mut manifest = valid_manifest_json();
    manifest["id"] = json!("dev.example/../escape");
    assert!(matches!(
        parse(manifest),
        Err(InstallReject::InvalidManifest(_))
    ));
}

#[test]
fn rejects_id_with_uppercase() {
    let mut manifest = valid_manifest_json();
    manifest["id"] = json!("Dev.Example.Browser");
    assert!(matches!(
        parse(manifest),
        Err(InstallReject::InvalidManifest(_))
    ));
}

#[test]
fn rejects_non_three_component_version() {
    let mut manifest = valid_manifest_json();
    manifest["version"] = json!("1.0");
    assert!(matches!(
        parse(manifest),
        Err(InstallReject::InvalidManifest(_))
    ));
}

#[test]
fn rejects_leading_zero_version_component() {
    let mut manifest = valid_manifest_json();
    manifest["version"] = json!("01.2.3");
    assert!(matches!(
        parse(manifest),
        Err(InstallReject::InvalidManifest(_))
    ));
}

#[test]
fn rejects_unicode_digits_in_version() {
    // The `regex` crate's `\d` shorthand matches any Unicode decimal digit,
    // not just ASCII 0-9; SemVer itself only ever means ASCII digits.
    let mut manifest = valid_manifest_json();
    manifest["version"] = json!("\u{0661}.2.3"); // U+0661 ARABIC-INDIC DIGIT ONE
    assert!(matches!(
        parse(manifest),
        Err(InstallReject::InvalidManifest(_))
    ));
}

#[test]
fn accepts_valid_prerelease_and_build_version() {
    let mut manifest = valid_manifest_json();
    manifest["version"] = json!("1.0.0-alpha.1+build.5");
    assert!(parse(manifest).is_ok());
}

#[test]
fn rejects_name_over_the_length_limit() {
    let mut manifest = valid_manifest_json();
    manifest["name"] = json!("x".repeat(65));
    assert!(matches!(
        parse(manifest),
        Err(InstallReject::InvalidManifest(_))
    ));
}

#[test]
fn rejects_publisher_over_the_length_limit() {
    let mut manifest = valid_manifest_json();
    manifest["publisher"] = json!("x".repeat(65));
    assert!(matches!(
        parse(manifest),
        Err(InstallReject::InvalidManifest(_))
    ));
}

#[test]
fn rejects_contribution_title_over_the_length_limit() {
    let mut manifest = valid_manifest_json();
    manifest["contributions"][0]["title"] = json!("x".repeat(49));
    assert!(matches!(
        parse(manifest),
        Err(InstallReject::InvalidManifest(_))
    ));
}

#[test]
fn rejects_contribution_id_with_uppercase() {
    let mut manifest = valid_manifest_json();
    manifest["contributions"][0]["id"] = json!("Web");
    assert!(matches!(
        parse(manifest),
        Err(InstallReject::InvalidManifest(_))
    ));
}

#[test]
fn rejects_contribution_id_over_the_length_limit() {
    let mut manifest = valid_manifest_json();
    manifest["contributions"][0]["id"] = json!("w".repeat(33));
    assert!(matches!(
        parse(manifest),
        Err(InstallReject::InvalidManifest(_))
    ));
}

#[test]
fn rejects_license_with_disallowed_characters() {
    let mut manifest = valid_manifest_json();
    manifest["license"] = json!("Apache-2.0 OR <script>");
    assert!(matches!(
        parse(manifest),
        Err(InstallReject::InvalidManifest(_))
    ));
}

#[test]
fn rejects_a_compound_spdx_expression() {
    // `license` is a single SPDX identifier per `docs/plugin-browser-prototype.md` §2, not a
    // full SPDX license *expression* — `AND`/`OR`/`WITH` and parentheses are
    // not part of a single identifier's grammar.
    let mut manifest = valid_manifest_json();
    manifest["license"] = json!("Apache-2.0 OR MIT");
    assert!(matches!(
        parse(manifest),
        Err(InstallReject::InvalidManifest(_))
    ));
}

#[test]
fn accepts_a_spdx_identifier_with_a_trailing_plus() {
    let mut manifest = valid_manifest_json();
    manifest["license"] = json!("GPL-2.0+");
    assert!(parse(manifest).is_ok());
}

#[test]
fn accepts_a_spdx_identifier_with_an_or_later_suffix() {
    let mut manifest = valid_manifest_json();
    manifest["license"] = json!("GPL-3.0-or-later");
    assert!(parse(manifest).is_ok());
}

#[test]
fn rejects_a_fabricated_license_identifier() {
    // Grammar-valid (letters, digits, hyphens) but not a registered SPDX id.
    let mut manifest = valid_manifest_json();
    manifest["license"] = json!("Not-A-License");
    assert!(matches!(
        parse(manifest),
        Err(InstallReject::InvalidManifest(_))
    ));
}

#[test]
fn accepts_a_registered_spdx_identifier() {
    let mut manifest = valid_manifest_json();
    manifest["license"] = json!("MIT");
    assert!(parse(manifest).is_ok());
}

#[test]
fn rejects_empty_license() {
    let mut manifest = valid_manifest_json();
    manifest["license"] = json!("");
    assert!(matches!(
        parse(manifest),
        Err(InstallReject::InvalidManifest(_))
    ));
}

#[test]
fn rejects_non_https_homepage() {
    let mut manifest = valid_manifest_json();
    manifest["homepage"] = json!("http://example.com");
    assert!(matches!(
        parse(manifest),
        Err(InstallReject::InvalidManifest(_))
    ));
}

#[test]
fn accepts_a_valid_https_homepage() {
    let mut manifest = valid_manifest_json();
    manifest["homepage"] = json!("https://example.com/plugin");
    assert!(parse(manifest).is_ok());
}

#[test]
fn rejects_a_malformed_homepage_url() {
    let mut manifest = valid_manifest_json();
    manifest["homepage"] = json!("not a url");
    assert!(matches!(
        parse(manifest),
        Err(InstallReject::InvalidManifest(_))
    ));
}

#[test]
fn rejects_a_homepage_url_with_no_host() {
    let mut manifest = valid_manifest_json();
    manifest["homepage"] = json!("https://");
    assert!(matches!(
        parse(manifest),
        Err(InstallReject::InvalidManifest(_))
    ));
}

#[test]
fn rejects_wrong_runtime_type() {
    let mut manifest = valid_manifest_json();
    manifest["runtime"]["type"] = json!("http");
    assert!(matches!(
        parse(manifest),
        Err(InstallReject::InvalidManifest(_))
    ));
}

#[test]
fn rejects_empty_targets() {
    let mut manifest = valid_manifest_json();
    manifest["runtime"]["targets"] = json!({});
    assert!(matches!(
        parse(manifest),
        Err(InstallReject::InvalidManifest(_))
    ));
}

#[test]
fn rejects_absolute_target_path() {
    let mut manifest = valid_manifest_json();
    manifest["runtime"]["targets"]["aarch64-apple-darwin"]["path"] = json!("/etc/passwd");
    assert!(matches!(
        parse(manifest),
        Err(InstallReject::InvalidManifest(_))
    ));
}

#[test]
fn rejects_parent_segment_in_target_path() {
    let mut manifest = valid_manifest_json();
    manifest["runtime"]["targets"]["aarch64-apple-darwin"]["path"] = json!("../escape/bin");
    assert!(matches!(
        parse(manifest),
        Err(InstallReject::InvalidManifest(_))
    ));
}

#[test]
fn rejects_non_hex_digest() {
    let mut manifest = valid_manifest_json();
    manifest["runtime"]["targets"]["aarch64-apple-darwin"]["sha256"] = json!("not-hex");
    assert!(matches!(
        parse(manifest),
        Err(InstallReject::InvalidManifest(_))
    ));
}

#[test]
fn rejects_multiple_grants() {
    let mut manifest = valid_manifest_json();
    manifest["grants"] = json!(["browser.browse", "extra.grant"]);
    assert!(matches!(
        parse(manifest),
        Err(InstallReject::InvalidManifest(_))
    ));
}

#[test]
fn rejects_unknown_grant() {
    let mut manifest = valid_manifest_json();
    manifest["grants"] = json!(["some.other.grant"]);
    assert!(matches!(
        parse(manifest),
        Err(InstallReject::InvalidManifest(_))
    ));
}

#[test]
fn rejects_multiple_contributions() {
    let mut manifest = valid_manifest_json();
    manifest["contributions"] = json!([
        {"kind": "browser", "id": "web", "title": "Web"},
        {"kind": "browser", "id": "web2", "title": "Web 2"},
    ]);
    assert!(matches!(
        parse(manifest),
        Err(InstallReject::InvalidManifest(_))
    ));
}

#[test]
fn rejects_unknown_contribution_kind() {
    let mut manifest = valid_manifest_json();
    manifest["contributions"][0]["kind"] = json!("terminal");
    assert!(matches!(
        parse(manifest),
        Err(InstallReject::InvalidManifest(_))
    ));
}

#[test]
fn rejects_malformed_json() {
    let result = parse_and_validate(b"{ not json");
    assert!(matches!(result, Err(InstallReject::InvalidManifest(_))));
}
