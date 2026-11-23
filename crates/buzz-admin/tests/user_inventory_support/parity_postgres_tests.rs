//! Exercise shared inventory selection through the actual operator commands.
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

use super::workflow_cli;

#[tokio::test]
#[ignore = "requires Postgres"]
async fn preview_and_both_submission_modes_capture_identical_inventory() {
    let url = std::env::var("BUZZ_TEST_DATABASE_URL").unwrap();
    let pool = PgPool::connect(&url).await.unwrap();
    for trusted in [false, true] {
        let human = nostr::Keys::generate().public_key().to_hex();
        let agent = nostr::Keys::generate().public_key().to_hex();
        let relay = nostr::Keys::generate().public_key().to_hex();
        let ids = [
            Uuid::new_v4(),
            Uuid::new_v4(),
            Uuid::new_v4(),
            Uuid::new_v4(),
        ];
        for id in ids {
            sqlx::query("INSERT INTO communities(id,host) VALUES($1,$2)")
                .bind(id)
                .bind(format!("{id}.example.org"))
                .execute(&pool)
                .await
                .unwrap();
        }
        sqlx::query("INSERT INTO relay_members(community_id,pubkey,role) VALUES($1,$2,'member')")
            .bind(ids[0])
            .bind(&human)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO users(community_id,pubkey,agent_owner_pubkey,deactivated_at) VALUES($1,decode($2,'hex'),decode($3,'hex'),now())")
            .bind(ids[1]).bind(&agent).bind(&human).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO events(community_id,id,pubkey,created_at,kind,tags,content,sig) VALUES($1,decode($2,'hex'),decode($3,'hex'),now(),9,$4,'old content',decode(repeat('00',64),'hex'))")
            .bind(ids[2]).bind(&human).bind(&relay).bind(json!([["actor",human]])).execute(&pool).await.unwrap();
        // ids[3] is unrelated and must never enter the inventory.
        let relay_args = if trusted {
            vec!["--relay-pubkey", relay.as_str()]
        } else {
            vec![]
        };
        let mut preview_args = vec!["inventory", "--pubkey", human.as_str()];
        preview_args.extend(&relay_args);
        let preview = workflow_cli(&url, &preview_args, true).await;
        let rows = preview["communities"].as_array().unwrap();
        assert_eq!(rows.len(), if trusted { 3 } else { 2 });
        for (id, role) in [(ids[0], "participant"), (ids[1], "past_participant")] {
            assert!(rows
                .iter()
                .any(|r| r["community_id"] == id.to_string() && r["relationship"] == role));
        }
        assert_eq!(
            rows.iter().any(|r| r["community_id"] == ids[2].to_string()),
            trusted
        );
        assert!(!rows.iter().any(|r| r["community_id"] == ids[3].to_string()));
        let mut submit_args = vec![
            "submit",
            "--pubkey",
            human.as_str(),
            "--requested-by",
            "operator",
        ];
        submit_args.extend(&relay_args);
        let deletion = workflow_cli(&url, &submit_args, true).await;
        assert_eq!(deletion["inventory_manifest"], preview);
        assert_eq!(workflow_cli(&url, &submit_args, true).await, deletion);
        let run = Uuid::new_v4().to_string();
        submit_args.extend(["--read-only", "--run-id", run.as_str()]);
        let rehearsal = workflow_cli(&url, &submit_args, true).await;
        assert_eq!(workflow_cli(&url, &submit_args, true).await, rehearsal);
        assert_ne!(deletion["id"], rehearsal["id"]);
        let mut common = rehearsal["inventory_manifest"].clone();
        for field in ["execution_mode", "run_id", "workflow_version"] {
            common.as_object_mut().unwrap().remove(field);
        }
        assert_eq!(common, preview);
        for request in [&deletion, &rehearsal] {
            let inspection =
                workflow_cli(&url, &["inspect", request["id"].as_str().unwrap()], true).await;
            assert_eq!(inspection["status"], "inventoried");
            assert_eq!(inspection["approvals"], json!([]));
            assert_eq!(inspection["stage_history"], json!([]));
        }
        // An explicit attribution change must fail before any workflow writes.
        let id = deletion["id"].as_str().unwrap();
        workflow_cli(&url, &["run", id, "--relay-pubkey", agent.as_str()], false).await;
        let after: Value = workflow_cli(&url, &["inspect", id], true).await;
        assert_eq!(after["status"], "inventoried");
        assert_eq!(after["approvals"], json!([]));
        assert_eq!(after["stage_history"], json!([]));
        assert_eq!(after["execution"]["lease_generation"], 0);
    }
}
