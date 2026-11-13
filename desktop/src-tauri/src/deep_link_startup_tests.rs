#[test]
fn install_handlers_registers_configured_schemes_before_listening() {
    let source = include_str!("deep_link.rs");
    let setup_start = source
        .find("pub(crate) fn install_deep_link_handlers")
        .expect("deep-link setup function");
    let setup_end = source[setup_start..]
        .find("\n}\n")
        .map(|offset| setup_start + offset)
        .expect("end of deep-link setup function");
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
    // Both deep-link entry points -- the runtime `on_open_url` listener and the
    // Windows/Linux cold-start `get_current` replay -- must route through the
    // validated handler. A new source that bypasses it should fail this.
    assert_eq!(
        setup.matches("handle_deep_link_url(").count(),
        2,
        "every deep-link URL source must use the validated handler"
    );
}
