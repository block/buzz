use buzz_core::kind::{
    KIND_PLANNING_PLAYBOOK, KIND_PLANNING_TASK, KIND_PLANNING_TASK_ARTIFACT,
    KIND_PLANNING_TASK_DETAILS, KIND_PLANNING_TASK_EXECUTION,
};
use buzz_core::planning::{
    MissionConstraintV1, PlanningPlaybookV1, PlanningProjectV1, PlanningTaskArtifactV1,
    PlanningTaskDetailsV1, PlanningTaskExecutionV1, PlanningTaskV1,
};
use serde_json::json;

fn project() -> serde_json::Value {
    json!({
        "schemaVersion": 1,
        "id": "deployment-1",
        "title": "Regional logistics deployment",
        "purpose": "Sustain the assigned task group",
        "missionReadyDate": "2026-08-10",
        "status": "active",
        "progressPercent": 25,
        "owner": "Operations Officer",
        "linkedActivityIds": ["fas-activity-1"],
        "assumptions": ["Port services remain available"],
        "createdAt": "2026-07-29T00:00:00Z",
        "updatedAt": "2026-07-29T00:00:00Z"
    })
}

fn task() -> serde_json::Value {
    json!({
        "schemaVersion": 1,
        "id": "task-a",
        "projectId": "deployment-1",
        "wbs": "1.1",
        "parentTaskId": null,
        "title": "Prepare logistics support plan",
        "owner": "Logistics Officer",
        "status": "inProgress",
        "percentComplete": 20,
        "plannedStart": "2026-08-03",
        "dueDate": "2026-08-04",
        "durationWorkdays": 2,
        "dependencyIds": [],
        "fixedStart": null,
        "linkedCapabilityId": "replenishment-at-sea",
        "linkedMissionRequirementId": "sustain-task-group",
        "notes": null,
        "sourceEvidence": "Workbook row 4",
        "isSummary": false,
        "createdAt": "2026-07-29T00:00:00Z",
        "updatedAt": "2026-07-29T00:00:00Z"
    })
}

fn constraint() -> serde_json::Value {
    json!({
        "schemaVersion": 1,
        "id": "constraint-1",
        "projectId": "deployment-1",
        "type": "defect",
        "description": "Port seaboat davit unserviceable",
        "owner": "Marine Engineer Officer",
        "severity": "critical",
        "status": "open",
        "linkedMissionRequirementId": "conduct-seaboat-operations",
        "linkedCapabilityId": "seaboat-capability",
        "linkedTaskId": "repair-davit",
        "linkedMilestoneId": null,
        "requiredDate": "2026-08-10",
        "dispositionNote": null,
        "sourceEvidence": "Defect list 42",
        "createdAt": "2026-07-29T00:00:00Z",
        "updatedAt": "2026-07-29T00:00:00Z"
    })
}

fn task_details() -> serde_json::Value {
    json!({
        "schemaVersion": 1,
        "id": "details:task-a",
        "projectId": "deployment-1",
        "taskId": "task-a",
        "department": "MEO",
        "position": "Marine Engineering Officer",
        "individual": null,
        "agentId": "operations",
        "dueTime": "16:00",
        "executionMode": "hybrid",
        "outputType": "docx",
        "playbookId": null,
        "playbookRevisionId": null,
        "locked": false,
        "createdAt": "2026-07-29T00:00:00Z",
        "updatedAt": "2026-07-29T00:00:00Z"
    })
}

fn playbook() -> serde_json::Value {
    json!({
        "schemaVersion": 1,
        "id": "pre-departure",
        "title": "Pre-Departure",
        "description": "Prepare the ship for sailing.",
        "status": "active",
        "revisionId": "revision-1",
        "taskTemplates": [{
            "id": "navigation-plan",
            "title": "Navigation plan briefed",
            "instructions": "Brief the approved navigation plan.",
            "timing": "before",
            "offsetMinutes": 1440,
            "durationMinutes": 60,
            "dependencyIds": [],
            "department": "Navigation",
            "position": "Navigation Officer",
            "agentId": "navigation",
            "outputType": "response",
            "reschedulable": true,
            "locked": false,
            "linkedCapabilityId": null,
            "linkedMissionRequirementId": null
        }],
        "createdAt": "2026-07-29T00:00:00Z",
        "updatedAt": "2026-07-29T00:00:00Z"
    })
}

fn execution() -> serde_json::Value {
    json!({
        "schemaVersion": 1,
        "id": "execution-1",
        "projectId": "deployment-1",
        "taskId": "task-a",
        "status": "forReview",
        "mode": "hybrid",
        "summary": "Draft logistics plan prepared.",
        "body": "The plan uses the evidence available at execution time.",
        "missingInputs": ["MEO defect update"],
        "assumptions": ["Port services remain available"],
        "provider": "litellm",
        "model": "gpt-5.4",
        "startedAt": "2026-07-29T00:00:00Z",
        "completedAt": "2026-07-29T00:05:00Z",
        "error": null,
        "lateStart": false
    })
}

