#[test]
fn desktop_setup_registers_configured_deep_links_before_listening() {
    let source = include_str!("lib.rs");
    let setup_start = source
        .find("// Handle deep link URLs received while the app is running")
        .expect("deep-link setup block");
    let setup_end = source[setup_start..]
        .find("// Defer launch-time agent restoration")
        .map(|offset| setup_start + offset)
        .expect("end of deep-link setup block");
    let setup = &source[setup_start..setup_end];

    let register = setup
        .find(".register_all()")
        .expect("configured desktop schemes must be registered at startup");
    let subscribe = setup
        .find(".on_open_url(")
        .expect("runtime deep-link listener");
    assert!(
        register < subscribe,
        "scheme registration must happen before the runtime listener is attached"
    );
    assert_eq!(
        setup.matches("handle_deep_link_url(").count(),
        1,
        "runtime URLs must continue to use the validated handler"
    );
}
