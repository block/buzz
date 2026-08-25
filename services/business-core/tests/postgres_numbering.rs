use business_core::{
    numbering::{allocate_number, NumberingContext, NumberingRuleService},
    PgStore,
};
use chrono::Utc;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

#[tokio::test]
async fn numbering_pools_are_isolated_by_scope_and_period() {
    let Ok(database_url) = std::env::var("BUSINESS_CORE_NUMBERING_TEST_DATABASE_URL") else {
        eprintln!("skipping: BUSINESS_CORE_NUMBERING_TEST_DATABASE_URL is not set");
        return;
    };
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect(&database_url)
        .await
        .expect("numbering test database must be reachable");
    PgStore::new(pool.clone())
        .migrate()
        .await
        .expect("numbering migrations must apply");

    let actor = Uuid::new_v4();
    let role = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO enterprise_users(id,oidc_issuer,oidc_subject,display_name) VALUES($1,'test','numbering-ledger','Numbering Ledger Test')",
    )
    .bind(actor)
    .execute(&pool)
    .await
    .expect("actor fixture must insert");
    sqlx::query(
        "INSERT INTO business_roles(id,role_key,name) VALUES($1,'numbering_test','Numbering Test')",
    )
    .bind(role)
    .execute(&pool)
    .await
    .expect("role fixture must insert");
    sqlx::query("INSERT INTO business_role_permissions(role_id,permission_key) VALUES($1,'business_numbering_rules:read')")
        .bind(role)
        .execute(&pool)
        .await
        .expect("permission fixture must insert");
    sqlx::query(
        "INSERT INTO business_user_roles(enterprise_user_id,role_id,assigned_by) VALUES($1,$2,$1)",
    )
    .bind(actor)
    .bind(role)
    .execute(&pool)
    .await
    .expect("role assignment fixture must insert");

    let legal_a = Uuid::new_v4();
    let legal_b = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO business_legal_entities(id,code,name,country_code,functional_currency) VALUES($1,'LE_A','Legal A','CN','CNY'),($2,'LE_B','Legal B','CN','CNY')",
    )
    .bind(legal_a)
    .bind(legal_b)
    .execute(&pool)
    .await
    .expect("legal entity fixtures must insert");
    sqlx::query(
        r#"UPDATE business_numbering_rules
           SET segments='[{"type":"fixed","value":"SO-"},{"type":"scope"},{"type":"fixed","value":"-"},{"type":"date","format":"YYYYMMDD"},{"type":"fixed","value":"-"},{"type":"sequence","width":4}]',
               reset_period='daily',scope_dimension='legal_entity'
           WHERE record_type='sales_order'"#,
    )
    .execute(&pool)
    .await
    .expect("numbering fixture must update");

    let first_a = allocate(&pool, legal_a).await;
    let second_a = allocate(&pool, legal_a).await;
    let first_b = allocate(&pool, legal_b).await;
    let date = Utc::now().format("%Y%m%d").to_string();
    assert_eq!(first_a, format!("SO-LE_A-{date}-0001"));
    assert_eq!(second_a, format!("SO-LE_A-{date}-0002"));
    assert_eq!(first_b, format!("SO-LE_B-{date}-0001"));

    let mut rolled_back = pool.begin().await.expect("transaction must begin");
    let rolled_back_number = allocate_number(
        &mut rolled_back,
        "sales_order",
        "SO",
        Uuid::new_v4(),
        NumberingContext::new(legal_a, None),
    )
    .await
    .expect("rolled back allocation must render");
    rolled_back
        .rollback()
        .await
        .expect("numbering transaction must roll back");
    assert_eq!(rolled_back_number, format!("SO-LE_A-{date}-0003"));
    assert_eq!(allocate(&pool, legal_a).await, rolled_back_number);

    let ledger_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM business_numbering_issuances WHERE record_type='sales_order'",
    )
    .fetch_one(&pool)
    .await
    .expect("issuance ledger must be readable");
    assert_eq!(ledger_count, 4);

    sqlx::query(
        "UPDATE business_numbering_rules SET status='disabled' WHERE record_type='sales_order'",
    )
    .execute(&pool)
    .await
    .expect("fallback fixture must update");
    let first_fallback = allocate(&pool, legal_a).await;
    let mut failed_fallback = pool.begin().await.expect("transaction must begin");
    allocate_number(
        &mut failed_fallback,
        "sales_order",
        "SO",
        Uuid::new_v4(),
        NumberingContext::new(legal_a, None),
    )
    .await
    .expect("fallback allocation must render");
    failed_fallback
        .rollback()
        .await
        .expect("fallback transaction must roll back");
    let second_fallback = allocate(&pool, legal_a).await;
    assert_ne!(first_fallback, second_fallback);
    let fallback_gap: i64 = sqlx::query_scalar(
        r#"WITH sequenced AS (
             SELECT sequence_value-lag(sequence_value) OVER (ORDER BY sequence_value)-1 gap
             FROM business_numbering_issuances
             WHERE record_type='sales_order' AND source='fallback'
           ) SELECT COALESCE(max(gap),0) FROM sequenced"#,
    )
    .fetch_one(&pool)
    .await
    .expect("fallback gap must be detectable");
    assert_eq!(fallback_gap, 1);

    let ledger = NumberingRuleService::new(PgStore::new(pool.clone()))
        .ledger(actor)
        .await
        .expect("ledger service query must succeed");
    assert_eq!(ledger.summary.gap_count, 1);
    assert_eq!(ledger.summary.fallback_count, 2);
    assert!(ledger
        .recent_issuances
        .iter()
        .any(|item| item.gap_before == 1 && item.gap_reason.is_some()));
}

async fn allocate(pool: &sqlx::PgPool, legal_entity_id: Uuid) -> String {
    let mut tx = pool.begin().await.expect("transaction must begin");
    let number = allocate_number(
        &mut tx,
        "sales_order",
        "SO",
        Uuid::new_v4(),
        NumberingContext::new(legal_entity_id, None),
    )
    .await
    .expect("number allocation must succeed");
    tx.commit().await.expect("number allocation must commit");
    number
}
