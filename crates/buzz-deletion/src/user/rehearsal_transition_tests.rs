//! Infrastructure-free routing tests; handler outcomes are inputs, not integration evidence.
use super::*;
use serde_json::json;

fn request(relationships: &[&str]) -> EntityRequest {
    let run = Uuid::new_v4();
    let subject = "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798";
    EntityRequest {
        id: Uuid::new_v4(),
        entity_type: "user".into(),
        target: format!("rehearsal:{run}:{subject}"),
        status: "approved".into(),
        requested_by: "operator".into(),
        reason: None,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
        inventory_digest: None,
        inventory_manifest: Some(json!({"execution_mode":"rehearsal","run_id":run,
            "workflow_version":2,"communities":relationships.iter().enumerate().map(|(i,r)|
                json!({"community_id":Uuid::from_u128(i as u128+1),"relationship":r})).collect::<Vec<_>>()})),
    }
}
fn stage(request: &EntityRequest, new: NewEntityStage, previous: Option<Uuid>) -> EntityStage {
    EntityStage {
        id: Uuid::new_v4(),
        request_id: request.id,
        target_community_id: new.community,
        stage: new.stage,
        status: "pending".into(),
        is_current: true,
        started_at: None,
        finished_at: None,
        metadata_json: json!({"inputs":new.inputs,"previous_stage_id":previous}),
        last_error: None,
        updated_at: chrono::Utc::now(),
    }
}

#[test]
fn rehearsal_routes_owned_and_former_communities_then_global_report_and_terminal() {
    for relationships in [
        vec![],
        vec!["owner"],
        vec!["participant"],
        vec![
            "participant",
            "owner",
            "former_participant",
            "stored_community",
        ],
        vec!["owner", "participant", "owner"],
    ] {
        let mut request = request(&relationships);
        let StageTransition::Initialize(first) = decide_next(&request, &[]).unwrap() else {
            panic!()
        };
        let mut history = vec![stage(&request, first, None)];
        let mut visited = Vec::new();
        for _ in 0..30 {
            let last = history.last_mut().unwrap();
            visited.push((last.stage.clone(), last.target_community_id));
            // Restart with unfinished action resumes the identical persisted stage.
            for status in ["pending", "running", "failed"] {
                history.last_mut().unwrap().status = status.into();
                assert!(
                    matches!(decide_next(&request,&history).unwrap(),StageTransition::Resume(id) if id==history.last().unwrap().id)
                );
            }
            let last = history.last_mut().unwrap();
            last.status = "completed".into();
            last.metadata_json["result"] =
                json!({"execution_mode":"rehearsal","account_erasure_complete":false});
            let StageTransition::Advance {
                previous,
                decision,
                successor,
            } = decide_next(&request, &history).unwrap()
            else {
                panic!()
            };
            let last = history.last_mut().unwrap();
            last.is_current = false;
            last.metadata_json["transition"] = decision;
            if let Some(next) = successor {
                history.push(stage(&request, next, Some(previous)));
            } else {
                request.status = "completed".into();
                break;
            }
        }
        assert!(matches!(
            decide_next(&request, &history).unwrap(),
            StageTransition::Finished
        ));
        let mut expected = vec![("gate_user".into(), None), ("purge_media".into(), None)];
        for (i, relationship) in relationships.iter().enumerate() {
            let community = Some(Uuid::from_u128(i as u128 + 1));
            let steps: &[&str] = if *relationship == "owner" {
                &["delete_owned_community"]
            } else {
                &["purge_content", "purge_account", "purge_profile"]
            };
            expected.extend(steps.iter().map(|s| (s.to_string(), community)));
        }
        expected.extend([
            ("global_cleanup".into(), None),
            ("remaining_cleanup".into(), None),
        ]);
        assert_eq!(visited, expected);
        history[0].metadata_json["inputs"]["target"] = "different".into();
        assert!(decide_next(&request, &history).is_err());
    }
}

#[test]
fn rehearsal_mode_and_identity_cannot_be_reused_for_deletion() {
    use super::super::worker::validate_mode;
    let mut request = request(&[]);
    assert!(validate_mode(&request, false, true).is_ok());
    for (delete, read_only) in [(true, false), (true, true), (false, false)] {
        assert!(validate_mode(&request, delete, read_only).is_err());
    }
    request.inventory_manifest.as_mut().unwrap()["run_id"] = Uuid::new_v4().to_string().into();
    assert!(Inventory::read(&request).is_err());
    request.target = Inventory::subject(&self::request(&[])).unwrap().into();
    assert!(Inventory::is_rehearsal(&request).is_err());
    request.inventory_manifest = Some(json!({"communities":[]}));
    assert!(validate_mode(&request, true, false).is_ok());
    assert!(validate_mode(&request, false, true).is_err());
    // Old rehearsal identities remain parseable without changing saved approval inputs.
    request.target = format!("rehearsal:{}", request.target);
    request.inventory_manifest.as_mut().unwrap()["execution_mode"] = "rehearsal".into();
    assert!(Inventory::read(&request).is_ok());
}
