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
        param_headers: Vec::new(),
    };
    assert!(app_tool_allowed(&tool, McpAppToolCaller::App));
    assert!(!app_tool_allowed(&tool, McpAppToolCaller::Host));
}

#[test]
fn endpoint_policy_allows_https_and_loopback_http() {
    assert!(validate_mcp_endpoint("https://apps.example.com/mcp").is_ok());
    assert!(validate_mcp_endpoint("https://localhost:1337/mcp").is_ok());
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
    assert!(csp.contains("base-uri 'self'"));
}

#[test]
fn csp_rejects_bare_wildcards_and_csp_delimiters() {
    for source in [
        "https://*",
        "wss://*",
        "https://evil.example;x",
        "https://-bad.example",
        "https://bad-.example",
    ] {
        assert!(csp_origin(source).is_none(), "{source} must be rejected");
    }
    assert_eq!(
        csp_origin("https://*.example.com").as_deref(),
        Some("https://*.example.com")
    );
    assert_eq!(
        csp_origin("https://api.example.com").as_deref(),
        Some("https://api.example.com")
    );
    assert_eq!(
        csp_origin("http://[::1]:1337").as_deref(),
        Some("http://[::1]:1337")
    );
}

#[test]
fn reviewed_policy_allows_only_equal_or_narrower_capabilities() {
    let approved = McpAppResourcePolicy {
        csp: McpAppResourceCsp {
            connect_domains: vec![
                "https://api.example.com".to_string(),
                "https://events.example.com".to_string(),
            ],
            resource_domains: vec!["https://cdn.example.com".to_string()],
            ..Default::default()
        },
        requested_permissions: McpAppResourcePermissions {
            clipboard_write: Some(json!({})),
            ..Default::default()
        },
    };
    let narrower = McpAppResourcePolicy {
        csp: McpAppResourceCsp {
            connect_domains: vec!["https://api.example.com".to_string()],
            ..Default::default()
        },
        requested_permissions: Default::default(),
    };
    assert!(policy_is_subset(&narrower, &approved));

    let expanded_domain = McpAppResourcePolicy {
        csp: McpAppResourceCsp {
            connect_domains: vec!["https://unreviewed.example.com".to_string()],
            ..Default::default()
        },
        requested_permissions: Default::default(),
    };
    assert!(!policy_is_subset(&expanded_domain, &approved));

    let expanded_permission = McpAppResourcePolicy {
        csp: Default::default(),
        requested_permissions: McpAppResourcePermissions {
            camera: Some(json!({})),
            ..Default::default()
        },
    };
    assert!(!policy_is_subset(&expanded_permission, &approved));
}

#[test]
fn sse_parser_assembles_multiline_events_and_matches_request_id() {
    let event =
        b"event: message\r\ndata: {\"jsonrpc\":\"2.0\",\r\ndata: \"id\":7,\"result\":{}}\r\n";
    assert_eq!(
        sse_event_value(event).unwrap(),
        Some(json!({"jsonrpc": "2.0", "id": 7, "result": {}}))
    );
    assert!(response_matches_id(
        &sse_event_value(event).unwrap().unwrap(),
        7
    ));
    assert!(!response_matches_id(
        &json!({"jsonrpc": "2.0", "method": "notifications/progress"}),
        7
    ));
}

#[test]
fn sse_parser_finds_lf_and_crlf_event_boundaries() {
    assert_eq!(sse_event_end(b"data: {}\n\nnext"), Some((8, 10)));
    assert_eq!(sse_event_end(b"data: {}\r\n\r\nnext"), Some((8, 12)));
}

#[test]
fn sse_parser_skips_notifications_until_the_matching_response() {
    let mut pending = br#"data: {"jsonrpc":"2.0","method":"notifications/progress"}

data: {"jsonrpc":"2.0","id":9,"result":{"ok":true}}

"#
    .to_vec();
    assert_eq!(
        take_matching_sse_value(&mut pending, Some(9), true).unwrap(),
        Some(json!({"jsonrpc": "2.0", "id": 9, "result": {"ok": true}}))
    );
    assert!(pending.is_empty());
}

