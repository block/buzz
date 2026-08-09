use std::collections::BTreeSet;

use buzz_core::planning::{
    calculate_schedule, PlanningProjectV1, PlanningScheduleInput, PlanningTaskV1, WorkingCalendar,
};
use chrono::NaiveDate;
use serde_json::json;

fn project() -> PlanningProjectV1 {
    serde_json::from_value(json!({
        "schemaVersion": 1,
        "id": "deployment-1",
        "title": "Deployment",
        "purpose": "Mission ready",
        "missionReadyDate": "2026-08-10",
        "status": "active",
        "progressPercent": 0,
        "owner": "Operations Officer",
        "linkedActivityIds": [],
        "assumptions": [],
        "createdAt": "2026-07-29T00:00:00Z",
        "updatedAt": "2026-07-29T00:00:00Z"
    }))
    .unwrap()
}

fn task(id: &str, wbs: &str, duration: u32, dependencies: &[&str]) -> PlanningTaskV1 {
    serde_json::from_value(json!({
        "schemaVersion": 1,
        "id": id,
        "projectId": "deployment-1",
        "wbs": wbs,
        "parentTaskId": null,
        "title": id,
        "owner": "Owner",
        "status": "notStarted",
        "percentComplete": 0,
        "plannedStart": "2026-08-03",
        "dueDate": null,
        "durationWorkdays": duration,
        "dependencyIds": dependencies,
        "fixedStart": null,
        "linkedCapabilityId": null,
        "linkedMissionRequirementId": null,
        "notes": null,
        "sourceEvidence": "test fixture",
        "isSummary": false,
        "createdAt": "2026-07-29T00:00:00Z",
        "updatedAt": "2026-07-29T00:00:00Z"
    }))
    .unwrap()
}

fn input() -> PlanningScheduleInput {
    PlanningScheduleInput {
        project: project(),
        tasks: vec![
            task("A", "1", 2, &[]),
            task("B", "2", 3, &["A"]),
            task("C", "3", 1, &["A"]),
            task("D", "4", 1, &["B", "C"]),
        ],
        working_calendar: WorkingCalendar::default(),
        today: NaiveDate::from_ymd_opt(2026, 7, 29).unwrap(),
    }
}

#[test]
fn calculates_critical_path_and_float() {
    let schedule = calculate_schedule(&input()).unwrap();
    assert_eq!(schedule.project_duration_workdays, 6);
    assert_eq!(schedule.task("A").unwrap().total_float_workdays, 0);
    assert_eq!(schedule.task("B").unwrap().total_float_workdays, 0);
    assert_eq!(schedule.task("C").unwrap().total_float_workdays, 2);
    assert_eq!(schedule.task("D").unwrap().total_float_workdays, 0);
}

#[test]
fn honours_weekends_and_excluded_dates() {
    let mut input = input();
    input.working_calendar.excluded_dates =
        BTreeSet::from([NaiveDate::from_ymd_opt(2026, 8, 5).unwrap()]);
    let schedule = calculate_schedule(&input).unwrap();
    assert_eq!(
        schedule.task("A").unwrap().earliest_finish,
        NaiveDate::from_ymd_opt(2026, 8, 4).unwrap()
    );
    assert_eq!(
        schedule.task("B").unwrap().earliest_start,
        NaiveDate::from_ymd_opt(2026, 8, 6).unwrap()
    );
    assert_eq!(
        schedule.task("D").unwrap().earliest_finish,
        NaiveDate::from_ymd_opt(2026, 8, 11).unwrap()
    );
}

#[test]
fn reports_missing_dependencies_and_cycles() {
    let mut missing = input();
    missing.tasks[1].dependency_ids = vec!["missing".to_owned()];
    assert_eq!(
        calculate_schedule(&missing).unwrap_err().code,
        "missing_dependency"
    );

    let mut cycle = input();
    cycle.tasks[0].dependency_ids = vec!["D".to_owned()];
    assert_eq!(
        calculate_schedule(&cycle).unwrap_err().code,
        "dependency_cycle"
    );
}
