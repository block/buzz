use super::*;

const OTHER_KEY: &str = "2222222222222222222222222222222222222222222222222222222222222222";
const ACTOR: &str = "3333333333333333333333333333333333333333333333333333333333333333";

async fn snapshot(pool: &PgPool, tables: &[&str]) -> Value {
    let mut result = serde_json::Map::new();
    for table in tables {
        let sql = format!(
            "SELECT COALESCE(jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)), '[]') FROM {table} t"
        );
        result.insert(
            (*table).into(),
            sqlx::query_scalar::<_, Value>(sqlx::AssertSqlSafe(sql))
                .fetch_one(pool)
                .await
                .unwrap(),
        );
    }
    result.into()
}

async fn seed_feedback(f: &Fixture) {
    sqlx::query("INSERT INTO product_feedback(community_id,event_id,submitter_pubkey,body,tags,event_created_at)
        SELECT community,decode(md5(pubkey||COALESCE(community::text,'global'))||md5(pubkey||COALESCE(community::text,'global')),'hex'),decode(pubkey,'hex'),'private feedback','[[\"detail\",\"private\"]]',now()
        FROM (VALUES ($1::uuid),($2::uuid),(NULL::uuid)) c(community)
        CROSS JOIN (VALUES ($3::text),($4::text)) p(pubkey)")
        .bind(f.options.community).bind(f.other).bind(TARGET).bind(OTHER_KEY).execute(&f.pool).await.unwrap();
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn global_roles_listeners_feedback_preview_audit_cap_and_retry() {
    let f = Fixture::new().await;
    seed_feedback(&f).await;
    sqlx::query("INSERT INTO relay_operators(pubkey,role,added_by) VALUES (decode($1,'hex'),'operator',decode($2,'hex')),(decode($2,'hex'),'operator',decode($1,'hex'))")
        .bind(TARGET).bind(ACTOR).execute(&f.pool).await.unwrap();
    sqlx::query("INSERT INTO operator_listener_pubkeys(listener_pubkey,target_pubkey) SELECT decode(l,'hex'),decode(t,'hex') FROM (VALUES ($1::text,$2::text),($2,$1),($2,$2)) v(l,t)")
        .bind(TARGET).bind(OTHER_KEY).execute(&f.pool).await.unwrap();
    let tables = [
        "relay_operators",
        "relay_operator_audit",
        "operator_listener_pubkeys",
        "product_feedback",
    ];
    let before = snapshot(&f.pool, &tables).await;
    let mut options = GlobalAccountBatchOptions {
        pubkey: TARGET.into(),
        actor: ACTOR.into(),
        batch_size: 1,
        max_items: 100,
    };
    let store = f.store();
    let preview = store
        .run_global_account_batches(&options, ExecutionMode::ReadOnly, &mut Vec::new())
        .await
        .unwrap();
    assert_eq!(preview.items, 6); // role, both listener directions, feedback in both communities and NULL
    assert_eq!(snapshot(&f.pool, &tables).await, before);
    options.max_items = 1;
    let first = store
        .run_global_account_batches(&options, ExecutionMode::Delete, &mut Vec::new())
        .await
        .unwrap();
    assert_eq!(first.items, 1);
    assert!(!first.supported_scope_exhausted);
    let audit: (Vec<u8>, Vec<u8>, String, String) =
        sqlx::query_as("SELECT actor_pubkey,target_pubkey,op,prev_role FROM relay_operator_audit")
            .fetch_one(&f.pool)
            .await
            .unwrap();
    assert_eq!(
        audit,
        (
            hex::decode(ACTOR).unwrap(),
            hex::decode(TARGET).unwrap(),
            "revoke".into(),
            "operator".into()
        )
    );
    options.max_items = 100;
    assert_eq!(
        store
            .run_global_account_batches(&options, ExecutionMode::Delete, &mut Vec::new())
            .await
            .unwrap()
            .items,
        5
    );
    let after = snapshot(&f.pool, &tables).await;
    for table in [
        "relay_operators",
        "operator_listener_pubkeys",
        "product_feedback",
    ] {
        let expected: Vec<_> = before[table]
            .as_array()
            .unwrap()
            .iter()
            .filter(|row| match table {
                "relay_operators" => row["pubkey"] != format!("\\x{TARGET}"),
                "operator_listener_pubkeys" => {
                    row["listener_pubkey"] != format!("\\x{TARGET}")
                        && row["target_pubkey"] != format!("\\x{TARGET}")
                }
                _ => row["submitter_pubkey"] != format!("\\x{TARGET}"),
            })
            .cloned()
            .collect();
        assert_eq!(after[table], json!(expected));
    }
    assert_eq!(
        store
            .run_global_account_batches(&options, ExecutionMode::Delete, &mut Vec::new())
            .await
            .unwrap()
            .items,
        0
    );
    assert_eq!(snapshot(&f.pool, &tables).await, after); // retry never duplicates audit
    assert_eq!(
        f.snapshot().await["api_tokens"].as_array().unwrap().len(),
        18
    ); // global != all tenant records
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn global_last_operator_rolls_back_before_other_cleanup() {
    let f = Fixture::new().await;
    seed_feedback(&f).await;
    sqlx::query("INSERT INTO relay_operators(pubkey,role,added_by) VALUES (decode($1,'hex'),'operator',decode($1,'hex'))").bind(TARGET).execute(&f.pool).await.unwrap();
    let tables = [
        "relay_operators",
        "relay_operator_audit",
        "product_feedback",
    ];
    let before = snapshot(&f.pool, &tables).await;
    let result = f
        .store()
        .run_global_account_batches(
            &GlobalAccountBatchOptions {
                pubkey: TARGET.into(),
                actor: ACTOR.into(),
                batch_size: 2,
                max_items: 100,
            },
            ExecutionMode::Delete,
            &mut Vec::new(),
        )
        .await;
    assert!(matches!(result, Err(DbError::LastOperator)));
    assert_eq!(snapshot(&f.pool, &tables).await, before);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn personal_events_revisions_copies_and_community_feedback_are_removed() {
    let f = Fixture::new().await;
    seed_feedback(&f).await;
    // Every personal kind, plus content/profile/shared-list exclusions. Both live
    // and soft-deleted versions, identical IDs across communities, another author.
    sqlx::query("INSERT INTO events(community_id,id,pubkey,kind,created_at,content,tags,sig,deleted_at,d_tag)
        SELECT community,decode(md5(pubkey||kind||rev)||md5(pubkey||kind||rev),'hex'),decode(pubkey,'hex'),kind,
        '2026-11-01'::timestamptz + rev*interval '1 second','private setting','[[\"p\",\"private\"]]',decode(repeat('00',64),'hex'),CASE WHEN rev=1 THEN now() END,'private-coordinate'
        FROM (VALUES ($1::uuid),($2::uuid)) c(community) CROSS JOIN (VALUES ($3::text),($4::text)) p(pubkey)
        CROSS JOIN unnest(ARRAY[3,10000,10001,10002,10003,10030,30000,30003,30315,30078,0,9,30030,30180,39002,55555]) kind CROSS JOIN generate_series(1,2) rev")
        .bind(f.options.community).bind(f.other).bind(TARGET).bind(OTHER_KEY).execute(&f.pool).await.unwrap();
    sqlx::query("INSERT INTO event_mentions(community_id,pubkey_hex,event_id,event_created_at,event_kind) SELECT community_id,$1,id,created_at,kind FROM events").bind(ACTOR).execute(&f.pool).await.unwrap();
    sqlx::query(
        "INSERT INTO push_match_queue(community_id,event_id) SELECT community_id,id FROM events",
    )
    .execute(&f.pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO operator_listener_outbox(listener_pubkey,target_pubkey,community_id,event_id,event_kind,event_created_at) SELECT decode($1,'hex'),pubkey,community_id,id,kind,created_at FROM events").bind(ACTOR).execute(&f.pool).await.unwrap();
    let tables = [
        "events",
        "event_mentions",
        "push_match_queue",
        "operator_listener_outbox",
        "product_feedback",
        "parameterized_event_watermarks",
    ];
    let before = snapshot(&f.pool, &tables).await;
    let mut output = Vec::new();
    f.run(ExecutionMode::ReadOnly, &mut output).await.unwrap();
    assert_eq!(snapshot(&f.pool, &tables).await, before);
    let selected: BTreeSet<_> = before["events"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| {
            r["community_id"] == f.options.community.to_string()
                && r["pubkey"] == format!("\\x{TARGET}")
                && [
                    3, 10000, 10001, 10002, 10003, 10030, 30000, 30003, 30315, 30078,
                ]
                .contains(&r["kind"].as_i64().unwrap())
        })
        .map(|r| r["id"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(selected.len(), 20);
    f.run(ExecutionMode::Delete, &mut Vec::new()).await.unwrap();
    let after = snapshot(&f.pool, &tables).await;
    for table in tables {
        let expected: Vec<_> = before[table]
            .as_array()
            .unwrap()
            .iter()
            .filter(|r| {
                let in_community = r["community_id"] == f.options.community.to_string();
                if table == "product_feedback" {
                    !(in_community && r["submitter_pubkey"] == format!("\\x{TARGET}"))
                } else if table == "parameterized_event_watermarks" {
                    true
                } else {
                    !(in_community
                        && r[if table == "events" { "id" } else { "event_id" }]
                            .as_str()
                            .is_some_and(|id| selected.contains(id)))
                }
            })
            .cloned()
            .collect();
        // Membership cleanup adds signed channel and community rosters and
        // their mention entries; dedicated journeys test those independently.
        let generated: BTreeSet<_> = after["events"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|r| {
                (!r["channel_id"].is_null()
                    && [39000, 39001, 39002].contains(&r["kind"].as_i64().unwrap()))
                    || r["kind"] == 13534
            })
            .map(|r| r["id"].clone().to_string())
            .collect();
        let actual: Vec<_> = after[table]
            .as_array()
            .unwrap()
            .iter()
            .filter(|r| {
                !(matches!(table, "events" | "event_mentions")
                    && generated.contains(
                        &r[if table == "events" { "id" } else { "event_id" }].to_string(),
                    ))
            })
            .cloned()
            .collect();
        assert_eq!(json!(actual), json!(expected), "{table}");
    }
    assert_eq!(
        f.run(ExecutionMode::Delete, &mut Vec::new())
            .await
            .unwrap()
            .items,
        0
    );
    assert_eq!(snapshot(&f.pool, &tables).await, after);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn scope_refreshes_ownership_on_retry_and_covers_former_communities() {
    let f = Fixture::new().await;
    sqlx::query("UPDATE users SET agent_owner_pubkey=decode($1,'hex') WHERE community_id=$2 AND pubkey=decode($3,'hex')").bind(TARGET).bind(f.options.community).bind(OTHER_KEY).execute(&f.pool).await.unwrap();
    sqlx::query("DELETE FROM relay_members WHERE pubkey=$1")
        .bind(TARGET)
        .execute(&f.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE channel_members SET removed_at=now() WHERE pubkey=decode($1,'hex')")
        .bind(TARGET)
        .execute(&f.pool)
        .await
        .unwrap();
    let store = f.store();
    let scope = store.generate_account_scope(TARGET).await.unwrap();
    assert_eq!(scope.communities.len(), 2);
    assert_eq!(
        scope
            .communities
            .iter()
            .find(|c| c.community == f.options.community)
            .unwrap()
            .pubkeys
            .len(),
        2
    );
    assert_eq!(
        scope
            .communities
            .iter()
            .find(|c| c.community == f.other)
            .unwrap()
            .pubkeys
            .len(),
        1
    );
    let before = f.snapshot().await;
    store
        .run_account_scope(
            &scope,
            ACTOR,
            2,
            1000,
            ExecutionMode::ReadOnly,
            &mut Vec::new(),
        )
        .await
        .unwrap();
    assert_eq!(f.snapshot().await, before);
    // A saved key is not authority: transfer away after preview, then prove
    // both communities preserve that agent. A later retry discovers it anew.
    sqlx::query("UPDATE users SET agent_owner_pubkey=decode($1,'hex')")
        .bind(ACTOR)
        .execute(&f.pool)
        .await
        .unwrap();
    let saved: AccountScope = serde_json::from_value(serde_json::to_value(scope).unwrap()).unwrap();
    let first = store
        .run_account_scope(&saved, ACTOR, 2, 1, ExecutionMode::Delete, &mut Vec::new())
        .await
        .unwrap();
    assert_eq!(first.items, 1);
    assert!(!first.supported_scope_exhausted);
    let rest = store
        .run_account_scope(
            &saved,
            ACTOR,
            2,
            1000,
            ExecutionMode::Delete,
            &mut Vec::new(),
        )
        .await
        .unwrap();
    assert!(rest.supported_scope_exhausted);
    let retained: i64 =
        sqlx::query_scalar("SELECT count(*) FROM api_tokens WHERE owner_pubkey=decode($1,'hex')")
            .bind(OTHER_KEY)
            .fetch_one(&f.pool)
            .await
            .unwrap();
    assert_eq!(retained, 6);
    // Reassign only in the first community. The same file must pick this up.
    sqlx::query("UPDATE users SET agent_owner_pubkey=decode($1,'hex') WHERE community_id=$2 AND pubkey=decode($3,'hex')")
        .bind(TARGET).bind(f.options.community).bind(OTHER_KEY).execute(&f.pool).await.unwrap();
    store
        .run_account_scope(
            &saved,
            ACTOR,
            2,
            1000,
            ExecutionMode::Delete,
            &mut Vec::new(),
        )
        .await
        .unwrap();
    let tokens:Vec<(Uuid,String)>=sqlx::query_as("SELECT community_id,encode(owner_pubkey,'hex') FROM api_tokens ORDER BY community_id,owner_pubkey").fetch_all(&f.pool).await.unwrap();
    assert!(tokens
        .iter()
        .all(|(c, k)| k != TARGET && (*c != f.options.community || k != OTHER_KEY)));
    assert_eq!(
        tokens
            .iter()
            .filter(|(c, k)| *c == f.other && k == OTHER_KEY)
            .count(),
        3
    );
    assert_eq!(
        store
            .run_account_scope(
                &saved,
                ACTOR,
                2,
                1000,
                ExecutionMode::Delete,
                &mut Vec::new()
            )
            .await
            .unwrap()
            .items,
        0
    );
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn frozen_scope_rejects_owned_community_before_global_mutation() {
    let f = Fixture::new().await;
    seed_feedback(&f).await;
    sqlx::query("UPDATE relay_members SET role='owner' WHERE community_id=$1 AND pubkey=$2")
        .bind(f.other)
        .bind(TARGET)
        .execute(&f.pool)
        .await
        .unwrap();
    let scope = f.store().generate_account_scope(TARGET).await.unwrap();
    let before = snapshot(
        &f.pool,
        &["product_feedback", "relay_members", "api_tokens"],
    )
    .await;
    let error = f
        .store()
        .run_account_scope(
            &scope,
            ACTOR,
            2,
            1000,
            ExecutionMode::Delete,
            &mut Vec::new(),
        )
        .await
        .unwrap_err();
    assert!(error.to_string().contains("owned-community handoff"));
    assert_eq!(
        snapshot(
            &f.pool,
            &["product_feedback", "relay_members", "api_tokens"]
        )
        .await,
        before
    );
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn frozen_scope_without_any_community_still_cleans_global_records() {
    let pool = PgPool::connect(&crate::test_support::database_url())
        .await
        .unwrap();
    sqlx::query("INSERT INTO product_feedback(event_id,submitter_pubkey,body,event_created_at) VALUES (decode($1,'hex'),decode($1,'hex'),'private',now())").bind(TARGET).execute(&pool).await.unwrap();
    let store = Db::from_pool(pool.clone()).user_deletion_store();
    let scope = store.generate_account_scope(TARGET).await.unwrap();
    assert!(scope.communities.is_empty());
    let result = store
        .run_account_scope(
            &scope,
            ACTOR,
            2,
            1000,
            ExecutionMode::Delete,
            &mut Vec::new(),
        )
        .await
        .unwrap();
    assert_eq!(result.items, 1);
    assert!(result.supported_scope_exhausted && !result.account_erasure_complete);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM product_feedback")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
}
