use buzz_client::CommunityEndpoint;

#[test]
fn production_http_and_websocket_schemes_normalize_to_one_authority() {
    let from_https = CommunityEndpoint::parse("https://BUZZ.Example:443/")
        .expect("valid HTTPS community endpoint");
    assert_eq!(from_https.http_base_url().as_str(), "https://buzz.example/");
    assert_eq!(from_https.websocket_url().as_str(), "wss://buzz.example/");
    assert_eq!(
        from_https.query_url().as_str(),
        "https://buzz.example/query"
    );
    assert_eq!(
        from_https.events_url().as_str(),
        "https://buzz.example/events"
    );
    assert_eq!(
        from_https.upload_url().as_str(),
        "https://buzz.example/upload"
    );

    let from_wss =
        CommunityEndpoint::parse("wss://buzz.example").expect("valid WSS community endpoint");
    assert_eq!(from_wss.http_base_url(), from_https.http_base_url());
    assert_eq!(from_wss.websocket_url(), from_https.websocket_url());
}

#[test]
fn development_endpoint_preserves_non_default_effective_port() {
    let endpoint = CommunityEndpoint::parse("ws://127.0.0.1:3123/")
        .expect("valid development community endpoint");
    assert_eq!(endpoint.http_base_url().as_str(), "http://127.0.0.1:3123/");
    assert_eq!(endpoint.websocket_url().as_str(), "ws://127.0.0.1:3123/");
    assert_eq!(
        endpoint
            .same_authority_url("http://127.0.0.1:3123/media/a.png")
            .expect("same authority media URL")
            .as_str(),
        "http://127.0.0.1:3123/media/a.png"
    );
}

#[test]
fn effective_default_ports_are_the_same_authority() {
    let endpoint = CommunityEndpoint::parse("https://buzz.example").expect("endpoint");
    endpoint
        .same_authority_url("https://buzz.example:443/media/a.png")
        .expect("explicit default port is same authority");
    endpoint
        .same_authority_url("wss://buzz.example/socket")
        .expect("paired secure websocket scheme is same authority");
}

#[test]
fn invalid_or_ambiguous_bases_are_rejected() {
    for invalid in [
        "not a URL",
        "ftp://buzz.example",
        "https://user:secret@buzz.example",
        "https://buzz.example/community",
        "https://buzz.example?tenant=other",
        "https://buzz.example/#fragment",
    ] {
        assert!(
            CommunityEndpoint::parse(invalid).is_err(),
            "must reject {invalid}"
        );
    }
}

#[test]
fn cross_authority_resources_are_rejected_before_use() {
    let endpoint = CommunityEndpoint::parse("https://buzz.example").expect("endpoint");
    for foreign in [
        "https://other.example/media/a.png",
        "https://buzz.example:444/media/a.png",
        "http://buzz.example/media/a.png",
        "https://user@buzz.example/media/a.png",
    ] {
        assert!(
            endpoint.same_authority_url(foreign).is_err(),
            "must reject {foreign}"
        );
    }
}
