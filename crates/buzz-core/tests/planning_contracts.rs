use buzz_core::planning::{MissionConstraintV1, PlanningProjectV1, PlanningTaskV1};
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
