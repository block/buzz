use business_core::{model::BootstrapRequest, router, AppState, Config, PgStore};
use reqwest::Client;
use serde_json::json;
use sqlx::postgres::PgPoolOptions;
use std::time::{Duration, Instant};
use uuid::Uuid;

const SALES_ORDERS: i64 = 20_000;
const SHIPMENTS: i64 = 12_000;
const PURCHASE_ORDERS: i64 = 10_000;
const RECEIVED_PURCHASE_ORDERS: i64 = 6_000;
const SKU_LOCATIONS: i64 = 4_999;
const PROFIT_FACTS: i64 = SHIPMENTS * 2;
const SAMPLES: usize = 200;

#[tokio::test]
#[ignore = "capacity test requires a dedicated empty PostgreSQL database"]
async fn representative_s1_http_capacity() {
    let database_url = std::env::var("BUSINESS_CORE_S1_CAPACITY_DATABASE_URL")
        .expect("BUSINESS_CORE_S1_CAPACITY_DATABASE_URL must target a dedicated empty database");
    assert!(
        database_url.contains("s1_capacity"),
        "capacity database name must contain s1_capacity"
    );
    let pool = PgPoolOptions::new()
        .max_connections(10)
        .connect(&database_url)
        .await
        .unwrap();
    let store = PgStore::new(pool.clone());
    store.migrate().await.unwrap();
    let existing_users: i64 = sqlx::query_scalar("SELECT count(*) FROM enterprise_users")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(existing_users, 0, "capacity database must be empty");

    let actor = Uuid::new_v4();
    let legal_entity = Uuid::new_v4();
    let ledger = Uuid::new_v4();
    let business_unit = Uuid::new_v4();
    let department = Uuid::new_v4();
    let uom = Uuid::new_v4();
    let category = Uuid::new_v4();
    let brand = Uuid::new_v4();
    let warehouse = Uuid::new_v4();
    let customer = Uuid::new_v4();
    let supplier = Uuid::new_v4();
    let product = Uuid::new_v4();
    let initial_sku = Uuid::new_v4();
    let salesperson = Uuid::new_v4();
    let role = Uuid::new_v4();
    sqlx::query("INSERT INTO enterprise_users(id,oidc_issuer,oidc_subject,email,display_name) VALUES($1,'https://capacity.test','capacity-operator','capacity@example.test','Capacity Operator')")
        .bind(actor)
        .execute(&pool)
        .await
        .unwrap();
    let bootstrap: BootstrapRequest = serde_json::from_value(json!({
        "group":{"id":Uuid::new_v4(),"code":"CAP_GROUP","name":"S1 Capacity Group","baseCurrency":"CNY","timezone":"Asia/Shanghai"},
        "legalEntities":[{"id":legal_entity,"code":"CAP_LE","name":"Capacity Trading","countryCode":"CN","functionalCurrency":"CNY"}],
        "ledgerBooks":[{"id":ledger,"legalEntityId":legal_entity,"code":"CAP_BOOK","name":"Capacity Operating Book","currency":"CNY","fiscalYearStartMonth":1,"isPrimary":true}],
        "businessUnits":[{"id":business_unit,"legalEntityId":legal_entity,"code":"CAP_BU","name":"Capacity Business"}],
        "departments":[{"id":department,"businessUnitId":business_unit,"code":"CAP_OPS","name":"Operations"}],
        "unitsOfMeasure":[{"id":uom,"code":"CAP_EA","name":"Each","precisionScale":0}],
        "productCategories":[{"id":category,"code":"CAP_CAT","name":"Capacity Products"}],
        "brands":[{"id":brand,"code":"CAP_BRAND","name":"Capacity Brand"}],
        "warehouses":[{"id":warehouse,"legalEntityId":legal_entity,"businessUnitId":business_unit,"code":"CAP_WH","name":"Capacity Warehouse"}],
        "customers":[{"id":customer,"legalEntityId":legal_entity,"businessUnitId":business_unit,"code":"CAP_CUS","name":"Capacity Customer","creditCurrency":"CNY","creditLimitMinor":999999999999i64}],
        "suppliers":[{"id":supplier,"legalEntityId":legal_entity,"businessUnitId":business_unit,"code":"CAP_SUP","name":"Capacity Supplier"}],
        "products":[{"id":product,"code":"CAP_PROD","name":"Capacity Product","categoryId":category,"brandId":brand,"baseUomId":uom}],
        "skus":[{"id":initial_sku,"productId":product,"code":"CAP_SKU_00001","name":"Capacity SKU 1","barcode":"8800000000001"}],
        "salespeople":[{"id":salesperson,"enterpriseUserId":actor,"businessUnitId":business_unit,"code":"CAP_SP","name":"Capacity Operator"}],
        "roles":[{"id":role,"roleKey":"capacity_report_reader","name":"Capacity Report Reader","permissionKeys":["management_report:read"]}],
        "userRoles":[{"enterpriseUserId":actor,"roleId":role}],
        "scopes":[
            {"enterpriseUserId":actor,"dimension":"legal_entity","resourceId":legal_entity},
            {"enterpriseUserId":actor,"dimension":"warehouse","resourceId":warehouse},
            {"enterpriseUserId":actor,"dimension":"customer","resourceId":customer},
            {"enterpriseUserId":actor,"dimension":"supplier","resourceId":supplier},
            {"enterpriseUserId":actor,"dimension":"brand","resourceId":brand},
            {"enterpriseUserId":actor,"dimension":"business_unit","resourceId":business_unit}
        ],
        "assignmentPolicies":[],"approvalPolicies":[]
    }))
    .unwrap();
    store
        .bootstrap(actor, Uuid::new_v4(), &bootstrap)
        .await
        .unwrap();

    seed_representative_data(
        &pool,
        actor,
        legal_entity,
        business_unit,
        department,
        uom,
        category,
        brand,
        warehouse,
        customer,
        supplier,
        product,
        initial_sku,
    )
    .await;
    sqlx::query("ANALYZE").execute(&pool).await.unwrap();

    let credential = "s1-capacity-service-credential-32-bytes".to_string();
    let config = config(database_url, credential.clone());
    let app = router(AppState::new(store, &config));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = Client::builder().pool_max_idle_per_host(4).build().unwrap();
    let dashboard_url =
        format!("http://{address}/v1/operations/dashboard?managementPeriod=2026-08&currency=CNY");
    let quality_url = format!("http://{address}/v1/operations/data-quality");
    for _ in 0..10 {
        assert_success(&client, &dashboard_url, &credential, actor).await;
        assert_success(&client, &quality_url, &credential, actor).await;
    }
    let mut dashboard = Vec::with_capacity(SAMPLES);
    let mut quality = Vec::with_capacity(SAMPLES);
    for _ in 0..SAMPLES {
        let started = Instant::now();
        let body = assert_success(&client, &dashboard_url, &credential, actor).await;
        dashboard.push(started.elapsed());
        assert_eq!(body["sales"]["orderCount"], SALES_ORDERS);
        assert_eq!(body["purchasing"]["purchaseOrderCount"], PURCHASE_ORDERS);
        assert_eq!(body["inventory"]["skuLocationCount"], SKU_LOCATIONS);
        assert_eq!(body["reportHealth"]["status"], "complete");
        assert_eq!(body["run"]["status"], "completed");
        assert_eq!(body["diagnostics"]["status"], "healthy");
        assert!(body["diagnostics"]["slowestStage"].is_string());
        let started = Instant::now();
        let body = assert_success(&client, &quality_url, &credential, actor).await;
        quality.push(started.elapsed());
        assert_eq!(body["status"], "complete");
        assert_eq!(body["differenceCount"], 0);
        assert_eq!(body["alerts"], json!([]));
        assert_eq!(body["run"]["status"], "completed");
    }
    let dashboard_stats = stats(&mut dashboard);
    let quality_stats = stats(&mut quality);
    eprintln!(
        "{}",
        json!({
            "dataset": {"salesOrders":SALES_ORDERS,"shipments":SHIPMENTS,"purchaseOrders":PURCHASE_ORDERS,"receivedPurchaseOrders":RECEIVED_PURCHASE_ORDERS,"skuLocations":SKU_LOCATIONS,"profitFacts":PROFIT_FACTS},
            "samplesPerRoute": SAMPLES,
            "dashboardMs": dashboard_stats,
            "dataQualityMs": quality_stats,
            "thresholdMs": {"dashboardP95":500,"managementAggregateP95":2000}
        })
    );
    assert!(dashboard_stats.1 < 500.0, "dashboard P95 exceeded 500ms");
    assert!(quality_stats.1 < 2_000.0, "data-quality P95 exceeded 2s");
    server.abort();
}

