use axum::{
    body::{to_bytes, Body},
    http::Request,
};
use business_core::{
    model::{BootstrapRequest, GrantOperation, ResourceType, ScopeDimension},
    router, AppState, Config, PgStore,
};
use serde_json::json;
use sqlx::postgres::PgPoolOptions;
use tower::ServiceExt;
use uuid::Uuid;

struct Fixture {
    input: BootstrapRequest,
    admin: Uuid,
    salesperson: Uuid,
    approver: Uuid,
    warehouse_one: Uuid,
    warehouse_two: Uuid,
    customer: Uuid,
    business_unit_one: Uuid,
    role_approver: Uuid,
}

fn fixture() -> Fixture {
    let admin = Uuid::new_v4();
    let salesperson = Uuid::new_v4();
    let approver = Uuid::new_v4();
    let legal_one = Uuid::new_v4();
    let legal_two = Uuid::new_v4();
    let business_unit_one = Uuid::new_v4();
    let business_unit_two = Uuid::new_v4();
    let warehouse_one = Uuid::new_v4();
    let warehouse_two = Uuid::new_v4();
    let customer = Uuid::new_v4();
    let supplier = Uuid::new_v4();
    let uom = Uuid::new_v4();
    let category = Uuid::new_v4();
    let brand = Uuid::new_v4();
    let product = Uuid::new_v4();
    let role_admin = Uuid::new_v4();
    let role_sales = Uuid::new_v4();
    let role_approver = Uuid::new_v4();
    let mut input: BootstrapRequest = serde_json::from_value(json!({
        "group":{"id":Uuid::new_v4(),"code":"PAC_GROUP","name":"Pacioli Trading Group","baseCurrency":"CNY","timezone":"Asia/Shanghai"},
        "legalEntities":[
            {"id":legal_one,"code":"LE_CN_01","name":"Shanghai Trading Co.","countryCode":"CN","functionalCurrency":"CNY"},
            {"id":legal_two,"code":"LE_HK_01","name":"Hong Kong Trading Co.","countryCode":"HK","functionalCurrency":"HKD"}
        ],
        "ledgerBooks":[
            {"id":Uuid::new_v4(),"legalEntityId":legal_one,"code":"BOOK_CN","name":"China Primary Ledger","currency":"CNY","fiscalYearStartMonth":1,"isPrimary":true},
            {"id":Uuid::new_v4(),"legalEntityId":legal_two,"code":"BOOK_HK","name":"Hong Kong Primary Ledger","currency":"HKD","fiscalYearStartMonth":1,"isPrimary":true}
        ],
        "businessUnits":[
            {"id":business_unit_one,"legalEntityId":legal_one,"code":"BU_CN_01","name":"China Domestic Trade"},
            {"id":business_unit_two,"legalEntityId":legal_two,"code":"BU_HK_01","name":"Hong Kong Export Trade"}
        ],
        "departments":[{"id":Uuid::new_v4(),"businessUnitId":business_unit_one,"code":"DEPT_SALES","name":"Sales"}],
        "unitsOfMeasure":[{"id":uom,"code":"UOM_EA","name":"Each","precisionScale":0}],
        "productCategories":[{"id":category,"code":"CAT_AUDIO","name":"Audio Equipment"}],
        "brands":[{"id":brand,"code":"BRAND_BB","name":"Block Buzz"}],
        "warehouses":[
            {"id":warehouse_one,"legalEntityId":legal_one,"businessUnitId":business_unit_one,"code":"WH_SH_01","name":"Shanghai Warehouse"},
            {"id":warehouse_two,"legalEntityId":legal_two,"businessUnitId":business_unit_two,"code":"WH_HK_01","name":"Hong Kong Warehouse"}
        ],
        "customers":[{"id":customer,"legalEntityId":legal_one,"businessUnitId":business_unit_one,"code":"CUS_0001","name":"East China Retail","creditCurrency":"CNY","creditLimitMinor":10000000}],
        "suppliers":[{"id":supplier,"legalEntityId":legal_one,"businessUnitId":business_unit_one,"code":"SUP_0001","name":"Delta Components"}],
        "products":[{"id":product,"code":"PROD_0001","name":"Portable Speaker","categoryId":category,"brandId":brand,"baseUomId":uom}],
        "skus":[{"id":Uuid::new_v4(),"productId":product,"code":"SKU_0001","name":"Portable Speaker Black","barcode":"6900000000001"}],
        "salespeople":[{"id":Uuid::new_v4(),"enterpriseUserId":salesperson,"businessUnitId":business_unit_one,"code":"SP_0001","name":"Sales User"}],
        "roles":[
            {"id":role_admin,"roleKey":"business_admin","name":"Business Administrator","permissionKeys":["business_core:admin","business_authorization:read_all","business_master_data:read","business_directory:resolve"]},
            {"id":role_sales,"roleKey":"sales_operator","name":"Sales Operator","permissionKeys":["business_master_data:read","sales:order_handle"]},
            {"id":role_approver,"roleKey":"credit_approver","name":"Credit Approver","permissionKeys":["business_master_data:read","sales:credit_approve"]}
        ],
        "userRoles":[
            {"enterpriseUserId":admin,"roleId":role_admin},
            {"enterpriseUserId":salesperson,"roleId":role_sales},
            {"enterpriseUserId":approver,"roleId":role_approver}
        ],
        "scopes":[
            {"enterpriseUserId":admin,"dimension":"legal_entity","resourceId":legal_one},
            {"enterpriseUserId":admin,"dimension":"warehouse","resourceId":warehouse_one},
            {"enterpriseUserId":admin,"dimension":"customer","resourceId":customer},
            {"enterpriseUserId":admin,"dimension":"supplier","resourceId":supplier},
            {"enterpriseUserId":admin,"dimension":"brand","resourceId":brand},
            {"enterpriseUserId":admin,"dimension":"business_unit","resourceId":business_unit_one},
            {"enterpriseUserId":salesperson,"dimension":"legal_entity","resourceId":legal_one},
            {"enterpriseUserId":salesperson,"dimension":"warehouse","resourceId":warehouse_one},
            {"enterpriseUserId":salesperson,"dimension":"customer","resourceId":customer},
            {"enterpriseUserId":salesperson,"dimension":"brand","resourceId":brand},
            {"enterpriseUserId":salesperson,"dimension":"business_unit","resourceId":business_unit_one},
            {"enterpriseUserId":approver,"dimension":"legal_entity","resourceId":legal_one},
            {"enterpriseUserId":approver,"dimension":"customer","resourceId":customer},
            {"enterpriseUserId":approver,"dimension":"business_unit","resourceId":business_unit_one}
        ],
        "assignmentPolicies":[{"actionCode":"sales:exception_followup","requiredPermission":"sales:order_handle","eligibleRoleKeys":["sales_operator"]}],
        "approvalPolicies":[{"actionCode":"sales:credit_override","requiredPermission":"sales:credit_approve","eligibleRoleKeys":["credit_approver"],"minApprovers":1,"allowSelfApproval":false,"stepUpAmountMinor":5000000}]
    }))
    .unwrap();
    for number in 2..=20 {
        input.skus.push(
            serde_json::from_value(json!({
                "id": Uuid::new_v4(),
                "productId": product,
                "code": format!("SKU_{number:04}"),
                "name": format!("Portable Speaker Variant {number}"),
                "barcode": format!("690000000{number:04}")
            }))
            .unwrap(),
        );
    }
    for number in 2..=5 {
        input.customers.push(
            serde_json::from_value(json!({
                "id": Uuid::new_v4(),
                "legalEntityId": legal_one,
                "businessUnitId": business_unit_one,
                "code": format!("CUS_{number:04}"),
                "name": format!("Customer {number}"),
                "creditCurrency": "CNY",
                "creditLimitMinor": 1000000
            }))
            .unwrap(),
        );
    }
    for number in 2..=3 {
        input.suppliers.push(
            serde_json::from_value(json!({
                "id": Uuid::new_v4(),
                "legalEntityId": legal_one,
                "businessUnitId": business_unit_one,
                "code": format!("SUP_{number:04}"),
                "name": format!("Supplier {number}")
            }))
            .unwrap(),
        );
    }
    Fixture {
        input,
        admin,
        salesperson,
        approver,
        warehouse_one,
        warehouse_two,
        customer,
        business_unit_one,
        role_approver,
    }
}