#[test]
fn inner_app_frame_uses_an_opaque_origin_and_fixed_sandbox() {
    assert!(SANDBOX_PROXY_HTML.contains(r#"inner.setAttribute("sandbox", "allow-scripts")"#));
    assert!(SANDBOX_PROXY_HTML.contains("inner.srcdoc = htmlWithCsp(html, csp)"));
    assert!(SANDBOX_PROXY_HTML.contains("frame-src ${frames}"));
    assert!(SANDBOX_PROXY_HTML.contains("sources(csp?.frameDomains, \"'none'\")"));
    assert!(SANDBOX_PROXY_HTML.contains("sources(csp?.baseUriDomains, \"'self'\")"));
    assert!(!SANDBOX_PROXY_HTML.contains("new URL(document.referrer).origin"));
    assert!(!SANDBOX_PROXY_HTML.contains("http://localhost:1420"));
    assert!(!SANDBOX_PROXY_HTML.contains("http://127.0.0.1:1420"));
    #[cfg(debug_assertions)]
    {
        let debug_proxy = sandbox_proxy_html();
        assert!(debug_proxy.contains("http://localhost:1420"));
        assert!(debug_proxy.contains("http://127.0.0.1:1420"));
    }
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
fn resource_read_policy_takes_precedence_over_listing_metadata() {
    let listing = McpAppResource {
        uri: "ui://board".to_string(),
        name: None,
        title: None,
        description: None,
        mime_type: Some(MCP_APP_MIME_TYPE.to_string()),
        meta: json!({
            "ui": {
                "csp": {"connectDomains": ["https://listing.example.com"]}
            }
        }),
    };
    let response = json!({
        "result": {
            "contents": [{
                "uri": "ui://board",
                "mimeType": MCP_APP_MIME_TYPE,
                "text": "<main>Board</main>",
                "_meta": {
                    "ui": {
                        "csp": {"connectDomains": ["https://read.example.com"]}
                    }
                }
            }]
        }
    });
    let (_, csp, _) = parse_ui_resource(&response, "ui://board", Some(&listing)).unwrap();
    assert_eq!(csp.connect_domains, vec!["https://read.example.com"]);
}

#[test]
fn ui_resource_csp_is_sanitized_before_reaching_the_proxy() {
    let response = json!({
        "result": {
            "contents": [{
                "uri": "ui://board",
                "mimeType": MCP_APP_MIME_TYPE,
                "text": "<main>Board</main>",
                "_meta": {
                    "ui": {
                        "csp": {
                            "connectDomains": [
                                "https://api.example.com",
                                "wss://events.example.com",
                                "http://public.example.com",
                                "https://example.com/path"
                            ],
                            "frameDomains": ["https://video.example.com"]
                        }
                    }
                }
            }]
        }
    });
    let (_, csp, _) = parse_ui_resource(&response, "ui://board", None).unwrap();
    assert_eq!(
        csp.connect_domains,
        vec!["https://api.example.com", "wss://events.example.com"]
    );
    assert_eq!(csp.frame_domains, vec!["https://video.example.com"]);
}

#[test]
fn modern_request_carries_version_method_and_meta() {
    let params = prepare_params(McpEra::Modern, json!({}), MCP_MODERN_PROTOCOL_VERSION);
    let payload = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list", "params": params});
    let headers = build_mcp_headers(
        McpEra::Modern,
        Some(MCP_MODERN_PROTOCOL_VERSION),
        None,
        &payload,
        &[],
    );
    assert!(headers.contains(&("MCP-Protocol-Version".to_string(), "2026-07-28".to_string())));
    assert!(headers.contains(&("Mcp-Method".to_string(), "tools/list".to_string())));
    assert!(headers
        .iter()
        .all(|(name, _)| name.as_str() != "mcp-session-id"));
    let meta = payload
        .pointer("/params/_meta")
        .expect("modern params must carry _meta");
    assert_eq!(
        meta.get("io.modelcontextprotocol/protocolVersion"),
        Some(&json!(MCP_MODERN_PROTOCOL_VERSION))
    );
    let mime_types = meta
        .get("io.modelcontextprotocol/clientCapabilities")
        .and_then(|caps| caps.pointer("/extensions/io.modelcontextprotocol~1ui/mimeTypes"))
        .expect("capabilities must declare the ui extension");
    assert_eq!(mime_types, &json!([MCP_APP_MIME_TYPE]));
    assert!(meta.get("io.modelcontextprotocol/clientInfo").is_some());
}

#[test]
fn legacy_request_keeps_session_and_omits_modern_extras() {
    let params = prepare_params(
        McpEra::Legacy,
        json!({"cursor": "abc"}),
        MCP_PROTOCOL_VERSION,
    );
    assert_eq!(params, json!({"cursor": "abc"}));
    let payload = json!({"jsonrpc": "2.0", "id": 7, "method": "tools/list", "params": params});
    let headers = build_mcp_headers(
        McpEra::Legacy,
        Some(MCP_PROTOCOL_VERSION),
        Some("session-123"),
        &payload,
        &[],
    );
    assert!(headers.contains(&(
        "mcp-protocol-version".to_string(),
        MCP_PROTOCOL_VERSION.to_string()
    )));
    assert!(headers.contains(&("mcp-session-id".to_string(), "session-123".to_string())));
    assert!(headers.iter().all(|(name, _)| {
        name.as_str() != "Mcp-Method"
            && name.as_str() != "MCP-Protocol-Version"
            && name.as_str() != "Mcp-Name"
    }));
}

#[test]
fn mcp_name_header_targets_name_addressed_methods() {
    let call = json!({
        "jsonrpc": "2.0", "id": 1, "method": "tools/call",
        "params": {"name": "board.update", "arguments": {}}
    });
    let headers = build_mcp_headers(
        McpEra::Modern,
        Some(MCP_MODERN_PROTOCOL_VERSION),
        None,
        &call,
        &[],
    );
    assert!(headers.contains(&("Mcp-Name".to_string(), "board.update".to_string())));

    let read = json!({
        "jsonrpc": "2.0", "id": 2, "method": "resources/read",
        "params": {"uri": "ui://board"}
    });
    let headers = build_mcp_headers(
        McpEra::Modern,
        Some(MCP_MODERN_PROTOCOL_VERSION),
        None,
        &read,
        &[],
    );
    assert!(headers.contains(&("Mcp-Name".to_string(), "ui://board".to_string())));

    let list = json!({"jsonrpc": "2.0", "id": 3, "method": "tools/list", "params": {}});
    let headers = build_mcp_headers(
        McpEra::Modern,
        Some(MCP_MODERN_PROTOCOL_VERSION),
        None,
        &list,
        &[],
    );
    assert!(headers.iter().all(|(name, _)| name.as_str() != "Mcp-Name"));
}

#[test]
fn non_ascii_mcp_name_uses_base64_sentinel() {
    assert_eq!(mcp_name_header_value("board.update"), "board.update");
    assert_eq!(mcp_name_header_value("café"), "=?base64?Y2Fmw6k=?=");
    let call = json!({
        "jsonrpc": "2.0", "id": 1, "method": "tools/call",
        "params": {"name": "café", "arguments": {}}
    });
    let headers = build_mcp_headers(
        McpEra::Modern,
        Some(MCP_MODERN_PROTOCOL_VERSION),
        None,
        &call,
        &[],
    );
    assert!(headers.contains(&("Mcp-Name".to_string(), "=?base64?Y2Fmw6k=?=".to_string())));
}

#[test]
fn valid_nested_tool_parameters_are_mirrored_into_headers() {
    let response = json!({
        "result": {
            "tools": [{
                "name": "board.update",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "context": {
                            "type": "object",
                            "properties": {
                                "space": {
                                    "type": "string",
                                    "x-mcp-header": "Space"
                                }
                            }
                        },
                        "priority": {
                            "type": "integer",
                            "x-mcp-header": "Priority"
                        },
                        "approved": {
                            "type": "boolean",
                            "x-mcp-header": "Approved"
                        }
                    }
                }
            }]
        }
    });
    let tools = parse_tools(&response).unwrap();
    let params = json!({
        "name": "board.update",
        "arguments": {
            "context": {"space": "product"},
            "priority": 7,
            "approved": true
        }
    });
    let headers = tool_param_headers(&tools, "tools/call", &params).unwrap();
    assert_eq!(headers.len(), 3);
    assert!(headers.contains(&("Mcp-Param-Space".to_string(), "product".to_string())));
    assert!(headers.contains(&("Mcp-Param-Priority".to_string(), "7".to_string())));
    assert!(headers.contains(&("Mcp-Param-Approved".to_string(), "true".to_string())));
}

#[test]
fn invalid_parameter_header_annotations_exclude_only_that_tool() {
    let response = json!({
        "result": {
            "tools": [
                {
                    "name": "invalid",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "first": {"type": "string", "x-mcp-header": "Tenant"},
                            "second": {"type": "string", "x-mcp-header": "tenant"}
                        }
                    }
                },
                {
                    "name": "also-invalid",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "ratio": {"type": "number", "x-mcp-header": "Ratio"}
                        }
                    }
                },
                {
                    "name": "valid",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "region": {"type": "string", "x-mcp-header": "Region"}
                        }
                    }
                }
            ]
        }
    });
    let tools = parse_tools(&response).unwrap();
    assert_eq!(
        tools
            .iter()
            .map(|tool| tool.name.as_str())
            .collect::<Vec<_>>(),
        vec!["valid"]
    );
}

