use business_core::{
    b2::{
        model::{
            CreateInventoryOpening, DecimalString, InventoryOpeningLineInput,
            VersionCommand as B2VersionCommand,
        },
        DomainError, InventoryService,
    },
    b3::{
        model::{
            ApplySupplierPayment, CreateGoodsReceipt, CreatePurchaseOrder, CreateSupplierPayment,
            GoodsReceiptLineInput, PayableAllocationInput, PurchaseOrderLineInput,
            ReversePayableAllocation, VersionCommand,
        },
        PayablesService, PurchasingService, ReceivingService,
    },
    PgStore,
};
use chrono::NaiveDate;
use rust_decimal::Decimal;
use sqlx::{postgres::PgPoolOptions, Row};
use std::str::FromStr;
use uuid::Uuid;

#[path = "postgres_b3/concurrency.rs"]
mod concurrency;

struct Fixture {
    actor: Uuid,
    legal_entity: Uuid,
    business_unit: Uuid,
    warehouse: Uuid,
    supplier: Uuid,
    uom: Uuid,
    sku: Uuid,
}

#[tokio::test]
async fn b3_postgres_purchase_cost_payable_and_concurrency() {
    let Ok(database_url) = std::env::var("BUSINESS_CORE_B3_TEST_DATABASE_URL") else {
        eprintln!("skipping: BUSINESS_CORE_B3_TEST_DATABASE_URL is not set");
        return;
    };
    let pool = PgPoolOptions::new()
        .max_connections(16)
        .connect(&database_url)
        .await
        .unwrap();
    let store = PgStore::new(pool.clone());
    store.migrate().await.unwrap();
    let fixture = seed(&pool).await;
    let date = NaiveDate::from_ymd_opt(2026, 8, 21).unwrap();
    let purchasing = PurchasingService::new(store.clone(), "PO".into(), 30);
    let receiving =
        ReceivingService::new(store.clone(), purchasing.clone(), "GR".into(), "AP".into());
    let payables = PayablesService::new(store.clone(), "PAY".into());
    let inventory = InventoryService::new(store, "OPEN".into(), "AR".into());

    let opening = inventory
        .create_opening(
            fixture.actor,
            Uuid::new_v4(),
            "b3-opening-create-0001",
            &CreateInventoryOpening {
                legal_entity_id: fixture.legal_entity,
                business_date: date,
                currency: "CNY".into(),
                lines: vec![InventoryOpeningLineInput {
                    warehouse_id: fixture.warehouse,
                    sku_id: fixture.sku,
                    quantity: dec("10"),
                    unit_cost: dec("100"),
                }],
            },
        )
        .await
        .unwrap();
    inventory
        .post_opening(
            fixture.actor,
            Uuid::new_v4(),
            opening.id,
            "b3-opening-post-0001",
            &b2_version(1),
        )
        .await
        .unwrap();

    let entry_options = purchasing.entry_options(fixture.actor, None).await.unwrap();
    assert!(entry_options.can_create);
    assert!(!entry_options.can_update);
    assert!(entry_options.draft.is_none());
    sqlx::query("DELETE FROM business_role_permissions WHERE permission_key='purchase_order:create' AND role_id IN (SELECT role_id FROM business_user_roles WHERE enterprise_user_id=$1)")
        .bind(fixture.actor)
        .execute(&pool)
        .await
        .unwrap();
    assert!(
        !purchasing
            .entry_options(fixture.actor, None)
            .await
            .unwrap()
            .can_create
    );
    sqlx::query("INSERT INTO business_role_permissions(role_id,permission_key) SELECT role_id,'purchase_order:create' FROM business_user_roles WHERE enterprise_user_id=$1")
        .bind(fixture.actor)
        .execute(&pool)
        .await
        .unwrap();

    let order = create_order(
        &purchasing,
        &fixture,
        date,
        "b3-order-create-0001",
        "10",
        "120",
    )
    .await;
    let draft_options = purchasing
        .entry_options(fixture.actor, Some(order.id))
        .await
        .unwrap();
    assert!(draft_options.can_update);
    let draft = draft_options.draft.unwrap();
    assert_eq!(draft.id, order.id);
    assert_eq!(draft.supplier_id, fixture.supplier);
    assert_eq!(draft.business_unit_id, fixture.business_unit);
    assert_eq!(draft.payment_terms_days, 30);
    assert_eq!(draft.lines.len(), 1);
    assert_eq!(draft.lines[0].quantity.0, decimal("10"));
    assert_eq!(draft.lines[0].unit_price.0, decimal("120"));
    sqlx::query("DELETE FROM business_role_permissions WHERE permission_key='purchase_order:update_draft' AND role_id IN (SELECT role_id FROM business_user_roles WHERE enterprise_user_id=$1)")
        .bind(fixture.actor)
        .execute(&pool)
        .await
        .unwrap();
    assert!(
        !purchasing
            .entry_options(fixture.actor, Some(order.id))
            .await
            .unwrap()
            .can_update
    );
    sqlx::query("INSERT INTO business_role_permissions(role_id,permission_key) SELECT role_id,'purchase_order:update_draft' FROM business_user_roles WHERE enterprise_user_id=$1")
        .bind(fixture.actor)
        .execute(&pool)
        .await
        .unwrap();
    let order_preview = purchasing
        .confirmation_preview(fixture.actor, order.id)
        .await
        .unwrap();
    assert!(order_preview.can_confirm);
    assert_eq!(order_preview.readiness, "ready");
    assert_eq!(order_preview.order_date, date);
    assert_eq!(order_preview.expected_delivery_date, Some(date));
    assert_eq!(order_preview.subtotal_amount.0, decimal("1200"));
    assert_eq!(order_preview.net_amount.0, decimal("1200"));
    assert_eq!(order_preview.gross_amount.0, decimal("1200"));
    assert_eq!(order_preview.warehouse_count, 1);
    assert_eq!(order_preview.lines.len(), 1);
    assert!(order_preview.lines[0].ready);
    assert_eq!(order_preview.lines[0].ordered_quantity.0, decimal("10"));
    assert_eq!(order_preview.lines[0].unit_price.0, decimal("120"));
    sqlx::query("DELETE FROM business_role_permissions WHERE permission_key='purchase_order:confirm' AND role_id IN (SELECT role_id FROM business_user_roles WHERE enterprise_user_id=$1)")
        .bind(fixture.actor)
        .execute(&pool)
        .await
        .unwrap();
    let order_confirm_blocked = purchasing
        .confirmation_preview(fixture.actor, order.id)
        .await
        .unwrap();
    assert!(!order_confirm_blocked.can_confirm);
    assert_eq!(order_confirm_blocked.readiness, "permission_required");
    sqlx::query("INSERT INTO business_role_permissions(role_id,permission_key) SELECT role_id,'purchase_order:confirm' FROM business_user_roles WHERE enterprise_user_id=$1")
        .bind(fixture.actor)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE business_suppliers SET status='disabled' WHERE id=$1")
        .bind(fixture.supplier)
        .execute(&pool)
        .await
        .unwrap();
    let inactive_supplier_preview = purchasing
        .confirmation_preview(fixture.actor, order.id)
        .await
        .unwrap();
    assert!(!inactive_supplier_preview.can_confirm);
    assert_eq!(inactive_supplier_preview.readiness, "supplier_inactive");
    let inactive_supplier_confirm = purchasing
        .confirm_order(
            fixture.actor,
            Uuid::new_v4(),
            order.id,
            "b3-order-confirm-inactive-supplier-0001",
            &version(1),
        )
        .await;
    assert!(matches!(
        inactive_supplier_confirm,
        Err(DomainError::Invalid(_))
    ));
    sqlx::query("UPDATE business_suppliers SET status='active' WHERE id=$1")
        .bind(fixture.supplier)
        .execute(&pool)
        .await
        .unwrap();
    let confirmed_order = purchasing
        .confirm_order(
            fixture.actor,
            Uuid::new_v4(),
            order.id,
            "b3-order-confirm-0001",
            &version(1),
        )
        .await
        .unwrap();
    let replay = purchasing
        .confirm_order(
            fixture.actor,
            Uuid::new_v4(),
            order.id,
            "b3-order-confirm-0001",
            &version(1),
        )
        .await
        .unwrap();
    assert!(replay.idempotent_replay);
    assert_eq!(replay.id, confirmed_order.id);
    let confirmed_order_preview = purchasing
        .confirmation_preview(fixture.actor, order.id)
        .await
        .unwrap();
    assert!(!confirmed_order_preview.can_confirm);
    assert_eq!(confirmed_order_preview.readiness, "order_not_draft");
    let no_stock: Decimal = sqlx::query_scalar(
        "SELECT on_hand_quantity FROM inventory_balances WHERE legal_entity_id=$1 AND warehouse_id=$2 AND sku_id=$3",
    )
    .bind(fixture.legal_entity)
    .bind(fixture.warehouse)
    .bind(fixture.sku)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(no_stock, decimal("10"));
    let po_line: Uuid =
        sqlx::query_scalar("SELECT id FROM purchase_order_lines WHERE purchase_order_id=$1")
            .bind(order.id)
            .fetch_one(&pool)
            .await
            .unwrap();

    let receipt_options = receiving.draft_options(fixture.actor, 100).await.unwrap();
    assert!(receipt_options.can_create);
    assert_eq!(receipt_options.items.len(), 1);
    assert_eq!(receipt_options.items[0].purchase_order_line_id, po_line);
    assert_eq!(
        receipt_options.items[0].receivable_quantity.0,
        decimal("10")
    );
    assert_eq!(
        receipt_options.items[0].draft_allocated_quantity.0,
        Decimal::ZERO
    );
    sqlx::query("DELETE FROM business_role_permissions WHERE permission_key='goods_receipt:create' AND role_id IN (SELECT role_id FROM business_user_roles WHERE enterprise_user_id=$1)")
        .bind(fixture.actor)
        .execute(&pool)
        .await
        .unwrap();
    let receipt_create_blocked = receiving.draft_options(fixture.actor, 100).await.unwrap();
    assert!(!receipt_create_blocked.can_create);
    assert_eq!(receipt_create_blocked.items.len(), 1);
    sqlx::query("INSERT INTO business_role_permissions(role_id,permission_key) SELECT role_id,'goods_receipt:create' FROM business_user_roles WHERE enterprise_user_id=$1")
        .bind(fixture.actor)
        .execute(&pool)
        .await
        .unwrap();

    let first = create_receipt(
        &receiving,
        &fixture,
        date,
        order.id,
        po_line,
        "6",
        "b3-receipt-create-0001",
    )
    .await;
    let receipt_remainder = receiving.draft_options(fixture.actor, 100).await.unwrap();
    assert_eq!(receipt_remainder.items.len(), 1);
    assert_eq!(
        receipt_remainder.items[0].draft_allocated_quantity.0,
        decimal("6")
    );
    assert_eq!(
        receipt_remainder.items[0].receivable_quantity.0,
        decimal("4")
    );
    let duplicate_draft = receiving
        .create_receipt(
            fixture.actor,
            Uuid::new_v4(),
            "b3-receipt-overallocated-draft-0001",
            &CreateGoodsReceipt {
                purchase_order_id: order.id,
                warehouse_id: fixture.warehouse,
                receipt_date: date,
                lines: vec![GoodsReceiptLineInput {
                    purchase_order_line_id: po_line,
                    quantity: dec("5"),
                }],
            },
        )
        .await;
    assert!(matches!(duplicate_draft, Err(DomainError::OverReceipt)));

    let receipt_preview = receiving
        .confirmation_preview(fixture.actor, first.id)
        .await
        .unwrap();
    assert!(receipt_preview.can_confirm);
    assert_eq!(receipt_preview.readiness, "ready");
    assert_eq!(receipt_preview.expected_inventory_cost.0, decimal("720"));
    assert_eq!(receipt_preview.expected_tax_amount.0, Decimal::ZERO);
    assert_eq!(receipt_preview.expected_payable_amount.0, decimal("720"));
    assert_eq!(
        receipt_preview.expected_due_date,
        date + chrono::Duration::days(30)
    );
    assert_eq!(receipt_preview.lines.len(), 1);
    assert_eq!(
        receipt_preview.lines[0].current_on_hand_quantity.0,
        decimal("10")
    );
    assert_eq!(
        receipt_preview.lines[0].projected_on_hand_quantity.0,
        decimal("16")
    );
    assert_eq!(
        receipt_preview.lines[0].projected_inventory_value.0,
        decimal("1720")
    );
    assert_eq!(
        receipt_preview.lines[0].projected_average_unit_cost.0,
        decimal("107.5")
    );
    sqlx::query("DELETE FROM business_role_permissions WHERE permission_key='goods_receipt:confirm' AND role_id IN (SELECT role_id FROM business_user_roles WHERE enterprise_user_id=$1)")
        .bind(fixture.actor)
        .execute(&pool)
        .await
        .unwrap();
    let receipt_confirm_blocked = receiving
        .confirmation_preview(fixture.actor, first.id)
        .await
        .unwrap();
    assert!(!receipt_confirm_blocked.can_confirm);
    assert_eq!(receipt_confirm_blocked.readiness, "permission_required");
    sqlx::query("INSERT INTO business_role_permissions(role_id,permission_key) SELECT role_id,'goods_receipt:confirm' FROM business_user_roles WHERE enterprise_user_id=$1")
        .bind(fixture.actor)
        .execute(&pool)
        .await
        .unwrap();
    receiving
        .confirm_receipt(
            fixture.actor,
            Uuid::new_v4(),
            first.id,
            "b3-receipt-confirm-0001",
            &version(1),
        )
        .await
        .unwrap();
    let confirmed_receipt_preview = receiving
        .confirmation_preview(fixture.actor, first.id)
        .await
        .unwrap();
    assert!(!confirmed_receipt_preview.can_confirm);
    assert_eq!(confirmed_receipt_preview.readiness, "receipt_not_draft");
    assert_balance(&pool, &fixture, "16", "1720", "107.5").await;

    let second = create_receipt(
        &receiving,
        &fixture,
        date,
        order.id,
        po_line,
        "4",
        "b3-receipt-create-0002",
    )
    .await;
    receiving
        .confirm_receipt(
            fixture.actor,
            Uuid::new_v4(),
            second.id,
            "b3-receipt-confirm-0002",
            &version(1),
        )
        .await
        .unwrap();
    assert_balance(&pool, &fixture, "20", "2200", "110").await;
    let order_status =
        sqlx::query("SELECT lifecycle_status,receiving_status FROM purchase_orders WHERE id=$1")
            .bind(order.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        order_status.get::<String, _>("lifecycle_status"),
        "completed"
    );
    assert_eq!(
        order_status.get::<String, _>("receiving_status"),
        "received"
    );

    let payable_rows = sqlx::query(
        "SELECT id,original_amount,status FROM trade_payables WHERE purchase_order_id=$1 ORDER BY recognized_at,id",
    )
    .bind(order.id)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(payable_rows.len(), 2);
    assert_eq!(
        payable_rows[0].get::<Decimal, _>("original_amount"),
        decimal("720")
    );
    assert_eq!(
        payable_rows[1].get::<Decimal, _>("original_amount"),
        decimal("480")
    );

    let payment = payables
        .create_payment(
            fixture.actor,
            Uuid::new_v4(),
            "b3-payment-create-0001",
            &CreateSupplierPayment {
                legal_entity_id: fixture.legal_entity,
                supplier_id: fixture.supplier,
                currency: "CNY".into(),
                payment_date: date,
                amount: dec("1000"),
                payment_method: "bank_transfer".into(),
                external_reference: Some("MANUAL-001".into()),
                business_note: None,
            },
        )
        .await
        .unwrap();
    payables
        .confirm_payment(
            fixture.actor,
            Uuid::new_v4(),
            payment.id,
            "b3-payment-confirm-0001",
            &version(1),
        )
        .await
        .unwrap();
    let applied = payables
        .apply_payment(
            fixture.actor,
            Uuid::new_v4(),
            payment.id,
            "b3-payment-apply-0001",
            &ApplySupplierPayment {
                expected_payment_version: 2,
                allocations: vec![
                    PayableAllocationInput {
                        payable_id: payable_rows[0].get("id"),
                        amount: dec("720"),
                    },
                    PayableAllocationInput {
                        payable_id: payable_rows[1].get("id"),
                        amount: dec("280"),
                    },
                ],
            },
        )
        .await
        .unwrap();
    assert_eq!(applied.status, "fully_allocated");
    assert!(payables
        .reverse_payment(
            fixture.actor,
            Uuid::new_v4(),
            payment.id,
            "b3-payment-reverse-blocked-0001",
            &version(3),
        )
        .await
        .is_err());

    let allocation = sqlx::query(
        "SELECT id,payable_id FROM payable_allocations WHERE supplier_payment_id=$1 AND amount=280",
    )
    .bind(payment.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    let payable_version: i64 = sqlx::query_scalar("SELECT version FROM trade_payables WHERE id=$1")
        .bind(allocation.get::<Uuid, _>("payable_id"))
        .fetch_one(&pool)
        .await
        .unwrap();
    payables
        .reverse_allocation(
            fixture.actor,
            Uuid::new_v4(),
            allocation.get("id"),
            "b3-allocation-reverse-0001",
            &ReversePayableAllocation {
                expected_payment_version: 3,
                expected_payable_version: payable_version,
            },
        )
        .await
        .unwrap();

    let second_payable_id: Uuid = payable_rows[1].get("id");
    let payment_two = payables
        .create_payment(
            fixture.actor,
            Uuid::new_v4(),
            "b3-payment-create-0002",
            &CreateSupplierPayment {
                legal_entity_id: fixture.legal_entity,
                supplier_id: fixture.supplier,
                currency: "CNY".into(),
                payment_date: date,
                amount: dec("480"),
                payment_method: "bank_transfer".into(),
                external_reference: Some("MANUAL-002".into()),
                business_note: None,
            },
        )
        .await
        .unwrap();
    payables
        .confirm_payment(
            fixture.actor,
            Uuid::new_v4(),
            payment_two.id,
            "b3-payment-confirm-0002",
            &version(1),
        )
        .await
        .unwrap();
    payables
        .apply_payment(
            fixture.actor,
            Uuid::new_v4(),
            payment_two.id,
            "b3-payment-apply-0002",
            &ApplySupplierPayment {
                expected_payment_version: 2,
                allocations: vec![PayableAllocationInput {
                    payable_id: second_payable_id,
                    amount: dec("480"),
                }],
            },
        )
        .await
        .unwrap();
    let unsettled: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM trade_payables WHERE purchase_order_id=$1 AND status<>'settled'",
    )
    .bind(order.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(unsettled, 0);

    let blocked = receiving
        .reverse_receipt(
            fixture.actor,
            Uuid::new_v4(),
            first.id,
            "b3-receipt-reverse-blocked-0001",
            &version(2),
        )
        .await;
    assert!(matches!(
        blocked,
        Err(business_core::b2::DomainError::SubsequentInventoryMovementsExist)
            | Err(business_core::b2::DomainError::PayableAlreadySettled)
    ));

    concurrent_over_receipt(&purchasing, &receiving, &fixture, date, &pool).await;
    concurrent_inventory_receipts(&purchasing, &receiving, &fixture, date, &pool).await;
    concurrency::concurrent_over_allocation(
        &purchasing,
        &receiving,
        &payables,
        &fixture,
        date,
        &pool,
    )
    .await;
    reversible_receipt(&purchasing, &receiving, &fixture, date, &pool).await;
    let reconciliation = payables.reconcile(fixture.actor).await.unwrap();
    assert_eq!(reconciliation["consistent"], true);
    let movement_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM inventory_movements WHERE movement_type='purchase_receipt'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let audit_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM business_core_audit_events WHERE operation LIKE 'PURCHASE_%' OR operation LIKE 'GOODS_%' OR operation LIKE 'TRADE_PAYABLE_%' OR operation LIKE 'SUPPLIER_PAYMENT_%' OR operation LIKE 'PAYABLE_%'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let outbox_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM business_core_outbox WHERE topic LIKE 'purchase_%' OR topic LIKE 'goods_%' OR topic LIKE 'trade_payable_%' OR topic LIKE 'supplier_payment_%' OR topic LIKE 'payable_%'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(movement_count >= 2 && audit_count > 0 && outbox_count > 0);
    let append_only = sqlx::query("UPDATE purchase_order_events SET event_type='tampered'")
        .execute(&pool)
        .await;
    assert!(append_only.is_err());
}

async fn concurrent_inventory_receipts(
    purchasing: &PurchasingService,
    receiving: &ReceivingService,
    f: &Fixture,
    date: NaiveDate,
    pool: &sqlx::PgPool,
) {
    let before = sqlx::query(
        "SELECT on_hand_quantity,inventory_value FROM inventory_balances WHERE legal_entity_id=$1 AND warehouse_id=$2 AND sku_id=$3",
    )
    .bind(f.legal_entity)
    .bind(f.warehouse)
    .bind(f.sku)
    .fetch_one(pool)
    .await
    .unwrap();
    let a = create_order(
        purchasing,
        f,
        date,
        "b3-cost-race-order-create-0001",
        "10",
        "100",
    )
    .await;
    let b = create_order(
        purchasing,
        f,
        date,
        "b3-cost-race-order-create-0002",
        "20",
        "120",
    )
    .await;
    purchasing
        .confirm_order(
            f.actor,
            Uuid::new_v4(),
            a.id,
            "b3-cost-race-order-confirm-0001",
            &version(1),
        )
        .await
        .unwrap();
    purchasing
        .confirm_order(
            f.actor,
            Uuid::new_v4(),
            b.id,
            "b3-cost-race-order-confirm-0002",
            &version(1),
        )
        .await
        .unwrap();
    let line_a: Uuid =
        sqlx::query_scalar("SELECT id FROM purchase_order_lines WHERE purchase_order_id=$1")
            .bind(a.id)
            .fetch_one(pool)
            .await
            .unwrap();
    let line_b: Uuid =
        sqlx::query_scalar("SELECT id FROM purchase_order_lines WHERE purchase_order_id=$1")
            .bind(b.id)
            .fetch_one(pool)
            .await
            .unwrap();
    let receipt_a = create_receipt(
        receiving,
        f,
        date,
        a.id,
        line_a,
        "10",
        "b3-cost-race-receipt-create-0001",
    )
    .await;
    let receipt_b = create_receipt(
        receiving,
        f,
        date,
        b.id,
        line_b,
        "20",
        "b3-cost-race-receipt-create-0002",
    )
    .await;
    let left_version = version(1);
    let right_version = version(1);
    let left = receiving.confirm_receipt(
        f.actor,
        Uuid::new_v4(),
        receipt_a.id,
        "b3-cost-race-receipt-confirm-0001",
        &left_version,
    );
    let right = receiving.confirm_receipt(
        f.actor,
        Uuid::new_v4(),
        receipt_b.id,
        "b3-cost-race-receipt-confirm-0002",
        &right_version,
    );
    let (left, right) = tokio::join!(left, right);
    assert!(left.is_ok() && right.is_ok());
    let after = sqlx::query(
        "SELECT on_hand_quantity,inventory_value,average_unit_cost FROM inventory_balances WHERE legal_entity_id=$1 AND warehouse_id=$2 AND sku_id=$3",
    )
    .bind(f.legal_entity)
    .bind(f.warehouse)
    .bind(f.sku)
    .fetch_one(pool)
    .await
    .unwrap();
    let expected_quantity = before.get::<Decimal, _>("on_hand_quantity") + decimal("30");
    let expected_value = before.get::<Decimal, _>("inventory_value") + decimal("3400");
    assert_eq!(
        after.get::<Decimal, _>("on_hand_quantity"),
        expected_quantity
    );
    assert_eq!(after.get::<Decimal, _>("inventory_value"), expected_value);
    assert_eq!(
        after.get::<Decimal, _>("average_unit_cost"),
        (expected_value / expected_quantity).round_dp(6)
    );
}

async fn reversible_receipt(
    purchasing: &PurchasingService,
    receiving: &ReceivingService,
    f: &Fixture,
    date: NaiveDate,
    pool: &sqlx::PgPool,
) {
    let before: Decimal = sqlx::query_scalar(
        "SELECT on_hand_quantity FROM inventory_balances WHERE legal_entity_id=$1 AND warehouse_id=$2 AND sku_id=$3",
    )
    .bind(f.legal_entity)
    .bind(f.warehouse)
    .bind(f.sku)
    .fetch_one(pool)
    .await
    .unwrap();
    let order = create_order(
        purchasing,
        f,
        date,
        "b3-reversible-order-create-0001",
        "1",
        "130",
    )
    .await;
    purchasing
        .confirm_order(
            f.actor,
            Uuid::new_v4(),
            order.id,
            "b3-reversible-order-confirm-0001",
            &version(1),
        )
        .await
        .unwrap();
    let line: Uuid =
        sqlx::query_scalar("SELECT id FROM purchase_order_lines WHERE purchase_order_id=$1")
            .bind(order.id)
            .fetch_one(pool)
            .await
            .unwrap();
    let receipt = create_receipt(
        receiving,
        f,
        date,
        order.id,
        line,
        "1",
        "b3-reversible-receipt-create-0001",
    )
    .await;
    receiving
        .confirm_receipt(
            f.actor,
            Uuid::new_v4(),
            receipt.id,
            "b3-reversible-receipt-confirm-0001",
            &version(1),
        )
        .await
        .unwrap();
    receiving
        .reverse_receipt(
            f.actor,
            Uuid::new_v4(),
            receipt.id,
            "b3-reversible-receipt-reverse-0001",
            &version(2),
        )
        .await
        .unwrap();
    let after: Decimal = sqlx::query_scalar(
        "SELECT on_hand_quantity FROM inventory_balances WHERE legal_entity_id=$1 AND warehouse_id=$2 AND sku_id=$3",
    )
    .bind(f.legal_entity)
    .bind(f.warehouse)
    .bind(f.sku)
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(before, after);
}

async fn concurrent_over_receipt(
    purchasing: &PurchasingService,
    receiving: &ReceivingService,
    f: &Fixture,
    date: NaiveDate,
    pool: &sqlx::PgPool,
) {
    let order = create_order(
        purchasing,
        f,
        date,
        "b3-race-order-create-0001",
        "10",
        "120",
    )
    .await;
    purchasing
        .confirm_order(
            f.actor,
            Uuid::new_v4(),
            order.id,
            "b3-race-order-confirm-0001",
            &version(1),
        )
        .await
        .unwrap();
    let line: Uuid =
        sqlx::query_scalar("SELECT id FROM purchase_order_lines WHERE purchase_order_id=$1")
            .bind(order.id)
            .fetch_one(pool)
            .await
            .unwrap();
    let left_input = CreateGoodsReceipt {
        purchase_order_id: order.id,
        warehouse_id: f.warehouse,
        receipt_date: date,
        lines: vec![GoodsReceiptLineInput {
            purchase_order_line_id: line,
            quantity: dec("8"),
        }],
    };
    let right_input = left_input.clone();
    let left = receiving.create_receipt(
        f.actor,
        Uuid::new_v4(),
        "b3-race-receipt-create-0001",
        &left_input,
    );
    let right = receiving.create_receipt(
        f.actor,
        Uuid::new_v4(),
        "b3-race-receipt-create-0002",
        &right_input,
    );
    let (left, right) = tokio::join!(left, right);
    let receipt = match (left, right) {
        (Ok(receipt), Err(DomainError::OverReceipt))
        | (Err(DomainError::OverReceipt), Ok(receipt)) => receipt,
        outcome => panic!("expected one draft allocation to win: {outcome:?}"),
    };
    receiving
        .confirm_receipt(
            f.actor,
            Uuid::new_v4(),
            receipt.id,
            "b3-race-receipt-confirm-0001",
            &version(1),
        )
        .await
        .unwrap();
    let received: Decimal =
        sqlx::query_scalar("SELECT received_quantity FROM purchase_order_lines WHERE id=$1")
            .bind(line)
            .fetch_one(pool)
            .await
            .unwrap();
    assert_eq!(received, decimal("8"));
}

async fn create_order(
    service: &PurchasingService,
    f: &Fixture,
    date: NaiveDate,
    key: &str,
    quantity: &str,
    unit_price: &str,
) -> business_core::b3::model::CommandResult {
    service
        .create_order(
            f.actor,
            Uuid::new_v4(),
            key,
            &CreatePurchaseOrder {
                legal_entity_id: f.legal_entity,
                supplier_id: f.supplier,
                buyer_user_id: Some(f.actor),
                business_unit_id: f.business_unit,
                department_id: None,
                brand_id: None,
                currency: "CNY".into(),
                order_date: date,
                expected_delivery_date: Some(date),
                payment_terms_days: Some(30),
                supplier_reference: None,
                business_note: None,
                lines: vec![PurchaseOrderLineInput {
                    sku_id: f.sku,
                    warehouse_id: f.warehouse,
                    unit_of_measure_id: f.uom,
                    quantity: dec(quantity),
                    unit_price: dec(unit_price),
                    discount_amount: dec("0"),
                    tax_rate: dec("0"),
                    business_unit_id: None,
                    department_id: None,
                    brand_id: None,
                }],
            },
        )
        .await
        .unwrap()
}

async fn create_receipt(
    service: &ReceivingService,
    f: &Fixture,
    date: NaiveDate,
    order_id: Uuid,
    line_id: Uuid,
    quantity: &str,
    key: &str,
) -> business_core::b3::model::CommandResult {
    service
        .create_receipt(
            f.actor,
            Uuid::new_v4(),
            key,
            &CreateGoodsReceipt {
                purchase_order_id: order_id,
                warehouse_id: f.warehouse,
                receipt_date: date,
                lines: vec![GoodsReceiptLineInput {
                    purchase_order_line_id: line_id,
                    quantity: dec(quantity),
                }],
            },
        )
        .await
        .unwrap()
}

async fn assert_balance(
    pool: &sqlx::PgPool,
    f: &Fixture,
    quantity: &str,
    value: &str,
    average: &str,
) {
    let row = sqlx::query("SELECT on_hand_quantity,inventory_value,average_unit_cost FROM inventory_balances WHERE legal_entity_id=$1 AND warehouse_id=$2 AND sku_id=$3")
        .bind(f.legal_entity).bind(f.warehouse).bind(f.sku).fetch_one(pool).await.unwrap();
    assert_eq!(row.get::<Decimal, _>("on_hand_quantity"), decimal(quantity));
    assert_eq!(row.get::<Decimal, _>("inventory_value"), decimal(value));
    assert_eq!(row.get::<Decimal, _>("average_unit_cost"), decimal(average));
}

fn version(expected_version: i64) -> VersionCommand {
    VersionCommand {
        expected_version,
        reason_code: Some("TEST".into()),
    }
}

fn b2_version(expected_version: i64) -> B2VersionCommand {
    B2VersionCommand {
        expected_version,
        reason_code: Some("TEST".into()),
    }
}

fn dec(value: &str) -> DecimalString {
    DecimalString(decimal(value))
}

fn decimal(value: &str) -> Decimal {
    Decimal::from_str(value).unwrap()
}

async fn seed(pool: &sqlx::PgPool) -> Fixture {
    let f = Fixture {
        actor: Uuid::new_v4(),
        legal_entity: Uuid::new_v4(),
        business_unit: Uuid::new_v4(),
        warehouse: Uuid::new_v4(),
        supplier: Uuid::new_v4(),
        uom: Uuid::new_v4(),
        sku: Uuid::new_v4(),
    };
    let category = Uuid::new_v4();
    let product = Uuid::new_v4();
    let role = Uuid::new_v4();
    sqlx::query("INSERT INTO enterprise_users(id,oidc_issuer,oidc_subject,display_name) VALUES($1,'https://issuer.test',$2,'B3 Operator')").bind(f.actor).bind(f.actor.to_string()).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO business_group_profile(id,code,name,base_currency,timezone) VALUES($1,'B3_GROUP','B3 Test Group','CNY','Asia/Shanghai')").bind(Uuid::new_v4()).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO business_legal_entities(id,code,name,country_code,functional_currency) VALUES($1,'LE_B3','B3 Legal','CN','CNY')").bind(f.legal_entity).execute(pool).await.unwrap();
    sqlx::query(
        "INSERT INTO business_units(id,legal_entity_id,code,name) VALUES($1,$2,'BU_B3','B3 Trade')",
    )
    .bind(f.business_unit)
    .bind(f.legal_entity)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO business_warehouses(id,legal_entity_id,business_unit_id,code,name) VALUES($1,$2,$3,'WH_B3','B3 Warehouse')").bind(f.warehouse).bind(f.legal_entity).bind(f.business_unit).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO business_suppliers(id,legal_entity_id,business_unit_id,code,name,payment_terms_days) VALUES($1,$2,$3,'SUP_B3','B3 Supplier',30)").bind(f.supplier).bind(f.legal_entity).bind(f.business_unit).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO business_units_of_measure(id,code,name,precision_scale) VALUES($1,'UOM_B3','Each',0)").bind(f.uom).execute(pool).await.unwrap();
    sqlx::query(
        "INSERT INTO business_product_categories(id,code,name) VALUES($1,'CAT_B3','B3 Category')",
    )
    .bind(category)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO business_products(id,code,name,category_id,base_uom_id) VALUES($1,'PROD_B3','B3 Product',$2,$3)").bind(product).bind(category).bind(f.uom).execute(pool).await.unwrap();
    sqlx::query(
        "INSERT INTO business_skus(id,product_id,code,name) VALUES($1,$2,'SKU_B3','B3 SKU')",
    )
    .bind(f.sku)
    .bind(product)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO business_roles(id,role_key,name) VALUES($1,'b3_operator','B3 Operator')",
    )
    .bind(role)
    .execute(pool)
    .await
    .unwrap();
    for permission in [
        "purchase_order:read",
        "purchase_order:create",
        "purchase_order:update_draft",
        "purchase_order:confirm",
        "purchase_order:cancel_remaining",
        "goods_receipt:read",
        "goods_receipt:create",
        "goods_receipt:confirm",
        "goods_receipt:reverse",
        "inventory:read",
        "inventory_opening:create",
        "inventory_opening:post",
        "payable:read",
        "supplier_payment:read",
        "supplier_payment:create",
        "supplier_payment:confirm",
        "supplier_payment:reverse",
        "payable_allocation:create",
        "payable_allocation:reverse",
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
    sqlx::query("INSERT INTO business_supplier_scopes(enterprise_user_id,supplier_id,granted_by) VALUES($1,$2,$1)").bind(f.actor).bind(f.supplier).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO business_unit_scopes(enterprise_user_id,business_unit_id,granted_by) VALUES($1,$2,$1)").bind(f.actor).bind(f.business_unit).execute(pool).await.unwrap();
    f
}