fn artifact() -> serde_json::Value {
    json!({
        "schemaVersion": 1,
        "id": "artifact-1",
        "projectId": "deployment-1",
        "taskId": "task-a",
        "executionId": "execution-1",
        "fileName": "logistics-support-plan.docx",
        "path": "/Users/test/Command Adviser/logistics-support-plan.docx",
        "format": "docx",
        "storageState": "icloud",
        "agentId": "operations",
        "provider": "litellm",
        "model": "gpt-5.4",
        "summary": "Draft logistics plan.",
        "missingInputWarning": "MEO defect update",
        "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "sizeBytes": 2048,
        "createdAt": "2026-07-29T00:00:00Z"
    })
}

#[test]
fn accepts_exact_planning_contracts() {
    serde_json::from_value::<PlanningProjectV1>(project()).unwrap();
    serde_json::from_value::<PlanningTaskV1>(task()).unwrap();
    serde_json::from_value::<MissionConstraintV1>(constraint()).unwrap();
}

#[test]
fn rejects_unknown_fields_invalid_progress_and_self_references() {
    let mut extra = task();
    extra["unexpected"] = json!(true);
    assert!(serde_json::from_value::<PlanningTaskV1>(extra).is_err());

    let mut progress = task();
    progress["percentComplete"] = json!(101);
    assert!(serde_json::from_value::<PlanningTaskV1>(progress).is_err());

    let mut parent = task();
    parent["parentTaskId"] = json!("task-a");
    assert!(serde_json::from_value::<PlanningTaskV1>(parent).is_err());

    let mut dependency = task();
    dependency["dependencyIds"] = json!(["task-a"]);
    assert!(serde_json::from_value::<PlanningTaskV1>(dependency).is_err());
}

#[test]
fn candidate_constraints_require_a_disposition_note_and_link() {
    let mut candidate = constraint();
    candidate["status"] = json!("oplimCandidate");
    assert!(serde_json::from_value::<MissionConstraintV1>(candidate.clone()).is_err());
    candidate["dispositionNote"] = json!("CO consideration required");
    serde_json::from_value::<MissionConstraintV1>(candidate).unwrap();

    let mut unlinked = constraint();
    unlinked["linkedMissionRequirementId"] = json!(null);
    unlinked["linkedCapabilityId"] = json!(null);
    unlinked["linkedTaskId"] = json!(null);
    assert!(serde_json::from_value::<MissionConstraintV1>(unlinked).is_err());
}

#[test]
fn accepts_project_execution_companion_contracts_and_for_review_tasks() {
    serde_json::from_value::<PlanningTaskDetailsV1>(task_details()).unwrap();
    serde_json::from_value::<PlanningPlaybookV1>(playbook()).unwrap();
    serde_json::from_value::<PlanningTaskExecutionV1>(execution()).unwrap();
    serde_json::from_value::<PlanningTaskArtifactV1>(artifact()).unwrap();

    let mut review = task();
    review["status"] = json!("forReview");
    serde_json::from_value::<PlanningTaskV1>(review).unwrap();
}

#[test]
fn rejects_invalid_project_execution_contracts() {
    let mut bad_time = task_details();
    bad_time["dueTime"] = json!("4pm");
    assert!(serde_json::from_value::<PlanningTaskDetailsV1>(bad_time).is_err());

    let mut relative = artifact();
    relative["path"] = json!("relative/file.docx");
    assert!(serde_json::from_value::<PlanningTaskArtifactV1>(relative).is_err());

    let mut self_dependency = playbook();
    self_dependency["taskTemplates"][0]["dependencyIds"] = json!(["navigation-plan"]);
    assert!(serde_json::from_value::<PlanningPlaybookV1>(self_dependency).is_err());

    let mut too_many = execution();
    too_many["missingInputs"] = json!((0..129)
        .map(|index| format!("input-{index}"))
        .collect::<Vec<_>>());
    assert!(serde_json::from_value::<PlanningTaskExecutionV1>(too_many).is_err());
}

#[test]
fn project_execution_kinds_are_unique_parameterized_replaceable_values() {
    let values = [
        KIND_PLANNING_TASK,
        KIND_PLANNING_TASK_DETAILS,
        KIND_PLANNING_PLAYBOOK,
        KIND_PLANNING_TASK_EXECUTION,
        KIND_PLANNING_TASK_ARTIFACT,
    ];
    let unique = values
        .into_iter()
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(unique.len(), values.len());
    assert!(values.iter().all(|kind| (30_000..=39_999).contains(kind)));
}