#[test]
fn missing_and_null_parameter_headers_are_omitted() {
    let response = json!({
        "result": {
            "tools": [{
                "name": "query",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "region": {"type": "string", "x-mcp-header": "Region"},
                        "tenant": {"type": "string", "x-mcp-header": "Tenant"}
                    }
                }
            }]
        }
    });
    let tools = parse_tools(&response).unwrap();
    let headers = tool_param_headers(
        &tools,
        "tools/call",
        &json!({"name": "query", "arguments": {"region": null}}),
    )
    .unwrap();
    assert!(headers.is_empty());
}

#[test]
fn unsafe_parameter_header_values_use_the_base64_sentinel() {
    assert_eq!(
        mcp_param_header_value(&json!("hello world")).unwrap(),
        Some("hello world".to_string())
    );
    assert_eq!(
        mcp_param_header_value(&json!(" padded ")).unwrap(),
        Some("=?base64?IHBhZGRlZCA=?=".to_string())
    );
    assert_eq!(
        mcp_param_header_value(&json!("line1\nline2")).unwrap(),
        Some("=?base64?bGluZTEKbGluZTI=?=".to_string())
    );
    assert_eq!(
        mcp_param_header_value(&json!("=?base64?literal?=")).unwrap(),
        Some("=?base64?PT9iYXNlNjQ/bGl0ZXJhbD89?=".to_string())
    );
}

