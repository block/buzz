#[test]
fn ui() {
    let tests = trybuild::TestCases::new();
    tests.compile_fail("tests/ui/missing_marker.rs");
    tests.compile_fail("tests/ui/duplicate_marker.rs");
    tests.compile_fail("tests/ui/empty_reason.rs");
    tests.compile_fail("tests/ui/malformed_marker.rs");
    tests.pass("tests/ui/valid_markers.rs");
}
