use super::*;
use axum::body::to_bytes;
use business_analytics::{ACCEPTANCE_FINANCE_USER, ACCEPTANCE_SALES_USER};
use business_anomaly_contracts::BusinessAnomalyResult;
use tower::ServiceExt;

#[test]
fn iam_scope_maps_known_dimensions_and_rejects_mismatches() {
    let grant = EffectiveGrant {
        capability: business_iam::Capability::parse("sales_order:read").expect("capability"),
        data_scope: DataScope::Restricted(BTreeMap::from([
            (
                "legal_entity".into(),
                ["cn".to_string()].into_iter().collect(),
            ),
            (
                "warehouse".into(),
                ["shanghai".to_string()].into_iter().collect(),
            ),
        ])),
        obligations: Default::default(),
    };
    let scope = iam_authorization_scope(&grant, "sales_order:read").expect("known scope");
    assert_eq!(
        scope.legal_entity_ids,
        ["cn".to_string()].into_iter().collect()
    );
    assert_eq!(
        scope.warehouse_ids,
        ["shanghai".to_string()].into_iter().collect()
    );
    assert!(iam_authorization_scope(&grant, "inventory:read").is_none());

    let unknown = EffectiveGrant {
        data_scope: DataScope::Restricted(BTreeMap::from([(
            "untrusted_dimension".into(),
            ["all".to_string()].into_iter().collect(),
        )])),
        ..grant
    };
    assert!(iam_authorization_scope(&unknown, "sales_order:read").is_none());
}

fn request(tool: &str, user: Uuid, input: Value) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(format!("/v1/read/{tool}"))
        .header("content-type", "application/json")
        .header(
            "x-business-service-credential",
            "acceptance-service-credential-32-bytes-minimum",
        )
        .header("x-business-service-audience", "business-read-api")
        .header("x-enterprise-user-id", user.to_string())
        .header("x-identity-binding-id", Uuid::new_v4().to_string())
        .header("x-agent-delegation-id", Uuid::new_v4().to_string())
        .header("x-agent-id", "business-anomaly-agent")
        .header("x-agent-turn-id", "turn-1")
        .header("x-agent-used-calls", "1")
        .header(
            "x-agent-required-scope",
            required_capability(tool).expect("known test tool"),
        )
        .header("x-trace-id", Uuid::new_v4().to_string())
        .body(Body::from(input.to_string()))
        .expect("request")
}
fn app() -> Router {
    router_with_verifier(
        "acceptance-service-credential-32-bytes-minimum".into(),
        DelegationVerifier::AcceptanceTest,
    )
    .expect("router")
}

#[tokio::test]
async fn different_users_get_different_authorized_results() {
    let finance = app()
        .oneshot(request(
            "analyze_cross_domain_risks",
            ACCEPTANCE_FINANCE_USER,
            json!({"limit":100}),
        ))
        .await
        .expect("response");
    let sales = app()
        .oneshot(request(
            "analyze_cross_domain_risks",
            ACCEPTANCE_SALES_USER,
            json!({"limit":100}),
        ))
        .await
        .expect("response");
    let f: BusinessAnomalyResult = serde_json::from_slice(
        &to_bytes(finance.into_body(), 128 * 1024)
            .await
            .expect("body"),
    )
    .expect("json");
    let s: BusinessAnomalyResult =
        serde_json::from_slice(&to_bytes(sales.into_body(), 128 * 1024).await.expect("body"))
            .expect("json");
    assert!(f.totals.finding_count > s.totals.finding_count);
    assert_ne!(
        f.scope_summary["effectiveScopeHash"],
        s.scope_summary["effectiveScopeHash"]
    );
}