#[test]
fn rpc_errors_are_bounded_and_do_not_echo_server_data() {
    let error = json!({
        "code": -32000,
        "message": "x".repeat(MAX_MCP_ERROR_MESSAGE_CHARS + 10),
        "data": {"secret": "must not be shown"}
    });
    let rendered = format_rpc_error(&error);
    assert!(rendered.starts_with("code -32000: "));
    assert!(rendered.ends_with('…'));
    assert!(!rendered.contains("secret"));
    assert!(rendered.chars().count() <= MAX_MCP_ERROR_MESSAGE_CHARS + 20);
}

#[test]
fn era_probe_classifies_modern_errors_and_legacy_fallback() {
    let ok = reqwest::StatusCode::OK;
    let bad_request = reqwest::StatusCode::BAD_REQUEST;
    let result = json!({"jsonrpc": "2.0", "id": 1, "result": {"tools": []}});
    assert_eq!(classify_probe(ok, Some(&result)), ProbeOutcome::Modern);

    let unsupported = json!({
        "jsonrpc": "2.0", "id": 1,
        "error": {"code": -32022, "message": "unsupported", "data": {"supported": ["2026-07-28"]}}
    });
    assert_eq!(
        classify_probe(bad_request, Some(&unsupported)),
        ProbeOutcome::ModernRetry {
            supported: vec!["2026-07-28".to_string()]
        }
    );

    let mismatch = json!({
        "jsonrpc": "2.0", "id": 1,
        "error": {"code": -32020, "message": "header mismatch"}
    });
    assert!(matches!(
        classify_probe(bad_request, Some(&mismatch)),
        ProbeOutcome::ModernError { .. }
    ));

    let legacy_error = json!({
        "jsonrpc": "2.0", "id": 1,
        "error": {"code": -32000, "message": "server not initialized"}
    });
    assert_eq!(
        classify_probe(bad_request, Some(&legacy_error)),
        ProbeOutcome::Legacy
    );
    assert_eq!(classify_probe(bad_request, None), ProbeOutcome::Legacy);
    assert_eq!(
        classify_probe(reqwest::StatusCode::NOT_FOUND, None),
        ProbeOutcome::Legacy
    );
    assert_eq!(
        classify_probe(reqwest::StatusCode::METHOD_NOT_ALLOWED, None),
        ProbeOutcome::Legacy
    );
}

