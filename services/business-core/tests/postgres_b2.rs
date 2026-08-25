use business_core::{
    b2::{
        model::{
            AllocationInput, ApplyReceipt, CreateCustomerReceipt, CreateInventoryOpening,
            CreateSalesOrder, CreateShipment, DecimalString, InventoryOpeningLineInput,
            ReverseAllocation, SalesOrderLineInput, ShipmentLineInput, VersionCommand,
        },
        DomainError, InventoryService, SalesService, SettlementService,
    },
    PgStore,
};
use chrono::NaiveDate;
use rust_decimal::Decimal;
use sqlx::postgres::PgPoolOptions;
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
async fn b2_postgres_closed_loop_and_concurrency() {
    let Ok(database_url) = std::env::var("BUSINESS_CORE_B2_TEST_DATABASE_URL") else {
        eprintln!("skipping: BUSINESS_CORE_B2_TEST_DATABASE_URL is not set");
        return;
    };
    let pool = PgPoolOptions::new()
        .max_connections(12)
        .connect(&database_url)
        .await
        .unwrap();
    let store = PgStore::new(pool.clone());
    store.migrate().await.unwrap();
    let fixture = seed(&pool).await;
    let sales = SalesService::new(store.clone(), "SO".into(), "SHP".into(), 30);
    let inventory = InventoryService::new(store.clone(), "OPEN".into(), "AR".into());
    let settlement = SettlementService::new(store, "RCPT".into());
    let date = NaiveDate::from_ymd_opt(2026, 8, 21).unwrap();

    let opening = inventory
        .create_opening(
            fixture.actor,
            Uuid::new_v4(),
            "opening-create-0001",
            &CreateInventoryOpening {
                legal_entity_id: fixture.legal_entity,
                business_date: date,
                currency: "CNY".into(),
                lines: vec![InventoryOpeningLineInput {
                    warehouse_id: fixture.warehouse,
                    sku_id: fixture.sku,
                    quantity: dec(10),
                    unit_cost: dec(5),
                }],
            },
        )
        .await
        .unwrap();
    let posted = inventory
        .post_opening(
            fixture.actor,
            Uuid::new_v4(),
            opening.id,
            "opening-post-0001",
            &version(1),
        )
        .await
        .unwrap();
    assert_eq!(posted.status, "posted");
    let replay = inventory
        .post_opening(
            fixture.actor,
            Uuid::new_v4(),
            opening.id,
            "opening-post-0001",
            &version(1),
        )
        .await
        .unwrap();
    assert!(replay.idempotent_replay);

    let first = create_order(&sales, &fixture, date, "order-create-0001").await;
    let second = create_order(&sales, &fixture, date, "order-create-0002").await;
    let preview = sales
        .confirmation_preview(fixture.actor, first.id)
        .await
        .unwrap();
    assert!(preview.can_confirm);
    assert!(preview.all_available);
    assert_eq!(preview.readiness, "ready");
    assert_eq!(preview.lines.len(), 1);
    assert_eq!(preview.lines[0].required_quantity.0, Decimal::from(8));
    assert_eq!(preview.lines[0].available_quantity.0, Decimal::from(10));
    assert_eq!(
        preview.lines[0].expected_reserved_quantity.0,
        Decimal::from(8)
    );
    assert_eq!(preview.lines[0].shortage_quantity.0, Decimal::ZERO);

    sqlx::query("DELETE FROM business_role_permissions WHERE permission_key='sales_order:confirm' AND role_id IN (SELECT role_id FROM business_user_roles WHERE enterprise_user_id=$1)")
        .bind(fixture.actor)
        .execute(&pool)
        .await
        .unwrap();
    let permission_blocked = sales
        .confirmation_preview(fixture.actor, first.id)
        .await
        .unwrap();
    assert!(!permission_blocked.can_confirm);
    assert_eq!(permission_blocked.readiness, "permission_required");
    sqlx::query("INSERT INTO business_role_permissions(role_id,permission_key) SELECT role_id,'sales_order:confirm' FROM business_user_roles WHERE enterprise_user_id=$1")
        .bind(fixture.actor)
        .execute(&pool)
        .await
        .unwrap();

    let expected_a = version(1);
    let expected_b = version(1);
    let a = sales.confirm_order(
        fixture.actor,
        Uuid::new_v4(),
        first.id,
        "order-confirm-0001",
        &expected_a,
    );
    let b = sales.confirm_order(
        fixture.actor,
        Uuid::new_v4(),
        second.id,
        "order-confirm-0002",
        &expected_b,
    );
    let (a, b) = tokio::join!(a, b);
    let (confirmed, rejected) = match (a, b) {
        (Ok(order), Err(DomainError::InsufficientStock(_))) => (order, second.id),
        (Err(DomainError::InsufficientStock(_)), Ok(order)) => (order, first.id),
        outcome => panic!("expected exactly one confirmation: {outcome:?}"),
    };
    let shortage = sales
        .confirmation_preview(fixture.actor, rejected)
        .await
        .unwrap();
    assert!(!shortage.can_confirm);
    assert!(!shortage.all_available);
    assert_eq!(shortage.readiness, "insufficient_stock");
    assert_eq!(shortage.lines[0].available_quantity.0, Decimal::from(2));
    assert_eq!(shortage.lines[0].shortage_quantity.0, Decimal::from(6));
    let cancelled = sales
        .cancel_remaining(
            fixture.actor,
            Uuid::new_v4(),
            rejected,
            "order-cancel-remaining-0001",
            &version(1),
        )
        .await
        .unwrap();
    assert_eq!(cancelled.status, "cancelled");
    let reserved: Decimal = sqlx::query_scalar(
        "SELECT reserved_quantity FROM inventory_balances WHERE warehouse_id=$1 AND sku_id=$2",
    )
    .bind(fixture.warehouse)
    .bind(fixture.sku)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(reserved, Decimal::from(8));

    let shipment_options = sales
        .shipment_draft_options(fixture.actor, 100)
        .await
        .unwrap();
    assert!(shipment_options.can_create);
    assert_eq!(shipment_options.items.len(), 1);
    assert_eq!(
        shipment_options.items[0].shippable_quantity.0,
        Decimal::from(8)
    );
    assert_eq!(
        shipment_options.items[0].draft_allocated_quantity.0,
        Decimal::ZERO
    );
    sqlx::query("DELETE FROM business_role_permissions WHERE permission_key='shipment:create' AND role_id IN (SELECT role_id FROM business_user_roles WHERE enterprise_user_id=$1)")
        .bind(fixture.actor)
        .execute(&pool)
        .await
        .unwrap();
    let shipment_permission_blocked = sales
        .shipment_draft_options(fixture.actor, 100)
        .await
        .unwrap();
    assert!(!shipment_permission_blocked.can_create);
    assert_eq!(shipment_permission_blocked.items.len(), 1);
    sqlx::query("INSERT INTO business_role_permissions(role_id,permission_key) SELECT role_id,'shipment:create' FROM business_user_roles WHERE enterprise_user_id=$1")
        .bind(fixture.actor)
        .execute(&pool)
        .await
        .unwrap();

    let hold = sales
        .set_hold(
            fixture.actor,
            Uuid::new_v4(),
            confirmed.id,
            "order-hold-0001",
            &VersionCommand {
                expected_version: 2,
                reason_code: Some("CREDIT_REVIEW".into()),
            },
            true,
        )
        .await
        .unwrap();
    let line_id: Uuid =
        sqlx::query_scalar("SELECT id FROM sales_order_lines WHERE sales_order_id=$1")
            .bind(confirmed.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    let held_shipment = sales
        .create_shipment(
            fixture.actor,
            Uuid::new_v4(),
            "shipment-held-0001",
            &CreateShipment {
                sales_order_id: confirmed.id,
                warehouse_id: fixture.warehouse,
                shipment_date: date,
                lines: vec![ShipmentLineInput {
                    sales_order_line_id: line_id,
                    quantity: dec(3),
                }],
            },
        )
        .await;
    assert!(matches!(held_shipment, Err(DomainError::OrderOnHold)));
    assert!(sales
        .shipment_draft_options(fixture.actor, 100)
        .await
        .unwrap()
        .items
        .is_empty());
    let released = sales
        .set_hold(
            fixture.actor,
            Uuid::new_v4(),
            confirmed.id,
            "order-release-hold-0001",
            &VersionCommand {
                expected_version: hold.version,
                reason_code: Some("REVIEW_COMPLETE".into()),
            },
            false,
        )
        .await
        .unwrap();
    let shipment = sales
        .create_shipment(
            fixture.actor,
            Uuid::new_v4(),
            "shipment-create-0001",
            &CreateShipment {
                sales_order_id: confirmed.id,
                warehouse_id: fixture.warehouse,
                shipment_date: date,
                lines: vec![ShipmentLineInput {
                    sales_order_line_id: line_id,
                    quantity: dec(3),
                }],
            },
        )
        .await
        .unwrap();
    assert_eq!(released.status, "none");
    let draft_remainder = sales
        .shipment_draft_options(fixture.actor, 100)
        .await
        .unwrap();
    assert_eq!(draft_remainder.items.len(), 1);
    assert_eq!(
        draft_remainder.items[0].draft_allocated_quantity.0,
        Decimal::from(3)
    );
    assert_eq!(
        draft_remainder.items[0].shippable_quantity.0,
        Decimal::from(5)
    );
    let duplicate_draft = sales
        .create_shipment(
            fixture.actor,
            Uuid::new_v4(),
            "shipment-create-overallocated-0001",
            &CreateShipment {
                sales_order_id: confirmed.id,
                warehouse_id: fixture.warehouse,
                shipment_date: date,
                lines: vec![ShipmentLineInput {
                    sales_order_line_id: line_id,
                    quantity: dec(6),
                }],
            },
        )
        .await;
    assert!(matches!(duplicate_draft, Err(DomainError::Invalid(_))));

    let shipment_preview = sales
        .shipment_confirmation_preview(fixture.actor, shipment.id)
        .await
        .unwrap();
    assert!(shipment_preview.can_confirm);
    assert_eq!(shipment_preview.readiness, "ready");
    assert_eq!(shipment_preview.sales_amount.0, Decimal::from(300));
    assert_eq!(
        shipment_preview.expected_cost_amount.unwrap().0,
        Decimal::from(15)
    );
    assert_eq!(
        shipment_preview.expected_receivable_amount.0,
        Decimal::from(300)
    );
    assert_eq!(
        shipment_preview.expected_due_date,
        date + chrono::Duration::days(30)
    );
    assert_eq!(shipment_preview.lines.len(), 1);
    assert_eq!(shipment_preview.lines[0].quantity.0, Decimal::from(3));
    assert_eq!(
        shipment_preview.lines[0].average_unit_cost.unwrap().0,
        Decimal::from(5)
    );
    assert_eq!(
        shipment_preview.lines[0].expected_cost_amount.unwrap().0,
        Decimal::from(15)
    );

    sqlx::query("DELETE FROM business_role_permissions WHERE permission_key='shipment:confirm' AND role_id IN (SELECT role_id FROM business_user_roles WHERE enterprise_user_id=$1)")
        .bind(fixture.actor)
        .execute(&pool)
        .await
        .unwrap();
    let shipment_permission_blocked = sales
        .shipment_confirmation_preview(fixture.actor, shipment.id)
        .await
        .unwrap();
    assert!(!shipment_permission_blocked.can_confirm);
    assert_eq!(shipment_permission_blocked.readiness, "permission_required");
    sqlx::query("INSERT INTO business_role_permissions(role_id,permission_key) SELECT role_id,'shipment:confirm' FROM business_user_roles WHERE enterprise_user_id=$1")
        .bind(fixture.actor)
        .execute(&pool)
        .await
        .unwrap();

    sqlx::query("UPDATE inventory_balances SET on_hand_quantity=0,reserved_quantity=0,inventory_value=0,average_unit_cost=NULL WHERE legal_entity_id=$1 AND warehouse_id=$2 AND sku_id=$3")
        .bind(fixture.legal_entity)
        .bind(fixture.warehouse)
        .bind(fixture.sku)
        .execute(&pool)
        .await
        .unwrap();
    let cost_blocked = sales
        .shipment_confirmation_preview(fixture.actor, shipment.id)
        .await
        .unwrap();
    assert!(!cost_blocked.can_confirm);
    assert_eq!(cost_blocked.readiness, "missing_inventory_cost");
    assert!(cost_blocked.expected_cost_amount.is_none());
    assert_eq!(cost_blocked.lines[0].readiness, "missing_inventory_cost");
    sqlx::query("UPDATE inventory_balances SET on_hand_quantity=10,reserved_quantity=8,inventory_value=50,average_unit_cost=5 WHERE legal_entity_id=$1 AND warehouse_id=$2 AND sku_id=$3")
        .bind(fixture.legal_entity)
        .bind(fixture.warehouse)
        .bind(fixture.sku)
        .execute(&pool)
        .await
        .unwrap();

    let shipped = inventory
        .confirm_shipment(
            fixture.actor,
            Uuid::new_v4(),
            shipment.id,
            "shipment-confirm-0001",
            &version(1),
        )
        .await
        .unwrap();
    assert_eq!(shipped.status, "confirmed");
    let confirmed_preview = sales
        .shipment_confirmation_preview(fixture.actor, shipment.id)
        .await
        .unwrap();
    assert!(!confirmed_preview.can_confirm);
    assert_eq!(confirmed_preview.readiness, "shipment_not_draft");
    let post_shipment_options = sales
        .shipment_draft_options(fixture.actor, 100)
        .await
        .unwrap();
    assert_eq!(post_shipment_options.items.len(), 1);
    assert_eq!(
        post_shipment_options.items[0].draft_allocated_quantity.0,
        Decimal::ZERO
    );
    assert_eq!(
        post_shipment_options.items[0].shippable_quantity.0,
        Decimal::from(5)
    );
    assert_eq!(
        sales
            .shipments(fixture.actor, Some(shipment.id), 10)
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        inventory.openings(fixture.actor, 10).await.unwrap().len(),
        1
    );
    assert_eq!(
        inventory
            .movements(fixture.actor, Some(fixture.sku), 10)
            .await
            .unwrap()
            .len(),
        2
    );
    let balance = inventory
        .balances(fixture.actor, Some(fixture.sku), 10)
        .await
        .unwrap()
        .pop()
        .unwrap();
    assert_eq!(balance.on_hand_quantity.0, Decimal::from(7));
    assert_eq!(balance.reserved_quantity.0, Decimal::from(5));
    assert_eq!(balance.inventory_value.0, Decimal::from(35));

    let receivable = settlement
        .receivables(fixture.actor, Some(fixture.customer), 10)
        .await
        .unwrap()
        .pop()
        .unwrap();
    assert_eq!(receivable.original_amount.0, Decimal::from(300));
    assert_eq!(receivable.due_date, date + chrono::Duration::days(30));

    let race_one = receipt(&settlement, &fixture, date, 200, "receipt-race-create-0001").await;
    let race_two = receipt(&settlement, &fixture, date, 200, "receipt-race-create-0002").await;
    let race_one = settlement
        .confirm_receipt(
            fixture.actor,
            Uuid::new_v4(),
            race_one.id,
            "receipt-race-confirm-0001",
            &version(1),
        )
        .await
        .unwrap();
    let race_two = settlement
        .confirm_receipt(
            fixture.actor,
            Uuid::new_v4(),
            race_two.id,
            "receipt-race-confirm-0002",
            &version(1),
        )
        .await
        .unwrap();
    let race_input_one = ApplyReceipt {
        expected_receipt_version: race_one.version,
        allocations: vec![AllocationInput {
            receivable_id: receivable.id,
            amount: dec(200),
        }],
    };
    let race_input_two = ApplyReceipt {
        expected_receipt_version: race_two.version,
        allocations: vec![AllocationInput {
            receivable_id: receivable.id,
            amount: dec(200),
        }],
    };
    let race_apply_one = settlement.apply_receipt(
        fixture.actor,
        Uuid::new_v4(),
        race_one.id,
        "allocation-race-apply-0001",
        &race_input_one,
    );
    let race_apply_two = settlement.apply_receipt(
        fixture.actor,
        Uuid::new_v4(),
        race_two.id,
        "allocation-race-apply-0002",
        &race_input_two,
    );
    let (race_apply_one, race_apply_two) = tokio::join!(race_apply_one, race_apply_two);
    let (winning_receipt, winning_apply, losing_receipt) = match (race_apply_one, race_apply_two) {
        (Ok(result), Err(DomainError::OverAllocation)) => (race_one, result, race_two),
        (Err(DomainError::OverAllocation), Ok(result)) => (race_two, result, race_one),
        outcome => panic!("expected exactly one concurrent allocation: {outcome:?}"),
    };
    let race_allocation: Uuid = sqlx::query_scalar(
        "SELECT id FROM receivable_allocations WHERE receipt_id=$1 AND allocation_type='apply'",
    )
    .bind(winning_receipt.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    let race_receivable_version: i64 =
        sqlx::query_scalar("SELECT version FROM trade_receivables WHERE id=$1")
            .bind(receivable.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    settlement
        .reverse_allocation(
            fixture.actor,
            Uuid::new_v4(),
            race_allocation,
            "allocation-race-reverse-0001",
            &ReverseAllocation {
                expected_receipt_version: winning_apply.version,
                expected_receivable_version: race_receivable_version,
            },
        )
        .await
        .unwrap();
    settlement
        .reverse_receipt(
            fixture.actor,
            Uuid::new_v4(),
            winning_receipt.id,
            "receipt-race-reverse-winning",
            &version(winning_apply.version + 1),
        )
        .await
        .unwrap();
    settlement
        .reverse_receipt(
            fixture.actor,
            Uuid::new_v4(),
            losing_receipt.id,
            "receipt-race-reverse-losing",
            &version(losing_receipt.version),
        )
        .await
        .unwrap();

    let receipt_one = receipt(&settlement, &fixture, date, 100, "receipt-create-0001").await;
    let receipt_one = settlement
        .confirm_receipt(
            fixture.actor,
            Uuid::new_v4(),
            receipt_one.id,
            "receipt-confirm-0001",
            &version(1),
        )
        .await
        .unwrap();
    let receipt_one_applied = settlement
        .apply_receipt(
            fixture.actor,
            Uuid::new_v4(),
            receipt_one.id,
            "allocation-apply-0001",
            &ApplyReceipt {
                expected_receipt_version: receipt_one.version,
                allocations: vec![AllocationInput {
                    receivable_id: receivable.id,
                    amount: dec(100),
                }],
            },
        )
        .await
        .unwrap();
    let receipt_two = receipt(&settlement, &fixture, date, 200, "receipt-create-0002").await;
    let receipt_two = settlement
        .confirm_receipt(
            fixture.actor,
            Uuid::new_v4(),
            receipt_two.id,
            "receipt-confirm-0002",
            &version(1),
        )
        .await
        .unwrap();
    assert_eq!(
        settlement
            .receipts(fixture.actor, Some(fixture.customer), 10)
            .await
            .unwrap()
            .len(),
        4
    );
    let receipt_two_applied = settlement
        .apply_receipt(
            fixture.actor,
            Uuid::new_v4(),
            receipt_two.id,
            "allocation-apply-0002",
            &ApplyReceipt {
                expected_receipt_version: receipt_two.version,
                allocations: vec![AllocationInput {
                    receivable_id: receivable.id,
                    amount: dec(200),
                }],
            },
        )
        .await
        .unwrap();
    let settled = settlement
        .receivables(fixture.actor, Some(fixture.customer), 10)
        .await
        .unwrap()
        .pop()
        .unwrap();
    assert_eq!(settled.status, "settled");
    assert_eq!(settled.open_amount.0, Decimal::ZERO);
    assert!(matches!(
        inventory
            .reverse_shipment(
                fixture.actor,
                Uuid::new_v4(),
                shipment.id,
                "shipment-reverse-blocked-0001",
                &version(2)
            )
            .await,
        Err(DomainError::ReceivableAlreadySettled)
    ));
    let allocation_two: Uuid = sqlx::query_scalar(
        "SELECT id FROM receivable_allocations WHERE receipt_id=$1 AND allocation_type='apply'",
    )
    .bind(receipt_two.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    settlement
        .reverse_allocation(
            fixture.actor,
            Uuid::new_v4(),
            allocation_two,
            "allocation-reverse-0002",
            &ReverseAllocation {
                expected_receipt_version: receipt_two_applied.version,
                expected_receivable_version: settled.version,
            },
        )
        .await
        .unwrap();
    settlement
        .reverse_receipt(
            fixture.actor,
            Uuid::new_v4(),
            receipt_two.id,
            "receipt-reverse-0002",
            &version(receipt_two_applied.version + 1),
        )
        .await
        .unwrap();
    let allocation_one: Uuid = sqlx::query_scalar(
        "SELECT id FROM receivable_allocations WHERE receipt_id=$1 AND allocation_type='apply'",
    )
    .bind(receipt_one.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    let current_receivable_version: i64 =
        sqlx::query_scalar("SELECT version FROM trade_receivables WHERE id=$1")
            .bind(receivable.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    settlement
        .reverse_allocation(
            fixture.actor,
            Uuid::new_v4(),
            allocation_one,
            "allocation-reverse-0001",
            &ReverseAllocation {
                expected_receipt_version: receipt_one_applied.version,
                expected_receivable_version: current_receivable_version,
            },
        )
        .await
        .unwrap();
    settlement
        .reverse_receipt(
            fixture.actor,
            Uuid::new_v4(),
            receipt_one.id,
            "receipt-reverse-0001",
            &version(receipt_one_applied.version + 1),
        )
        .await
        .unwrap();
    let reversed_shipment = inventory
        .reverse_shipment(
            fixture.actor,
            Uuid::new_v4(),
            shipment.id,
            "shipment-reverse-0001",
            &version(2),
        )
        .await
        .unwrap();
    assert_eq!(reversed_shipment.status, "reversed");
    let restored = inventory
        .balances(fixture.actor, Some(fixture.sku), 10)
        .await
        .unwrap()
        .pop()
        .unwrap();
    assert_eq!(restored.on_hand_quantity.0, Decimal::from(10));
    assert_eq!(restored.reserved_quantity.0, Decimal::from(8));
    let current_order_version: i64 =
        sqlx::query_scalar("SELECT version FROM sales_orders WHERE id=$1")
            .bind(confirmed.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    sales
        .cancel_remaining(
            fixture.actor,
            Uuid::new_v4(),
            confirmed.id,
            "order-cancel-after-reversal-0001",
            &version(current_order_version),
        )
        .await
        .unwrap();
    let released_balance = inventory
        .balances(fixture.actor, Some(fixture.sku), 10)
        .await
        .unwrap()
        .pop()
        .unwrap();
    assert_eq!(released_balance.reserved_quantity.0, Decimal::ZERO);
    assert!(inventory.reconcile(fixture.actor).await.unwrap().is_empty());
    assert!(settlement
        .reconcile(fixture.actor)
        .await
        .unwrap()
        .is_empty());
    assert!(sqlx::query("UPDATE inventory_movements SET quantity=999")
        .execute(&pool)
        .await
        .is_err());
    let audit_count: i64 = sqlx::query_scalar("SELECT count(*) FROM business_core_audit_events")
        .fetch_one(&pool)
        .await
        .unwrap();
    let outbox_count: i64 = sqlx::query_scalar("SELECT count(*) FROM business_core_outbox")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(audit_count >= 15);
    assert_eq!(audit_count, outbox_count);

    let mut order_reads = Vec::with_capacity(100);
    let mut inventory_reads = Vec::with_capacity(100);
    let mut receivable_reads = Vec::with_capacity(100);
    for _ in 0..100 {
        let started = std::time::Instant::now();
        sales.list_orders(fixture.actor, 100).await.unwrap();
        order_reads.push(started.elapsed());
        let started = std::time::Instant::now();
        inventory
            .balances(fixture.actor, Some(fixture.sku), 100)
            .await
            .unwrap();
        inventory_reads.push(started.elapsed());
        let started = std::time::Instant::now();
        settlement
            .receivables(fixture.actor, Some(fixture.customer), 100)
            .await
            .unwrap();
        receivable_reads.push(started.elapsed());
    }
    eprintln!(
        "B2 local PostgreSQL direct-service reads (100 samples each): orders p50={:?} p95={:?}; inventory p50={:?} p95={:?}; receivables p50={:?} p95={:?}",
        percentile(&mut order_reads, 50),
        percentile(&mut order_reads, 95),
        percentile(&mut inventory_reads, 50),
        percentile(&mut inventory_reads, 95),
        percentile(&mut receivable_reads, 50),
        percentile(&mut receivable_reads, 95),
    );
}

fn percentile(samples: &mut [std::time::Duration], percentile: usize) -> std::time::Duration {
    samples.sort_unstable();
    samples[((samples.len() - 1) * percentile) / 100]
}

async fn create_order(
    sales: &SalesService,
    fixture: &Fixture,
    date: NaiveDate,
    key: &str,
) -> business_core::b2::model::CommandResult {
    sales
        .create_order(
            fixture.actor,
            Uuid::new_v4(),
            key,
            &CreateSalesOrder {
                legal_entity_id: fixture.legal_entity,
                customer_id: fixture.customer,
                salesperson_user_id: None,
                business_unit_id: fixture.business_unit,
                department_id: None,
                brand_id: Some(fixture.brand),
                currency: "CNY".into(),
                order_date: date,
                requested_delivery_date: Some(date),
                payment_terms_days: None,
                customer_reference: None,
                business_note: None,
                lines: vec![SalesOrderLineInput {
                    sku_id: fixture.sku,
                    warehouse_id: fixture.warehouse,
                    unit_of_measure_id: fixture.uom,
                    quantity: dec(8),
                    unit_price: dec(100),
                    discount_amount: dec(0),
                    tax_rate: dec(0),
                    business_unit_id: None,
                    department_id: None,
                    brand_id: Some(fixture.brand),
                }],
            },
        )
        .await
        .unwrap()
}

async fn receipt(
    service: &SettlementService,
    fixture: &Fixture,
    date: NaiveDate,
    amount: i64,
    key: &str,
) -> business_core::b2::model::CommandResult {
    service
        .create_receipt(
            fixture.actor,
            Uuid::new_v4(),
            key,
            &CreateCustomerReceipt {
                legal_entity_id: fixture.legal_entity,
                customer_id: fixture.customer,
                currency: "CNY".into(),
                receipt_date: date,
                amount: dec(amount),
                payment_method: "bank_transfer".into(),
                external_reference: Some(format!("BANK-{amount}")),
                business_note: None,
            },
        )
        .await
        .unwrap()
}

fn dec(value: i64) -> DecimalString {
    DecimalString(Decimal::from(value))
}

fn version(expected_version: i64) -> VersionCommand {
    VersionCommand {
        expected_version,
        reason_code: None,
    }
}

async fn seed(pool: &sqlx::PgPool) -> Fixture {
    let fixture = Fixture {
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
    sqlx::query("INSERT INTO enterprise_users(id,oidc_issuer,oidc_subject,display_name) VALUES($1,'https://issuer.test',$2,'B2 Operator')").bind(fixture.actor).bind(fixture.actor.to_string()).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO business_group_profile(id,code,name,base_currency,timezone) VALUES($1,'B2_GROUP','B2 Test Group','CNY','Asia/Shanghai')").bind(Uuid::new_v4()).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO business_legal_entities(id,code,name,country_code,functional_currency) VALUES($1,'LE_B2','B2 Legal','CN','CNY')").bind(fixture.legal_entity).execute(pool).await.unwrap();
    sqlx::query(
        "INSERT INTO business_units(id,legal_entity_id,code,name) VALUES($1,$2,'BU_B2','B2 Trade')",
    )
    .bind(fixture.business_unit)
    .bind(fixture.legal_entity)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO business_warehouses(id,legal_entity_id,business_unit_id,code,name) VALUES($1,$2,$3,'WH_B2','B2 Warehouse')").bind(fixture.warehouse).bind(fixture.legal_entity).bind(fixture.business_unit).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO business_customers(id,legal_entity_id,business_unit_id,code,name,credit_currency,payment_terms_days) VALUES($1,$2,$3,'CUS_B2','B2 Customer','CNY',30)").bind(fixture.customer).bind(fixture.legal_entity).bind(fixture.business_unit).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO business_units_of_measure(id,code,name,precision_scale) VALUES($1,'UOM_B2','Each',0)").bind(fixture.uom).execute(pool).await.unwrap();
    sqlx::query(
        "INSERT INTO business_product_categories(id,code,name) VALUES($1,'CAT_B2','B2 Category')",
    )
    .bind(category)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO business_brands(id,code,name) VALUES($1,'BRAND_B2','B2 Brand')")
        .bind(fixture.brand)
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO business_products(id,code,name,category_id,brand_id,base_uom_id) VALUES($1,'PROD_B2','B2 Product',$2,$3,$4)").bind(product).bind(category).bind(fixture.brand).bind(fixture.uom).execute(pool).await.unwrap();
    sqlx::query(
        "INSERT INTO business_skus(id,product_id,code,name) VALUES($1,$2,'SKU_B2','B2 SKU')",
    )
    .bind(fixture.sku)
    .bind(product)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO business_roles(id,role_key,name) VALUES($1,'b2_operator','B2 Operator')",
    )
    .bind(role)
    .execute(pool)
    .await
    .unwrap();
    let permissions = [
        "sales_order:read",
        "sales_order:create",
        "sales_order:update_draft",
        "sales_order:confirm",
        "sales_order:cancel",
        "sales_order:place_hold",
        "sales_order:release_hold",
        "shipment:create",
        "shipment:confirm",
        "shipment:reverse",
        "inventory:read",
        "inventory_opening:create",
        "inventory_opening:post",
        "inventory_opening:reverse",
        "receivable:read",
        "customer_receipt:read",
        "customer_receipt:create",
        "customer_receipt:confirm",
        "customer_receipt:reverse",
        "receivable_allocation:create",
        "receivable_allocation:reverse",
    ];
    for permission in permissions {
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
    .bind(fixture.actor)
    .bind(role)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO business_legal_entity_scopes(enterprise_user_id,legal_entity_id,granted_by) VALUES($1,$2,$1)").bind(fixture.actor).bind(fixture.legal_entity).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO business_warehouse_scopes(enterprise_user_id,warehouse_id,granted_by) VALUES($1,$2,$1)").bind(fixture.actor).bind(fixture.warehouse).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO business_customer_scopes(enterprise_user_id,customer_id,granted_by) VALUES($1,$2,$1)").bind(fixture.actor).bind(fixture.customer).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO business_brand_scopes(enterprise_user_id,brand_id,granted_by) VALUES($1,$2,$1)").bind(fixture.actor).bind(fixture.brand).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO business_unit_scopes(enterprise_user_id,business_unit_id,granted_by) VALUES($1,$2,$1)").bind(fixture.actor).bind(fixture.business_unit).execute(pool).await.unwrap();
    fixture
}
