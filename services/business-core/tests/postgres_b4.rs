use business_core::{
    b2::{
        model::{
            CreateInventoryOpening, CreateSalesOrder, CreateShipment, DecimalString,
            InventoryOpeningLineInput, SalesOrderLineInput, ShipmentLineInput,
            VersionCommand as B2VersionCommand,
        },
        InventoryService, SalesService,
    },
    b4::{
        model::{
            AdjustmentLineInput, CreateAdjustmentBatch, FixedWeightInput, GenerateReportSnapshot,
            PostAdjustment, VersionCommand,
        },
        AdjustmentService, ProfitProjectionService, ProfitReportingService,
    },
    s1::{
        CreateSubscription, GenerateOperatingSnapshot, IncidentCommand, OperationsService,
        SubscriptionCommand,
    },
    PgStore,
};
use chrono::NaiveDate;
use rust_decimal::Decimal;
use serde_json::json;
use sqlx::postgres::PgPoolOptions;
use std::str::FromStr;
use uuid::Uuid;

struct Fixture {
    actor: Uuid,
    legal_entity: Uuid,
    business_unit: Uuid,
    warehouse: Uuid,
    customer: Uuid,
    brand: Uuid,
    uom: Uuid,
    sku: Uuid,
}

#[tokio::test]
async fn b4_postgres_profit_projection_adjustment_reporting_and_concurrency() {
    let Ok(database_url) = std::env::var("BUSINESS_CORE_B4_TEST_DATABASE_URL") else {
        eprintln!("skipping: BUSINESS_CORE_B4_TEST_DATABASE_URL is not set");
        return;
    };
    let pool = PgPoolOptions::new()
        .max_connections(16)
        .connect(&database_url)
        .await
        .unwrap();
    let store = PgStore::new(pool.clone());
    store.migrate().await.unwrap();
    let f = seed(&pool).await;
    let date = NaiveDate::from_ymd_opt(2026, 8, 21).unwrap();
    let sales = SalesService::new(store.clone(), "SO".into(), "SHP".into(), 30);
    let inventory = InventoryService::new(store.clone(), "OPEN".into(), "AR".into());
    let projection = ProfitProjectionService::new(store.clone());
    let adjustments = AdjustmentService::new(store.clone(), "ADJ".into(), 500);
    let reporting = ProfitReportingService::new(store.clone(), "MGR".into(), 1000, true, 60);
    let operations = OperationsService::new(store, true, 60);

    let opening = inventory
        .create_opening(
            f.actor,
            Uuid::new_v4(),
            "b4-opening-create-0001",
            &CreateInventoryOpening {
                legal_entity_id: f.legal_entity,
                business_date: date,
                currency: "CNY".into(),
                lines: vec![InventoryOpeningLineInput {
                    warehouse_id: f.warehouse,
                    sku_id: f.sku,
                    quantity: dec("30"),
                    unit_cost: dec("60"),
                }],
            },
        )
        .await
        .unwrap();
    inventory
        .post_opening(
            f.actor,
            Uuid::new_v4(),
            opening.id,
            "b4-opening-post-0001",
            &b2_version(1),
        )
        .await
        .unwrap();

    let first = shipped_order(
        &sales, &inventory, &f, &pool, date, "first", "10", "100", "100",
    )
    .await;
    let second = shipped_order(
        &sales, &inventory, &f, &pool, date, "second", "5", "100", "0",
    )
    .await;
    let projected = projection
        .project_pending(f.actor, Uuid::new_v4(), 100)
        .await
        .unwrap();
    assert_eq!(projected["factsProjected"], 4);
    assert_eq!(
        sqlx::query_scalar::<_, Decimal>(
            "SELECT net_revenue FROM order_profit_current WHERE sales_order_id=$1",
        )
        .bind(first.0)
        .fetch_one(&pool)
        .await
        .unwrap(),
        decimal("900")
    );
    assert_eq!(
        sqlx::query_scalar::<_, Decimal>(
            "SELECT product_cost FROM order_profit_current WHERE sales_order_id=$1",
        )
        .bind(first.0)
        .fetch_one(&pool)
        .await
        .unwrap(),
        decimal("600")
    );
    let heartbeat_version_before: i64 = sqlx::query_scalar(
        "SELECT version FROM profit_projection_offsets WHERE consumer_name='profit_projection_v1'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        projection
            .project_pending(f.actor, Uuid::new_v4(), 100)
            .await
            .unwrap()["factsProjected"],
        0
    );
    let heartbeat_version_after: i64 = sqlx::query_scalar(
        "SELECT version FROM profit_projection_offsets WHERE consumer_name='profit_projection_v1'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(heartbeat_version_after > heartbeat_version_before);

    let freight = adjustments
        .create(
            f.actor,
            Uuid::new_v4(),
            "b4-freight-create-0001",
            &batch(&f, date, "outbound_freight", "50", "direct", vec![first.0]),
        )
        .await
        .unwrap();
    let freight_preview = adjustments
        .preview(
            f.actor,
            Uuid::new_v4(),
            freight.id,
            "b4-freight-preview-0001",
            &VersionCommand {
                expected_version: 1,
            },
        )
        .await
        .unwrap();
    adjustments
        .post(
            f.actor,
            Uuid::new_v4(),
            freight.id,
            "b4-freight-post-0001",
            &PostAdjustment {
                expected_version: freight_preview.batch_version,
                preview_id: freight_preview.preview_id,
                preview_hash: freight_preview.preview_hash.clone(),
            },
        )
        .await
        .unwrap();
    let profit = reporting
        .order_profits(f.actor, Some(first.0), None, Some("2026-08"), 10)
        .await
        .unwrap();
    assert_eq!(profit["items"][0]["grossProfit"], "300.000000");
    assert_eq!(profit["items"][0]["contributionProfit"], "250.000000");

    let operating = adjustments
        .create(
            f.actor,
            Uuid::new_v4(),
            "b4-operating-create-0001",
            &batch(
                &f,
                date,
                "allocated_operating_expense",
                "30",
                "net_revenue",
                vec![first.0, second.0],
            ),
        )
        .await
        .unwrap();
    let operating_preview = adjustments
        .preview(
            f.actor,
            Uuid::new_v4(),
            operating.id,
            "b4-operating-preview-0001",
            &VersionCommand {
                expected_version: 1,
            },
        )
        .await
        .unwrap();
    adjustments
        .post(
            f.actor,
            Uuid::new_v4(),
            operating.id,
            "b4-operating-post-0001",
            &PostAdjustment {
                expected_version: operating_preview.batch_version,
                preview_id: operating_preview.preview_id,
                preview_hash: operating_preview.preview_hash,
            },
        )
        .await
        .unwrap();
    let allocated: Decimal = sqlx::query_scalar(
        "SELECT sum(allocated_amount) FROM operational_adjustment_allocations WHERE batch_id=$1",
    )
    .bind(operating.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(allocated, decimal("30"));

    let race = adjustments
        .create(
            f.actor,
            Uuid::new_v4(),
            "b4-race-create-0001",
            &batch(&f, date, "platform_fee", "10", "direct", vec![second.0]),
        )
        .await
        .unwrap();
    let race_preview = adjustments
        .preview(
            f.actor,
            Uuid::new_v4(),
            race.id,
            "b4-race-preview-0001",
            &VersionCommand {
                expected_version: 1,
            },
        )
        .await
        .unwrap();
    let post = PostAdjustment {
        expected_version: race_preview.batch_version,
        preview_id: race_preview.preview_id,
        preview_hash: race_preview.preview_hash,
    };
    let left = adjustments.post(f.actor, Uuid::new_v4(), race.id, "b4-race-post-0001", &post);
    let right = adjustments.post(f.actor, Uuid::new_v4(), race.id, "b4-race-post-0002", &post);
    let (left, right) = tokio::join!(left, right);
    assert_eq!(usize::from(left.is_ok()) + usize::from(right.is_ok()), 1);

    for dimension in [
        "customer",
        "sku",
        "product_category",
        "brand",
        "salesperson",
        "business_unit",
        "legal_entity",
    ] {
        let result = reporting
            .profitability(f.actor, "2026-08", "CNY", dimension, None, 100)
            .await
            .unwrap();
        assert!(
            !result["items"].as_array().unwrap().is_empty(),
            "{dimension}"
        );
    }
    let cross = reporting
        .profitability(f.actor, "2026-08", "CNY", "customer", Some("brand"), 100)
        .await
        .unwrap();
    assert_eq!(cross["items"].as_array().unwrap().len(), 1);
    let change = reporting
        .profit_change(
            f.actor,
            NaiveDate::from_ymd_opt(2026, 7, 1).unwrap(),
            NaiveDate::from_ymd_opt(2026, 7, 31).unwrap(),
            NaiveDate::from_ymd_opt(2026, 8, 1).unwrap(),
            NaiveDate::from_ymd_opt(2026, 8, 31).unwrap(),
            "CNY",
        )
        .await
        .unwrap();
    assert_eq!(change["unexplainedDifference"], "0.00");
    assert_ne!(change["change"], "0");

    let snapshot = reporting
        .generate_snapshot(
            f.actor,
            Uuid::new_v4(),
            "b4-snapshot-create-0001",
            &GenerateReportSnapshot {
                report_type: "management_profit_statement".into(),
                management_period: "2026-08".into(),
                currency: "CNY".into(),
                legal_entity_ids: vec![f.legal_entity],
                supersedes_snapshot_id: None,
            },
        )
        .await
        .unwrap();
    let old_hash: String =
        sqlx::query_scalar("SELECT source_hash FROM management_report_snapshots WHERE id=$1")
            .bind(snapshot.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    adjustments
        .reverse(
            f.actor,
            Uuid::new_v4(),
            freight.id,
            "b4-freight-reverse-0001",
            &VersionCommand {
                expected_version: 3,
            },
        )
        .await
        .unwrap();
    let second_snapshot = reporting
        .generate_snapshot(
            f.actor,
            Uuid::new_v4(),
            "b4-snapshot-create-0002",
            &GenerateReportSnapshot {
                report_type: "management_profit_statement".into(),
                management_period: "2026-08".into(),
                currency: "CNY".into(),
                legal_entity_ids: vec![f.legal_entity],
                supersedes_snapshot_id: Some(snapshot.id),
            },
        )
        .await
        .unwrap();
    assert_ne!(snapshot.id, second_snapshot.id);
    assert_eq!(
        old_hash,
        sqlx::query_scalar::<_, String>(
            "SELECT source_hash FROM management_report_snapshots WHERE id=$1"
        )
        .bind(snapshot.id)
        .fetch_one(&pool)
        .await
        .unwrap()
    );
    assert!(projection.reconcile(f.actor).await.unwrap()["consistent"]
        .as_bool()
        .unwrap());
    assert!(sqlx::query("UPDATE profit_facts SET amount=0")
        .execute(&pool)
        .await
        .is_err());
    assert!(
        sqlx::query("UPDATE management_report_snapshot_rows SET row_key='tampered'")
            .execute(&pool)
            .await
            .is_err()
    );
    assert!(reporting
        .management_report(f.actor, "2026-08", "CNY")
        .await
        .unwrap()["warnings"][0]
        .as_str()
        .unwrap()
        .contains("不是法定会计报表"));
    let dashboard = operations
        .dashboard(f.actor, "2026-08", "CNY")
        .await
        .unwrap();
    assert_eq!(dashboard["sales"]["orderCount"], 2);
    assert_eq!(dashboard["sales"]["fulfillmentRate"], "1.00000000");
    assert_eq!(dashboard["inventory"]["inventoryValue"], "900.000000");
    assert!(dashboard["profit"]["sourceWatermark"]
        .as_i64()
        .is_some_and(|value| value > 0));
    assert_eq!(dashboard["reportHealth"]["status"], "complete");
    assert_eq!(dashboard["reportHealth"]["alerts"], json!([]));
    assert_eq!(dashboard["diagnostics"]["status"], "healthy");
    assert!(dashboard["diagnostics"]["slowestStage"].is_string());
    let operating_snapshot = operations
        .generate_operating_snapshot(
            f.actor,
            Uuid::new_v4(),
            "s15-daily-snapshot-20260821",
            &GenerateOperatingSnapshot {
                cadence: "daily".into(),
                currency: "CNY".into(),
                period_start: date,
                utc_offset_minutes: 480,
            },
        )
        .await
        .unwrap();
    assert_eq!(operating_snapshot["created"], true);
    let repeated_snapshot = operations
        .generate_operating_snapshot(
            f.actor,
            Uuid::new_v4(),
            "s15-daily-snapshot-repeat",
            &GenerateOperatingSnapshot {
                cadence: "daily".into(),
                currency: "CNY".into(),
                period_start: date,
                utc_offset_minutes: 480,
            },
        )
        .await
        .unwrap();
    assert_eq!(repeated_snapshot["created"], false);
    assert_eq!(repeated_snapshot["id"], operating_snapshot["id"]);
    let trends = operations
        .operating_trends(f.actor, "daily", "CNY", 14)
        .await
        .unwrap();
    assert_eq!(trends["items"][0]["metrics"]["salesOrderCount"], 2);
    assert_eq!(
        trends["items"][0]["metrics"]["managementOperatingProfit"],
        "500.000000"
    );
    assert!(
        sqlx::query("UPDATE operating_report_snapshots SET data_quality_status='blocked'")
            .execute(&pool)
            .await
            .is_err()
    );
    let subscription = operations
        .create_operating_subscription(
            f.actor,
            Uuid::new_v4(),
            "s15-daily-subscription-create",
            &CreateSubscription {
                cadence: "daily".into(),
                currency: "CNY".into(),
                utc_offset_minutes: 480,
                delivery_hour: 8,
            },
        )
        .await
        .unwrap();
    let subscription_id = Uuid::parse_str(subscription["id"].as_str().unwrap()).unwrap();
    let paused = operations
        .command_operating_subscription(
            f.actor,
            Uuid::new_v4(),
            "s15-daily-subscription-pause",
            subscription_id,
            &SubscriptionCommand {
                action: "pause".into(),
                expected_version: subscription["version"].as_i64().unwrap(),
            },
        )
        .await
        .unwrap();
    assert_eq!(paused["status"], "paused");
    assert_eq!(
        operations
            .list_operating_subscriptions(f.actor)
            .await
            .unwrap()["items"][0]["status"],
        "paused"
    );
    let quality = operations.data_quality(f.actor).await.unwrap();
    assert_eq!(quality["status"], "complete");
    assert_eq!(quality["differenceCount"], 0);
    assert_eq!(quality["projection"]["pendingEvents"], 0);
    assert_eq!(quality["projection"]["pendingFailures"], 0);
    assert!(quality["projection"]["freshnessAgeSeconds"].is_number());
    assert_eq!(quality["alerts"], json!([]));
    let s13_indexes: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pg_indexes WHERE schemaname='public' AND indexname=ANY($1)",
    )
    .bind(vec![
        "sales_orders_operating_dashboard_idx",
        "shipments_operating_dashboard_idx",
        "purchase_orders_operating_dashboard_idx",
        "purchase_order_lines_dashboard_idx",
        "profit_facts_operating_dashboard_idx",
        "profit_projection_failures_pending_idx",
        "business_core_outbox_profit_projection_idx",
    ])
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(s13_indexes, 7);
    sqlx::query("UPDATE inventory_balances SET inventory_value=inventory_value+1 WHERE legal_entity_id=$1 AND warehouse_id=$2 AND sku_id=$3")
        .bind(f.legal_entity).bind(f.warehouse).bind(f.sku).execute(&pool).await.unwrap();
    let drift = operations.data_quality(f.actor).await.unwrap();
    assert_eq!(drift["status"], "blocked");
    assert_eq!(drift["checks"][0]["differenceCount"], 1);
    assert_eq!(drift["alerts"][0]["severity"], "critical");
    let scan_trace = Uuid::new_v4();
    let scan = operations
        .scan_incidents(f.actor, scan_trace, "s14-incident-scan-drift")
        .await
        .unwrap();
    assert_eq!(scan["createdCount"], 1);
    assert_eq!(scan["activeAlertCount"], 1);
    assert_eq!(
        operations
            .scan_incidents(f.actor, scan_trace, "s14-incident-scan-drift")
            .await
            .unwrap(),
        scan
    );
    let incidents = operations.list_incidents(f.actor).await.unwrap();
    let incident = &incidents["items"][0];
    assert_eq!(incident["alertCode"], "RECONCILIATION_DIFFERENCE");
    assert_eq!(incident["conditionStatus"], "active");
    assert_eq!(incident["reviewStatus"], "open");
    assert_eq!(incident["overdue"], false);
    assert_eq!(incident["events"][0]["eventType"], "detected");
    let incident_id = Uuid::parse_str(incident["id"].as_str().unwrap()).unwrap();
    let mut version = incident["version"].as_i64().unwrap();
    assert!(matches!(
        operations
            .command_incident(
                f.actor,
                Uuid::new_v4(),
                "s14-incident-stale",
                incident_id,
                IncidentCommand {
                    action: "claim".into(),
                    expected_version: version + 1,
                    due_at: None,
                    note: None,
                },
            )
            .await,
        Err(business_core::b2::DomainError::VersionConflict)
    ));
    let claimed = operations
        .command_incident(
            f.actor,
            Uuid::new_v4(),
            "s14-incident-claim",
            incident_id,
            IncidentCommand {
                action: "claim".into(),
                expected_version: version,
                due_at: None,
                note: Some("由经营负责人跟进".into()),
            },
        )
        .await
        .unwrap();
    assert_eq!(claimed["assigneeUserId"], f.actor.to_string());
    version = claimed["version"].as_i64().unwrap();
    let acknowledged = operations
        .command_incident(
            f.actor,
            Uuid::new_v4(),
            "s14-incident-ack",
            incident_id,
            IncidentCommand {
                action: "acknowledge".into(),
                expected_version: version,
                due_at: None,
                note: None,
            },
        )
        .await
        .unwrap();
    version = acknowledged["version"].as_i64().unwrap();
    let started = operations
        .command_incident(
            f.actor,
            Uuid::new_v4(),
            "s14-incident-start",
            incident_id,
            IncidentCommand {
                action: "start".into(),
                expected_version: version,
                due_at: None,
                note: None,
            },
        )
        .await
        .unwrap();
    version = started["version"].as_i64().unwrap();
    assert!(matches!(
        operations
            .command_incident(
                f.actor,
                Uuid::new_v4(),
                "s14-incident-resolve-too-early",
                incident_id,
                IncidentCommand {
                    action: "resolve".into(),
                    expected_version: version,
                    due_at: None,
                    note: None,
                },
            )
            .await,
        Err(business_core::b2::DomainError::Invalid(_))
    ));
    sqlx::query("UPDATE inventory_balances SET inventory_value=inventory_value-1 WHERE legal_entity_id=$1 AND warehouse_id=$2 AND sku_id=$3")
        .bind(f.legal_entity).bind(f.warehouse).bind(f.sku).execute(&pool).await.unwrap();
    assert_eq!(
        operations.data_quality(f.actor).await.unwrap()["status"],
        "complete"
    );
    let clear_scan = operations
        .scan_incidents(f.actor, Uuid::new_v4(), "s14-incident-scan-clear")
        .await
        .unwrap();
    assert_eq!(clear_scan["clearedCount"], 1);
    let cleared = operations.list_incidents(f.actor).await.unwrap();
    let cleared_incident = &cleared["items"][0];
    assert_eq!(cleared_incident["conditionStatus"], "cleared");
    version = cleared_incident["version"].as_i64().unwrap();
    let resolved = operations
        .command_incident(
            f.actor,
            Uuid::new_v4(),
            "s14-incident-resolve",
            incident_id,
            IncidentCommand {
                action: "resolve".into(),
                expected_version: version,
                due_at: None,
                note: Some("对账恢复一致".into()),
            },
        )
        .await
        .unwrap();
    assert_eq!(resolved["reviewStatus"], "resolved");
    assert!(sqlx::query(
        "UPDATE operating_report_incident_events SET payload='{}' WHERE incident_id=$1"
    )
    .bind(incident_id)
    .execute(&pool)
    .await
    .is_err());
    let lifecycle_audits: i64 = sqlx::query_scalar("SELECT count(*) FROM business_core_audit_events WHERE target_type='operating_report_incident'")
        .fetch_one(&pool).await.unwrap();
    assert!(lifecycle_audits >= 6);
    let outsider = Uuid::new_v4();
    sqlx::query("INSERT INTO enterprise_users(id,oidc_issuer,oidc_subject,display_name) VALUES($1,'https://issuer.test',$2,'S1 No Access')")
        .bind(outsider).bind(outsider.to_string()).execute(&pool).await.unwrap();
    assert!(matches!(
        operations.dashboard(outsider, "2026-08", "CNY").await,
        Err(business_core::b2::DomainError::NotFoundOrForbidden)
    ));
}

#[allow(clippy::too_many_arguments)]
async fn shipped_order(
    sales: &SalesService,
    inventory: &InventoryService,
    f: &Fixture,
    pool: &sqlx::PgPool,
    date: NaiveDate,
    key: &str,
    quantity: &str,
    price: &str,
    discount: &str,
) -> (Uuid, Uuid) {
    let order = sales
        .create_order(
            f.actor,
            Uuid::new_v4(),
            &format!("b4-{key}-order-create"),
            &CreateSalesOrder {
                legal_entity_id: f.legal_entity,
                customer_id: f.customer,
                salesperson_user_id: None,
                business_unit_id: f.business_unit,
                department_id: None,
                brand_id: Some(f.brand),
                currency: "CNY".into(),
                order_date: date,
                requested_delivery_date: Some(date),
                payment_terms_days: None,
                customer_reference: None,
                business_note: None,
                lines: vec![SalesOrderLineInput {
                    sku_id: f.sku,
                    warehouse_id: f.warehouse,
                    unit_of_measure_id: f.uom,
                    quantity: dec(quantity),
                    unit_price: dec(price),
                    discount_amount: dec(discount),
                    tax_rate: dec("0"),
                    business_unit_id: None,
                    department_id: None,
                    brand_id: Some(f.brand),
                }],
            },
        )
        .await
        .unwrap();
    sales
        .confirm_order(
            f.actor,
            Uuid::new_v4(),
            order.id,
            &format!("b4-{key}-order-confirm"),
            &b2_version(1),
        )
        .await
        .unwrap();
    let line: Uuid = sqlx::query_scalar("SELECT id FROM sales_order_lines WHERE sales_order_id=$1")
        .bind(order.id)
        .fetch_one(pool)
        .await
        .unwrap();
    let shipment = sales
        .create_shipment(
            f.actor,
            Uuid::new_v4(),
            &format!("b4-{key}-shipment-create"),
            &CreateShipment {
                sales_order_id: order.id,
                warehouse_id: f.warehouse,
                shipment_date: date,
                lines: vec![ShipmentLineInput {
                    sales_order_line_id: line,
                    quantity: dec(quantity),
                }],
            },
        )
        .await
        .unwrap();
    inventory
        .confirm_shipment(
            f.actor,
            Uuid::new_v4(),
            shipment.id,
            &format!("b4-{key}-shipment-confirm"),
            &b2_version(1),
        )
        .await
        .unwrap();
    (order.id, shipment.id)
}

fn batch(
    f: &Fixture,
    date: NaiveDate,
    metric: &str,
    amount: &str,
    basis: &str,
    orders: Vec<Uuid>,
) -> CreateAdjustmentBatch {
    CreateAdjustmentBatch {
        legal_entity_id: f.legal_entity,
        currency: "CNY".into(),
        management_period: "2026-08".into(),
        lines: vec![AdjustmentLineInput {
            metric_type: metric.into(),
            amount: dec(amount),
            business_date: date,
            allocation_basis: basis.into(),
            direct_sales_order_id: (basis == "direct").then(|| orders[0]),
            customer_id: None,
            sku_id: None,
            brand_id: None,
            salesperson_user_id: None,
            business_unit_id: None,
            department_id: None,
            warehouse_id: None,
            sales_order_ids: if basis == "direct" { vec![] } else { orders },
            fixed_weights: Vec::<FixedWeightInput>::new(),
            reason_code: "TEST_ADJUSTMENT".into(),
            source_reference: None,
            business_note: None,
        }],
    }
}

fn dec(value: &str) -> DecimalString {
    DecimalString(decimal(value))
}
fn decimal(value: &str) -> Decimal {
    Decimal::from_str(value).unwrap()
}
fn b2_version(expected_version: i64) -> B2VersionCommand {
    B2VersionCommand {
        expected_version,
        reason_code: None,
    }
}

async fn seed(pool: &sqlx::PgPool) -> Fixture {
    let f = Fixture {
        actor: Uuid::new_v4(),
        legal_entity: Uuid::new_v4(),
        business_unit: Uuid::new_v4(),
        warehouse: Uuid::new_v4(),
        customer: Uuid::new_v4(),
        brand: Uuid::new_v4(),
        uom: Uuid::new_v4(),
        sku: Uuid::new_v4(),
    };
    let category = Uuid::new_v4();
    let product = Uuid::new_v4();
    let role = Uuid::new_v4();
    sqlx::query("INSERT INTO enterprise_users(id,oidc_issuer,oidc_subject,display_name) VALUES($1,'https://issuer.test',$2,'B4 Operator')").bind(f.actor).bind(f.actor.to_string()).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO business_group_profile(id,code,name,base_currency,timezone) VALUES($1,'B4_GROUP','B4 Test','CNY','Asia/Shanghai')").bind(Uuid::new_v4()).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO business_legal_entities(id,code,name,country_code,functional_currency) VALUES($1,'LE_B4','B4 Legal','CN','CNY')").bind(f.legal_entity).execute(pool).await.unwrap();
    sqlx::query(
        "INSERT INTO business_units(id,legal_entity_id,code,name) VALUES($1,$2,'BU_B4','B4 Trade')",
    )
    .bind(f.business_unit)
    .bind(f.legal_entity)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO business_warehouses(id,legal_entity_id,business_unit_id,code,name) VALUES($1,$2,$3,'WH_B4','B4 Warehouse')").bind(f.warehouse).bind(f.legal_entity).bind(f.business_unit).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO business_customers(id,legal_entity_id,business_unit_id,code,name,credit_currency,payment_terms_days) VALUES($1,$2,$3,'CUS_B4','B4 Customer','CNY',30)").bind(f.customer).bind(f.legal_entity).bind(f.business_unit).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO business_units_of_measure(id,code,name,precision_scale) VALUES($1,'UOM_B4','Each',0)").bind(f.uom).execute(pool).await.unwrap();
    sqlx::query(
        "INSERT INTO business_product_categories(id,code,name) VALUES($1,'CAT_B4','B4 Category')",
    )
    .bind(category)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO business_brands(id,code,name) VALUES($1,'BRAND_B4','B4 Brand')")
        .bind(f.brand)
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO business_products(id,code,name,category_id,brand_id,base_uom_id) VALUES($1,'PROD_B4','B4 Product',$2,$3,$4)").bind(product).bind(category).bind(f.brand).bind(f.uom).execute(pool).await.unwrap();
    sqlx::query(
        "INSERT INTO business_skus(id,product_id,code,name) VALUES($1,$2,'SKU_B4','B4 SKU')",
    )
    .bind(f.sku)
    .bind(product)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO business_roles(id,role_key,name) VALUES($1,'b4_operator','B4 Operator')",
    )
    .bind(role)
    .execute(pool)
    .await
    .unwrap();
    for permission in [
        "sales_order:read",
        "sales_order:create",
        "sales_order:confirm",
        "shipment:create",
        "shipment:confirm",
        "shipment:reverse",
        "inventory:read",
        "inventory_opening:create",
        "inventory_opening:post",
        "profit:read",
        "profit:read_detail",
        "profit_adjustment:read",
        "profit_adjustment:create",
        "profit_adjustment:update_draft",
        "profit_adjustment:preview",
        "profit_adjustment:post",
        "profit_adjustment:reverse",
        "management_report:read",
        "management_report:manage_incidents",
        "management_report:manage_subscriptions",
        "management_report:generate_snapshot",
        "management_report:read_snapshot",
    ] {
        sqlx::query("INSERT INTO business_role_permissions(role_id,permission_key) VALUES($1,$2)")
            .bind(role)
            .bind(permission)
            .execute(pool)
            .await
            .unwrap();
    }
    sqlx::query(
        "INSERT INTO business_user_roles(enterprise_user_id,role_id,assigned_by) VALUES($1,$2,$1)",
    )
    .bind(f.actor)
    .bind(role)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO business_legal_entity_scopes(enterprise_user_id,legal_entity_id,granted_by) VALUES($1,$2,$1)").bind(f.actor).bind(f.legal_entity).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO business_warehouse_scopes(enterprise_user_id,warehouse_id,granted_by) VALUES($1,$2,$1)").bind(f.actor).bind(f.warehouse).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO business_customer_scopes(enterprise_user_id,customer_id,granted_by) VALUES($1,$2,$1)").bind(f.actor).bind(f.customer).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO business_brand_scopes(enterprise_user_id,brand_id,granted_by) VALUES($1,$2,$1)").bind(f.actor).bind(f.brand).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO business_unit_scopes(enterprise_user_id,business_unit_id,granted_by) VALUES($1,$2,$1)").bind(f.actor).bind(f.business_unit).execute(pool).await.unwrap();
    f
}
