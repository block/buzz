use super::*;

pub(super) async fn concurrent_over_allocation(
    purchasing: &PurchasingService,
    receiving: &ReceivingService,
    payables: &PayablesService,
    f: &Fixture,
    date: NaiveDate,
    pool: &sqlx::PgPool,
) {
    let order = create_order(
        purchasing,
        f,
        date,
        "b3-allocation-race-order-create-0001",
        "5",
        "100",
    )
    .await;
    purchasing
        .confirm_order(
            f.actor,
            Uuid::new_v4(),
            order.id,
            "b3-allocation-race-order-confirm-0001",
            &version(1),
        )
        .await
        .unwrap();
    let line = sqlx::query_scalar("SELECT id FROM purchase_order_lines WHERE purchase_order_id=$1")
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
        "5",
        "b3-allocation-race-receipt-create-0001",
    )
    .await;
    receiving
        .confirm_receipt(
            f.actor,
            Uuid::new_v4(),
            receipt.id,
            "b3-allocation-race-receipt-confirm-0001",
            &version(1),
        )
        .await
        .unwrap();
    let payable_id = sqlx::query_scalar("SELECT id FROM trade_payables WHERE goods_receipt_id=$1")
        .bind(receipt.id)
        .fetch_one(pool)
        .await
        .unwrap();
    let mut payment_ids = Vec::new();
    for sequence in [1, 2] {
        let payment = payables
            .create_payment(
                f.actor,
                Uuid::new_v4(),
                &format!("b3-allocation-race-payment-create-{sequence:04}"),
                &CreateSupplierPayment {
                    legal_entity_id: f.legal_entity,
                    supplier_id: f.supplier,
                    currency: "CNY".into(),
                    payment_date: date,
                    amount: dec("300"),
                    payment_method: "bank_transfer".into(),
                    external_reference: None,
                    business_note: None,
                },
            )
            .await
            .unwrap();
        payables
            .confirm_payment(
                f.actor,
                Uuid::new_v4(),
                payment.id,
                &format!("b3-allocation-race-payment-confirm-{sequence:04}"),
                &version(1),
            )
            .await
            .unwrap();
        payment_ids.push(payment.id);
    }
    let input = |payable_id| ApplySupplierPayment {
        expected_payment_version: 2,
        allocations: vec![PayableAllocationInput {
            payable_id,
            amount: dec("300"),
        }],
    };
    let left_input = input(payable_id);
    let right_input = input(payable_id);
    let left = payables.apply_payment(
        f.actor,
        Uuid::new_v4(),
        payment_ids[0],
        "b3-allocation-race-apply-0001",
        &left_input,
    );
    let right = payables.apply_payment(
        f.actor,
        Uuid::new_v4(),
        payment_ids[1],
        "b3-allocation-race-apply-0002",
        &right_input,
    );
    let (left, right) = tokio::join!(left, right);
    assert!(matches!(
        (left, right),
        (Ok(_), Err(business_core::b2::DomainError::OverAllocation))
            | (Err(business_core::b2::DomainError::OverAllocation), Ok(_))
    ));
    let open: Decimal = sqlx::query_scalar("SELECT open_amount FROM trade_payables WHERE id=$1")
        .bind(payable_id)
        .fetch_one(pool)
        .await
        .unwrap();
    assert_eq!(open, decimal("200"));
}