#[tokio::test]
async fn disjoint_scope_and_semantic_filters_fail_closed() {
    let denied = app()
        .oneshot(request(
            "search_business_anomalies",
            ACCEPTANCE_SALES_USER,
            json!({"legalEntityIds":["LE-B"],"limit":100}),
        ))
        .await
        .expect("response");
    let denied: BusinessAnomalyResult = serde_json::from_slice(
        &to_bytes(denied.into_body(), 128 * 1024)
            .await
            .expect("body"),
    )
    .expect("json");
    assert!(denied.findings.is_empty());

    let first = app()
        .oneshot(request(
            "search_business_anomalies",
            ACCEPTANCE_FINANCE_USER,
            json!({
                "anomalyTypes":["negative_inventory"],
                "severities":["critical"],
                "skuIds":["SKU-NEG"],
                "limit":1
            }),
        ))
        .await
        .expect("response");
    let first: BusinessAnomalyResult =
        serde_json::from_slice(&to_bytes(first.into_body(), 128 * 1024).await.expect("body"))
            .expect("json");
    assert_eq!(first.totals.finding_count, 1);
    assert_eq!(first.findings.len(), 1);
    assert_eq!(first.findings[0].r#type, "negative_inventory");
    assert_eq!(
        first.findings[0].primary_resource.id.as_deref(),
        Some("SKU-NEG")
    );
    assert!(!first.pagination.expect("pagination").has_more);
}
#[tokio::test]
async fn exact_id_denial_is_indistinguishable_from_missing() {
    for id in ["SO-B-001", "SO-NOT-THERE"] {
        let response = app()
            .oneshot(request(
                "get_sales_order",
                ACCEPTANCE_SALES_USER,
                json!({"orderId":id}),
            ))
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
}
#[tokio::test]
async fn service_identity_and_context_are_mandatory() {
    let response = app()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/read/query_inventory_balance")
                .body(Body::from("{}"))
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}
#[tokio::test]
async fn response_trace_is_request_trace() {
    let trace = Uuid::new_v4();
    let mut req = request(
        "analyze_order_profit_risks",
        ACCEPTANCE_FINANCE_USER,
        json!({"limit":20}),
    );
    req.headers_mut()
        .insert("x-trace-id", trace.to_string().parse().expect("header"));
    let response = app().oneshot(req).await.expect("response");
    let value: BusinessAnomalyResult = serde_json::from_slice(
        &to_bytes(response.into_body(), 128 * 1024)
            .await
            .expect("body"),
    )
    .expect("json");
    assert_eq!(value.trace_id, trace);
}

#[tokio::test]
async fn all_eight_v5_routes_enforce_contract_and_return_bounded_results() {
    let search = app()
        .oneshot(request(
            "search_business_anomalies",
            ACCEPTANCE_FINANCE_USER,
            json!({"limit":100}),
        ))
        .await
        .expect("search response");
    assert_eq!(search.status(), StatusCode::OK);
    let search_result: BusinessAnomalyResult = serde_json::from_slice(
        &to_bytes(search.into_body(), 128 * 1024)
            .await
            .expect("search body"),
    )
    .expect("search result");
    let finding_id = search_result.findings[0].id;

    let cases = [
        ("get_business_anomaly", json!({"findingId":finding_id})),
        ("analyze_order_profit_risks", json!({"limit":100})),
        ("analyze_receivable_risks", json!({"limit":100})),
        ("analyze_inventory_risks", json!({"limit":100})),
        ("analyze_purchase_cost_risks", json!({"limit":100})),
        ("analyze_cross_domain_risks", json!({"limit":100})),
        (
            "explain_profit_change",
            json!({
                "basePeriod":{"from":"2026-06-01","to":"2026-06-30"},
                "comparisonPeriod":{"from":"2026-07-01","to":"2026-07-31"}
            }),
        ),
    ];
    for (tool, input) in cases {
        let response = app()
            .oneshot(request(tool, ACCEPTANCE_FINANCE_USER, input))
            .await
            .expect("route response");
        assert_eq!(response.status(), StatusCode::OK, "{tool}");
        let body = to_bytes(response.into_body(), 128 * 1024)
            .await
            .expect("route body");
        assert!(body.len() <= 128 * 1024, "{tool}");
        let result: BusinessAnomalyResult = serde_json::from_slice(&body).expect("anomaly result");
        assert_eq!(result.rule_set_version, "trade-risk-v1.0", "{tool}");
        assert!(result.findings.len() <= 100, "{tool}");
    }
}

#[tokio::test]
async fn direct_api_rejects_prompt_shaped_and_generic_query_fields() {
    for input in [
        json!({"sql":"select * from receivables"}),
        json!({"url":"https://example.invalid/export"}),
        json!({"limit":101}),
    ] {
        let response = app()
            .oneshot(request(
                "analyze_cross_domain_risks",
                ACCEPTANCE_FINANCE_USER,
                input,
            ))
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
}

#[tokio::test]
async fn in_process_acceptance_cross_domain_p95_is_below_target() {
    let mut samples = Vec::new();
    for _ in 0..50 {
        let started = std::time::Instant::now();
        let response = app()
            .oneshot(request(
                "analyze_cross_domain_risks",
                ACCEPTANCE_FINANCE_USER,
                json!({"limit":100}),
            ))
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::OK);
        samples.push(started.elapsed());
    }
    samples.sort();
    let p95 = samples[(samples.len() * 95 / 100).min(samples.len() - 1)];
    eprintln!("desensitized in-process cross-domain p95: {p95:?}");
    assert!(p95 < Duration::from_secs(10));
}