#[allow(clippy::too_many_arguments)]
async fn seed_representative_data(
    pool: &sqlx::PgPool,
    actor: Uuid,
    legal_entity: Uuid,
    business_unit: Uuid,
    department: Uuid,
    uom: Uuid,
    category: Uuid,
    brand: Uuid,
    warehouse: Uuid,
    customer: Uuid,
    supplier: Uuid,
    product: Uuid,
    initial_sku: Uuid,
) {
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("INSERT INTO business_skus(id,product_id,code,name,status) SELECT md5('cap-sku-'||g)::uuid,$1,'CAP_SKU_'||lpad(g::text,5,'0'),'Capacity SKU '||g,'active' FROM generate_series(2,5000) g")
        .bind(product).execute(&mut *tx).await.unwrap();
    sqlx::query("INSERT INTO sales_orders(id,order_number,legal_entity_id,customer_id,salesperson_user_id,business_unit_id,department_id,brand_id,currency,order_date,payment_terms_days,payment_terms_snapshot,lifecycle_status,hold_status,fulfillment_status,subtotal_amount,discount_amount,net_amount,tax_amount,gross_amount,created_by_user_id,updated_by_user_id,confirmed_at,completed_at,version,trace_id) SELECT md5('cap-so-'||g)::uuid,'CAP-SO-'||lpad(g::text,6,'0'),$1,$2,$3,$4,$5,$6,'CNY',DATE '2026-08-01'+((g-1)%21)::int,30,'{\"days\":30}'::jsonb,CASE WHEN g<=12000 THEN 'completed' ELSE 'confirmed' END,'none',CASE WHEN g<=12000 THEN 'shipped' ELSE 'reserved' END,100,0,100,13,113,$3,$3,now(),CASE WHEN g<=12000 THEN now() ELSE NULL END,2,md5('cap-so-trace-'||g)::uuid FROM generate_series(1,20000) g")
        .bind(legal_entity).bind(customer).bind(actor).bind(business_unit).bind(department).bind(brand).execute(&mut *tx).await.unwrap();
    sqlx::query("INSERT INTO sales_order_lines(id,sales_order_id,line_number,sku_id,warehouse_id,unit_of_measure_id,ordered_quantity,reserved_quantity,shipped_quantity,unit_price,net_amount,tax_rate,tax_amount,gross_amount,business_unit_id,department_id,brand_id) SELECT md5('cap-sol-'||g)::uuid,md5('cap-so-'||g)::uuid,1,$1,$2,$3,1,0,CASE WHEN g<=12000 THEN 1 ELSE 0 END,100,100,0.13,13,113,$4,$5,$6 FROM generate_series(1,20000) g")
        .bind(initial_sku).bind(warehouse).bind(uom).bind(business_unit).bind(department).bind(brand).execute(&mut *tx).await.unwrap();
    sqlx::query("INSERT INTO inventory_reservations(id,sales_order_id,sales_order_line_id,legal_entity_id,warehouse_id,sku_id,reserved_quantity,consumed_quantity,status,trace_id) SELECT md5('cap-res-'||g)::uuid,md5('cap-so-'||g)::uuid,md5('cap-sol-'||g)::uuid,$1,$2,$3,1,1,'consumed',md5('cap-res-trace-'||g)::uuid FROM generate_series(1,12000) g")
        .bind(legal_entity).bind(warehouse).bind(initial_sku).execute(&mut *tx).await.unwrap();
    sqlx::query("INSERT INTO shipments(id,shipment_number,sales_order_id,legal_entity_id,warehouse_id,customer_id,shipment_date,status,sales_amount,cost_amount,currency,created_by_user_id,confirmed_by_user_id,confirmed_at,version,trace_id) SELECT md5('cap-shp-'||g)::uuid,'CAP-SHP-'||lpad(g::text,6,'0'),md5('cap-so-'||g)::uuid,$1,$2,$3,DATE '2026-08-01'+((g-1)%21)::int,'confirmed',100,60,'CNY',$4,$4,now(),2,md5('cap-shp-trace-'||g)::uuid FROM generate_series(1,12000) g")
        .bind(legal_entity).bind(warehouse).bind(customer).bind(actor).execute(&mut *tx).await.unwrap();
    sqlx::query("INSERT INTO shipment_lines(id,shipment_id,sales_order_line_id,sku_id,quantity,sales_amount,unit_cost,total_cost,cost_snapshot_at,inventory_reservation_id) SELECT md5('cap-shpl-'||g)::uuid,md5('cap-shp-'||g)::uuid,md5('cap-sol-'||g)::uuid,$1,1,100,60,60,now(),md5('cap-res-'||g)::uuid FROM generate_series(1,12000) g")
        .bind(initial_sku).execute(&mut *tx).await.unwrap();
    sqlx::query("INSERT INTO profit_facts(id,metric_type,direction,amount,currency,quantity,legal_entity_id,sales_order_id,sales_order_line_id,shipment_id,shipment_line_id,customer_id,sku_id,product_category_id,brand_id,salesperson_user_id,business_unit_id,department_id,warehouse_id,business_date,management_period,source_system,source_type,source_id,source_line_id,source_event_id,source_event_version,data_as_of,trace_id) SELECT md5('cap-pf-'||m.metric||'-'||g)::uuid,m.metric,'normal',m.amount,'CNY',1,$1,md5('cap-so-'||g)::uuid,md5('cap-sol-'||g)::uuid,md5('cap-shp-'||g)::uuid,md5('cap-shpl-'||g)::uuid,$2,$3,$4,$5,$6,$7,$8,$9,DATE '2026-08-01'+((g-1)%21)::int,'2026-08','business_core_b2','shipment',md5('cap-shp-'||g)::uuid,md5('cap-shpl-'||g)::uuid,md5('cap-shp-event-'||g)::uuid,1,now(),md5('cap-pf-trace-'||g)::uuid FROM generate_series(1,12000) g CROSS JOIN (VALUES('net_revenue'::text,100::numeric),('product_cost'::text,60::numeric)) m(metric,amount)")
        .bind(legal_entity).bind(customer).bind(initial_sku).bind(category).bind(brand).bind(actor).bind(business_unit).bind(department).bind(warehouse).execute(&mut *tx).await.unwrap();
    sqlx::query("INSERT INTO purchase_orders(id,purchase_order_number,legal_entity_id,supplier_id,buyer_user_id,business_unit_id,department_id,brand_id,currency,order_date,payment_terms_days,payment_terms_snapshot,lifecycle_status,receiving_status,subtotal_amount,discount_amount,net_amount,tax_amount,gross_amount,created_by_user_id,updated_by_user_id,confirmed_at,completed_at,version,trace_id) SELECT md5('cap-po-'||g)::uuid,'CAP-PO-'||lpad(g::text,6,'0'),$1,$2,$3,$4,$5,$6,'CNY',DATE '2026-08-01'+((g-1)%21)::int,30,'{\"days\":30}'::jsonb,CASE WHEN g<=6000 THEN 'completed' ELSE 'confirmed' END,CASE WHEN g<=6000 THEN 'received' ELSE 'unreceived' END,80,0,80,10.4,90.4,$3,$3,now(),CASE WHEN g<=6000 THEN now() ELSE NULL END,2,md5('cap-po-trace-'||g)::uuid FROM generate_series(1,10000) g")
        .bind(legal_entity).bind(supplier).bind(actor).bind(business_unit).bind(department).bind(brand).execute(&mut *tx).await.unwrap();
    sqlx::query("INSERT INTO purchase_order_lines(id,purchase_order_id,line_number,sku_id,warehouse_id,unit_of_measure_id,ordered_quantity,received_quantity,unit_price,net_amount,tax_rate,tax_amount,gross_amount,received_net_amount,received_tax_amount,received_gross_amount,provisional_inventory_cost_amount,business_unit_id,department_id,brand_id) SELECT md5('cap-pol-'||g)::uuid,md5('cap-po-'||g)::uuid,1,$1,$2,$3,1,CASE WHEN g<=6000 THEN 1 ELSE 0 END,80,80,0.13,10.4,90.4,CASE WHEN g<=6000 THEN 80 ELSE 0 END,CASE WHEN g<=6000 THEN 10.4 ELSE 0 END,CASE WHEN g<=6000 THEN 90.4 ELSE 0 END,CASE WHEN g<=6000 THEN 80 ELSE 0 END,$4,$5,$6 FROM generate_series(1,10000) g")
        .bind(initial_sku).bind(warehouse).bind(uom).bind(business_unit).bind(department).bind(brand).execute(&mut *tx).await.unwrap();
    sqlx::query("INSERT INTO inventory_movements(id,legal_entity_id,warehouse_id,sku_id,movement_type,quantity,unit_cost,total_cost,currency,source_type,source_id,source_line_id,business_date,created_by_user_id,trace_id) SELECT md5('cap-move-'||g)::uuid,$1,$2,md5('cap-sku-'||g)::uuid,'opening_balance',10,25,250,'CNY','capacity_seed',md5('cap-source-'||g)::uuid,md5('cap-source-line-'||g)::uuid,DATE '2026-08-01',$3,md5('cap-move-trace-'||g)::uuid FROM generate_series(2,5000) g")
        .bind(legal_entity).bind(warehouse).bind(actor).execute(&mut *tx).await.unwrap();
    sqlx::query("INSERT INTO inventory_balances(legal_entity_id,warehouse_id,sku_id,on_hand_quantity,reserved_quantity,inventory_value,average_unit_cost,last_movement_id) SELECT $1,$2,md5('cap-sku-'||g)::uuid,10,0,250,25,md5('cap-move-'||g)::uuid FROM generate_series(2,5000) g")
        .bind(legal_entity).bind(warehouse).execute(&mut *tx).await.unwrap();
    sqlx::query("INSERT INTO profit_projection_offsets(consumer_name,last_fact_sequence,updated_at,version) VALUES('profit_projection_v1',(SELECT max(fact_sequence) FROM profit_facts),now(),1) ON CONFLICT(consumer_name) DO UPDATE SET last_fact_sequence=EXCLUDED.last_fact_sequence,updated_at=now(),version=profit_projection_offsets.version+1")
        .execute(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
}

async fn assert_success(
    client: &Client,
    url: &str,
    credential: &str,
    actor: Uuid,
) -> serde_json::Value {
    let response = client
        .get(url)
        .header("x-business-service-credential", credential)
        .header("x-service-audience", "business-core")
        .header("x-enterprise-user-id", actor.to_string())
        .header("x-trace-id", Uuid::new_v4().to_string())
        .send()
        .await
        .unwrap();
    let status = response.status();
    if status != 200 {
        panic!(
            "capacity request failed with {status}: {}",
            response.text().await.unwrap()
        );
    }
    response.json().await.unwrap()
}

fn stats(samples: &mut [Duration]) -> (f64, f64, f64) {
    samples.sort_unstable();
    let milliseconds = |sample: Duration| sample.as_secs_f64() * 1_000.0;
    (
        milliseconds(samples[(samples.len() - 1) * 50 / 100]),
        milliseconds(samples[(samples.len() - 1) * 95 / 100]),
        milliseconds(*samples.last().unwrap()),
    )
}

fn config(database_url: String, service_credential: String) -> Config {
    Config {
        database_url,
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        service_credential,
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
    }
}