#[tokio::test]
async fn b1_postgres_authorization_flow() {
    let Ok(database_url) = std::env::var("BUSINESS_CORE_TEST_DATABASE_URL") else {
        eprintln!("skipping: BUSINESS_CORE_TEST_DATABASE_URL is not set");
        return;
    };
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await
        .unwrap();
    let store = PgStore::new(pool.clone());
    store.migrate().await.unwrap();
    sqlx::raw_sql(
        "TRUNCATE business_core_outbox, business_core_audit_events, business_approval_policies, business_assignment_policies, business_unit_scopes, business_brand_scopes, business_supplier_scopes, business_customer_scopes, business_warehouse_scopes, business_legal_entity_scopes, business_user_roles, business_role_permissions, business_roles, business_salespeople, business_skus, business_products, business_suppliers, business_customers, business_warehouses, business_brands, business_product_categories, business_units_of_measure, business_departments, business_units, business_ledger_books, business_legal_entities, business_group_profile, enterprise_users CASCADE; UPDATE business_authorization_revision SET revision=1,updated_at=now() WHERE singleton;",
    )
    .execute(&pool)
    .await
    .unwrap();
    let fixture = fixture();
    for (id, subject, name) in [
        (fixture.admin, "admin", "Business Admin"),
        (fixture.salesperson, "sales", "Sales User"),
        (fixture.approver, "approver", "Credit Approver"),
    ] {
        sqlx::query("INSERT INTO enterprise_users(id,oidc_issuer,oidc_subject,email,display_name) VALUES($1,'https://identity.test',$2,$3,$4)")
            .bind(id).bind(subject).bind(format!("{subject}@example.test")).bind(name).execute(&pool).await.unwrap();
    }
    let bootstrapped = store
        .bootstrap(fixture.admin, Uuid::new_v4(), &fixture.input)
        .await
        .unwrap();
    assert_eq!(bootstrapped.group_id, fixture.input.group.id);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM business_skus")
            .fetch_one(&pool)
            .await
            .unwrap(),
        20
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM business_customers")
            .fetch_one(&pool)
            .await
            .unwrap(),
        5
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM business_suppliers")
            .fetch_one(&pool)
            .await
            .unwrap(),
        3
    );

    let snapshot = store.snapshot(fixture.salesperson).await.unwrap();
    assert!(snapshot.permission_keys.contains("sales:order_handle"));
    assert!(snapshot
        .scopes
        .warehouse_ids
        .contains(&fixture.warehouse_one));
    assert!(!snapshot
        .scopes
        .warehouse_ids
        .contains(&fixture.warehouse_two));
    assert_eq!(snapshot.effective_scope_hash.len(), 64);

    let (allowed, _) = store
        .can_access(
            fixture.salesperson,
            "business_master_data:read",
            ResourceType::Warehouse,
            fixture.warehouse_one,
        )
        .await
        .unwrap();
    assert!(allowed);
    let (allowed, _) = store
        .can_access(
            fixture.salesperson,
            "business_master_data:read",
            ResourceType::Warehouse,
            fixture.warehouse_two,
        )
        .await
        .unwrap();
    assert!(!allowed);

    let customer = store
        .resource(ResourceType::Customer, fixture.customer)
        .await
        .unwrap();
    let assignment = store
        .assignment_policy("sales:exception_followup")
        .await
        .unwrap();
    let assignees = store
        .eligible_users(
            &customer,
            &assignment.required_permission,
            &assignment.eligible_role_keys,
            None,
            None,
        )
        .await
        .unwrap();
    assert_eq!(assignees.len(), 1);
    assert_eq!(assignees[0].enterprise_user_id, fixture.salesperson);

    let revision = store.authorization_revision().await.unwrap();
    let next = store
        .mutate_role(
            fixture.admin,
            Uuid::new_v4(),
            fixture.salesperson,
            fixture.role_approver,
            GrantOperation::Grant,
            revision,
        )
        .await
        .unwrap();
    assert!(next > revision);
    let conflict = store
        .mutate_scope(
            fixture.admin,
            Uuid::new_v4(),
            fixture.salesperson,
            ScopeDimension::BusinessUnit,
            fixture.business_unit_one,
            GrantOperation::Grant,
            revision,
        )
        .await;
    assert!(matches!(
        conflict,
        Err(business_core::store::StoreError::Conflict)
    ));

    let audit_update = sqlx::query("UPDATE business_core_audit_events SET operation='tampered'")
        .execute(&pool)
        .await;
    assert!(audit_update.is_err());
    let tenant_columns: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM information_schema.columns WHERE table_schema='public' AND table_name LIKE 'business_%' AND column_name IN ('tenant_id','client_group_id')",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(tenant_columns, 0);

    let credential = "business-core-integration-secret-32-bytes".to_string();
    let config = Config {
        database_url,
        bind_addr: "127.0.0.1:3110".parse().unwrap(),
        service_credential: credential.clone(),
        service_audience: "business-core".into(),
        bootstrap_enabled: false,
        bootstrap_user_id: None,
        sales_enabled: true,
        inventory_enabled: true,
        receivables_enabled: true,
        purchasing_enabled: true,
        receiving_enabled: true,
        payables_enabled: true,
        profitability_enabled: true,
        management_reporting_enabled: true,
        operational_adjustments_enabled: true,
        profit_projection_worker_enabled: true,
        profit_projection_batch_size: 200,
        profit_projection_retry_limit: 5,
        sales_order_number_prefix: "SO".into(),
        shipment_number_prefix: "SHP".into(),
        receivable_number_prefix: "AR".into(),
        customer_receipt_number_prefix: "RCPT".into(),
        inventory_opening_number_prefix: "OPEN".into(),
        inventory_count_number_prefix: "CNT".into(),
        purchase_requisition_number_prefix: "PRQ".into(),
        purchase_order_number_prefix: "PO".into(),
        goods_receipt_number_prefix: "GR".into(),
        trade_payable_number_prefix: "AP".into(),
        supplier_payment_number_prefix: "PAY".into(),
        sales_return_number_prefix: "SRET".into(),
        purchase_return_number_prefix: "PRET".into(),
        profit_adjustment_number_prefix: "ADJ".into(),
        management_report_snapshot_number_prefix: "MGR".into(),
        profit_management_timezone: "Asia/Shanghai".into(),
        profit_default_currency: "CNY".into(),
        profit_allocation_max_targets: 500,
        profit_report_max_rows: 1000,
        profit_data_stale_after_minutes: 15,
        default_payment_terms_days: 30,
        default_supplier_payment_terms_days: 30,
        default_currency: "CNY".into(),
        command_rate_limit_per_minute: 60,
        business_web_origin: "https://business.example.test".into(),
        business_web_embed_origin: "https://business.example.test".into(),
        business_session_cookie_name: "__Host-bizfin_business".into(),
    };
    let workbench_session_id = Uuid::new_v4();
    let identity_binding_id = Uuid::new_v4();
    let embed_session_id = Uuid::new_v4();
    let business_session_id = Uuid::new_v4();
    let session_token = "browser-session-security-test";
    let csrf_token = "browser-csrf-security-test";
    let trace_id = Uuid::new_v4();
    sqlx::query("INSERT INTO workbench_sessions(id,enterprise_user_id,status,expires_at,trace_id) VALUES($1,$2,'active',now()+interval '1 hour',$3)")
        .bind(workbench_session_id).bind(fixture.salesperson).bind(trace_id).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO buzz_identity_bindings(id,enterprise_user_id,buzz_pubkey,device_id,device_name,device_platform,status) VALUES($1,$2,$3,'integration-device','Integration Device','linux','active')")
        .bind(identity_binding_id).bind(fixture.salesperson).bind("a".repeat(64)).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO embed_sessions(id,code_hash,enterprise_user_id,identity_binding_id,workbench_session_id,audience,deployment_id,target_path,target_resource_type,target_resource_id,status,expires_at,trace_id) VALUES($1,$2,$3,$4,$5,'business-dock','integration','/','business_home','home','consumed',now()+interval '1 hour',$6)")
        .bind(embed_session_id).bind(business_auth_gateway::security::hash("consumed-embed-code")).bind(fixture.salesperson).bind(identity_binding_id).bind(workbench_session_id).bind(trace_id).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO business_sessions(id,session_token_hash,csrf_token_hash,enterprise_user_id,identity_binding_id,workbench_session_id,embed_session_id,status,expires_at,trace_id) VALUES($1,$2,$3,$4,$5,$6,$7,'active',now()+interval '1 hour',$8)")
        .bind(business_session_id).bind(business_auth_gateway::security::hash(session_token)).bind(business_auth_gateway::security::hash(csrf_token)).bind(fixture.salesperson).bind(identity_binding_id).bind(workbench_session_id).bind(embed_session_id).bind(trace_id).execute(&pool).await.unwrap();
    let app = router(AppState::new(store, &config));

    for (origin, csrf, expected_code) in [
        (
            Some("https://attacker.example"),
            Some(csrf_token),
            "origin_rejected",
        ),
        (Some("https://business.example.test"), None, "csrf_required"),
        (
            Some("https://business.example.test"),
            Some("wrong-csrf"),
            "csrf_rejected",
        ),
    ] {
        let mut request = Request::post("/api/v1/purchase-orders")
            .header("content-type", "application/json")
            .header("cookie", format!("__Host-bizfin_business={session_token}"));
        if let Some(origin) = origin {
            request = request.header("origin", origin);
        }
        if let Some(csrf) = csrf {
            request = request.header("x-csrf-token", csrf);
        }
        let response = app
            .clone()
            .oneshot(request.body(Body::from("{}")).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), 403);
        let body = to_bytes(response.into_body(), 4096).await.unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&body).unwrap()["code"],
            expected_code
        );
    }
    let accepted_by_browser_security = app
        .clone()
        .oneshot(
            Request::post("/api/v1/purchase-orders")
                .header("content-type", "application/json")
                .header("origin", "https://business.example.test")
                .header("x-csrf-token", csrf_token)
                .header("cookie", format!("__Host-bizfin_business={session_token}"))
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(accepted_by_browser_security.status(), 422);

    let unauthorized = app
        .clone()
        .oneshot(
            Request::post("/v1/authorization/access-check")
                .header("content-type", "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(unauthorized.status(), 401);
    let authorized = app
        .oneshot(
            Request::post("/v1/authorization/access-check")
                .header("content-type", "application/json")
                .header("x-business-service-credential", credential)
                .header("x-service-audience", "business-core")
                .header("x-enterprise-user-id", fixture.salesperson.to_string())
                .header("x-trace-id", Uuid::new_v4().to_string())
                .body(Body::from(
                    json!({
                        "enterpriseUserId": fixture.salesperson,
                        "permissionKey": "business_master_data:read",
                        "resourceType": "warehouse",
                        "resourceId": fixture.warehouse_one
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(authorized.status(), 200);
}
