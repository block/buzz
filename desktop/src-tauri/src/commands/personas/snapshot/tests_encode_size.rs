//! Export-size guard tests for `validate_snapshot_encode_size`.
//!
//! Kept in a sibling file so `snapshot/tests.rs` stays under the
//! 1000-line gate; `#[path]`-included from there as a child module,
//! so `super::*` still resolves to the shared test imports.
//!
//! Tests call `validate_snapshot_encode_size` directly so they prove the
//! exact production guard — not a manual reconstruction.  Removing or
//! reversing the check in production code will cause these tests to fail.

use super::*;

/// JSON: boundary-1 passes, boundary is the last legal byte count.
#[test]
fn validate_encode_size_json_at_boundary_minus_1_passes() {
    assert!(validate_snapshot_encode_size(MAX_SNAPSHOT_JSON_BYTES - 1, false).is_ok());
}

/// JSON: exactly at the boundary is the last accepted size.
#[test]
fn validate_encode_size_json_at_boundary_passes() {
    assert!(validate_snapshot_encode_size(MAX_SNAPSHOT_JSON_BYTES, false).is_ok());
}

/// JSON: boundary+1 is rejected.
#[test]
fn validate_encode_size_json_over_boundary_is_rejected() {
    let err = validate_snapshot_encode_size(MAX_SNAPSHOT_JSON_BYTES + 1, false).unwrap_err();
    assert!(
        err.contains("size limit"),
        "error must mention size limit, got: {err}"
    );
}

/// PNG: boundary-1 passes.
#[test]
fn validate_encode_size_png_at_boundary_minus_1_passes() {
    assert!(validate_snapshot_encode_size(MAX_SNAPSHOT_PNG_BYTES - 1, true).is_ok());
}

/// PNG: exactly at the boundary passes.
#[test]
fn validate_encode_size_png_at_boundary_passes() {
    assert!(validate_snapshot_encode_size(MAX_SNAPSHOT_PNG_BYTES, true).is_ok());
}

/// PNG: boundary+1 is rejected.
#[test]
fn validate_encode_size_png_over_boundary_is_rejected() {
    let err = validate_snapshot_encode_size(MAX_SNAPSHOT_PNG_BYTES + 1, true).unwrap_err();
    assert!(
        err.contains("size limit"),
        "error must mention size limit, got: {err}"
    );
}
