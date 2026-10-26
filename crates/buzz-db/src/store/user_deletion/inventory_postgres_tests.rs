use super::*;

#[tokio::test]
#[ignore = "requires Postgres"]
async fn user_approval_validates_inventory_shape_before_saving_evidence() {
    let pool = sqlx::PgPool::connect(&crate::test_support::database_url())
        .await
        .unwrap();
    let db = Db::from_pool(pool.clone());
    let users = db.user_deletion_store();
    let request = users
        .submit("demo:invalid-scope", "operator", None)
        .await
        .unwrap();
    let mut inventory = request.inventory_manifest.unwrap();
    inventory["communities"][0]["relationship"] = "owner".into();
    let digest = crate::entity_deletion::inventory_digest(&inventory).unwrap();
    sqlx::query("UPDATE entity_deletion_requests SET inventory_manifest = $2, inventory_digest = $3 WHERE id = $1")
        .bind(request.id).bind(inventory).bind(&digest).execute(&pool).await.unwrap();
    assert!(users
        .approve(request.id, "approver", &digest)
        .await
        .is_err());
    let after = db
        .entity_deletion_store()
        .inspect(request.id)
        .await
        .unwrap();
    assert_eq!(after.request.status, "inventoried");
    assert!(after.approvals.is_empty());
    assert!(after.stage_history.is_empty());
}
