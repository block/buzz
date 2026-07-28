use super::*;

#[test]
fn extracts_nested_and_legacy_ui_resource_uris() {
    assert_eq!(
        ui_resource_uri(&json!({"ui": {"resourceUri": "ui://board"}})).as_deref(),
        Some("ui://board")
    );
    assert_eq!(
        ui_resource_uri(&json!({"ui/resourceUri": "ui://legacy"})).as_deref(),
        Some("ui://legacy")
    );
    assert!(ui_resource_uri(&json!({"ui": {"resourceUri": "https://bad"}})).is_none());
}

#[test]
fn app_visibility_defaults_to_model_and_app() {
    let default = tool_visibility(&json!({}));
    assert_eq!(default, vec!["model", "app"]);
    let model_only = tool_visibility(&json!({"ui": {"visibility": ["model"]}}));
    assert_eq!(model_only, vec!["model"]);
}

#[test]
fn caller_visibility_is_enforced() {
    let tool = McpAppTool {
        name: "private".to_string(),
        title: None,
        description: None,
        input_schema: json!({}),
        output_schema: None,
        annotations: None,
        meta: json!({}),
        ui_resource_uri: None,
        visibility: vec!["app".to_string()],
    };
    assert!(app_tool_allowed(&tool, McpAppToolCaller::App));
    assert!(!app_tool_allowed(&tool, McpAppToolCaller::Host));
}

#[test]
fn endpoint_policy_allows_https_and_loopback_http() {
    assert!(validate_mcp_endpoint("https://apps.example.com/mcp").is_ok());
    assert!(validate_mcp_endpoint("http://127.0.0.1:1337/mcp").is_ok());
    assert!(validate_mcp_endpoint("http://apps.example.com/mcp").is_err());
    assert!(validate_mcp_endpoint("https://user:secret@apps.example.com/mcp").is_err());
}

#[test]
fn rejects_private_and_transitional_network_addresses() {
    for address in [
        "10.0.0.1",
        "100.64.0.1",
        "192.168.1.1",
        "::ffff:127.0.0.1",
        "::ffff:169.254.169.254",
        "64:ff9b::7f00:1",
        "2002:7f00:1::",
        "2001::1",
        "ff02::1",
    ] {
        assert!(
            is_private_ip(address.parse().unwrap()),
            "{address} must be rejected"
        );
    }
    assert!(!is_private_ip("2606:4700:4700::1111".parse().unwrap()));
}

#[test]
fn csp_drops_invalid_sources_and_defaults_closed() {
    let csp = sandbox_csp(&McpAppResourceCsp {
        connect_domains: vec![
            "https://api.example.com".to_string(),
            "https://example.com/path".to_string(),
        ],
        resource_domains: Vec::new(),
        frame_domains: Vec::new(),
        base_uri_domains: Vec::new(),
    });
    assert!(csp.contains("connect-src https://api.example.com"));
    assert!(!csp.contains("https://example.com/path"));
    assert!(csp.contains("script-src 'self' 'unsafe-inline'"));
    assert!(csp.contains("img-src 'self' data: blob:"));
    assert!(csp.contains("frame-src 'self'"));
    assert!(!csp.contains("frame-src 'self' buzz-mcp-app:"));
    assert!(csp.contains("object-src 'none'"));
}

#[test]
fn inner_app_frame_uses_an_opaque_origin_and_fixed_sandbox() {
    assert!(SANDBOX_PROXY_HTML.contains(r#"inner.setAttribute("sandbox", "allow-scripts")"#));
    assert!(SANDBOX_PROXY_HTML.contains("inner.srcdoc = html"));
    assert!(!SANDBOX_PROXY_HTML.contains("allow-same-origin allow-forms"));
    assert!(!SANDBOX_PROXY_HTML.contains("inner.setAttribute(\"sandbox\", sandbox)"));
}

#[test]
fn parses_text_ui_resource_and_metadata() {
    let response = json!({
        "result": {
            "contents": [{
                "uri": "ui://board",
                "mimeType": MCP_APP_MIME_TYPE,
                "text": "<main>Board</main>",
                "_meta": {
                    "ui": {
                        "csp": {"connectDomains": ["https://api.example.com"]},
                        "permissions": {"clipboardWrite": {}}
                    }
                }
            }]
        }
    });
    let (html, csp, permissions) = parse_ui_resource(&response, "ui://board", None).unwrap();
    assert_eq!(html, "<main>Board</main>");
    assert_eq!(csp.connect_domains, vec!["https://api.example.com"]);
    assert!(permissions.clipboard_write.is_some());
}

#[test]
fn rejects_ui_resource_uri_mismatch() {
    let response = json!({
        "result": {
            "contents": [{
                "uri": "ui://other",
                "mimeType": MCP_APP_MIME_TYPE,
                "text": "<main>Other</main>"
            }]
        }
    });
    assert!(parse_ui_resource(&response, "ui://board", None).is_err());
}