#[test]
fn absent_result_type_reads_as_complete() {
    assert_eq!(result_completion(&json!({"tools": []})), "complete");
    assert_eq!(
        result_completion(&json!({"resultType": "partial"})),
        "partial"
    );
    let response = json!({
        "result": {"resultType": "complete", "ttlMs": 5000, "cacheScope": "origin", "content": []}
    });
    let result =
        extract_result(&response, "tools/call").expect("advisory fields must be tolerated");
    assert_eq!(result.get("content"), Some(&json!([])));
}

#[test]
fn resource_not_found_accepts_both_error_codes() {
    assert!(is_resource_not_found(
        &json!({"code": -32002, "message": "not found"})
    ));
    assert!(is_resource_not_found(
        &json!({"code": -32602, "message": "not found"})
    ));
    assert!(!is_resource_not_found(
        &json!({"code": -32000, "message": "other"})
    ));
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

#[test]
fn generic_invalid_params_does_not_classify_a_legacy_server_as_modern() {
    // -32602 is standard JSON-RPC "Invalid params", not a modern-only code. A
    // 2025-11-25 server rejecting our modern probe must still fall back to the
    // initialize handshake rather than hard-failing the connection.
    assert!(matches!(
        classify_probe(
            reqwest::StatusCode::BAD_REQUEST,
            Some(
                &json!({"jsonrpc": "2.0", "error": {"code": -32602, "message": "Invalid params"}})
            )
        ),
        ProbeOutcome::Legacy
    ));
    // A genuinely modern-only code still classifies as modern.
    assert!(!matches!(
        classify_probe(
            reqwest::StatusCode::BAD_REQUEST,
            Some(
                &json!({"jsonrpc": "2.0", "error": {"code": -32021, "message": "missing capability"}})
            )
        ),
        ProbeOutcome::Legacy
    ));
}

#[test]
fn control_characters_never_reach_a_raw_mcp_name_header() {
    // Tool names and resource URIs are server-authored. Any byte outside
    // printable-ASCII must take the base64 sentinel path so CR/LF can never
    // split headers.
    for raw in [
        "evil\r\nX-Injected: 1",
        "a\nb",
        "a\rb",
        "a\u{0000}b",
        "a b",
        "tab\there",
        "del\u{007f}",
        "=?base64?spoof?=",
        "café",
    ] {
        let value = mcp_name_header_value(raw);
        assert!(
            value.starts_with("=?base64?") && value.ends_with("?="),
            "{raw:?} must be sentinel-encoded, got {value:?}"
        );
        assert!(
            !value.contains(['\r', '\n', '\0']),
            "{raw:?} produced an unsafe header value"
        );
    }
    // Ordinary header-safe names stay plain.
    assert_eq!(mcp_name_header_value("weather.get"), "weather.get");
    assert_eq!(mcp_name_header_value("ui://app/board"), "ui://app/board");
}

#[test]
fn probe_falls_back_to_legacy_without_a_recognized_modern_error() {
    // A legacy server may answer the sessionless probe with 5xx or an
    // unrecognized error. Both must fall back to the handshake, not hard-fail.
    for status in [
        reqwest::StatusCode::INTERNAL_SERVER_ERROR,
        reqwest::StatusCode::UNAUTHORIZED,
        reqwest::StatusCode::BAD_REQUEST,
    ] {
        assert!(matches!(classify_probe(status, None), ProbeOutcome::Legacy));
    }
    assert!(matches!(
        classify_probe(
            reqwest::StatusCode::OK,
            Some(
                &json!({"jsonrpc": "2.0", "error": {"code": -32000, "message": "not initialized"}})
            )
        ),
        ProbeOutcome::Legacy
    ));
}
