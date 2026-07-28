//! Strict planning contracts and deterministic working-day scheduling.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use chrono::{Datelike, Duration, NaiveDate};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Lifecycle state for a bounded planning project.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ProjectStatus {
    /// Still being assembled.
    Draft,
    /// Approved for active management.
    Active,
    /// Mission-ready outcome achieved.
    Complete,
    /// Project deliberately stopped.
    Cancelled,
}

/// Lifecycle state for one planning task.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum TaskStatus {
    /// Work has not begun.
    NotStarted,
    /// Work is underway.
    InProgress,
    /// Work cannot progress.
    Blocked,
    /// Work is complete.
    Complete,
    /// Work is no longer required.
    Cancelled,
}

/// Lifecycle and disposition state for a mission constraint.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ConstraintStatus {
    /// Constraint remains unresolved.
    Open,
    /// Mitigation is in place but the constraint remains relevant.
    Mitigated,
    /// Constraint has been explicitly resolved.
    Resolved,
    /// The mission was changed to accommodate the constraint.
    MissionChanged,
    /// Candidate for a future OPLIM workflow.
    OplimCandidate,
    /// Candidate for a future operational-risk workflow.
    RiskCandidate,
}

/// Nature of a mission constraint.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ConstraintType {
    /// Equipment or system defect.
    Defect,
    /// Capability is absent or not available.
    MissingCapability,
    /// Readiness condition is not met.
    Readiness,
    /// Dependency is controlled externally.
    ExternalDependency,
    /// Planning assumption needs confirmation.
    Assumption,
}

/// Operational significance of a mission constraint.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ConstraintSeverity {
    /// Limited effect.
    Low,
    /// Material but manageable effect.
    Medium,
    /// Major effect requiring command attention.
    High,
    /// Mission-threatening effect.
    Critical,
}

/// Owner-authored planning project.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(
    rename_all = "camelCase",
    deny_unknown_fields,
    try_from = "PlanningProjectWire"
)]
pub struct PlanningProjectV1 {
    /// Contract version.
    pub schema_version: u8,
    /// Stable project identifier.
    pub id: String,
    /// Human-readable title.
    pub title: String,
    /// Purpose or mission outcome.
    pub purpose: String,
    /// Desired mission-ready date.
    pub mission_ready_date: String,
    /// Current project status.
    pub status: ProjectStatus,
    /// Overall reported progress.
    pub progress_percent: u8,
    /// Responsible owner.
    pub owner: String,
    /// Battle Rhythm activity identifiers associated with the project.
    pub linked_activity_ids: Vec<String>,
    /// Explicit planning assumptions.
    pub assumptions: Vec<String>,
    /// Record creation time.
    pub created_at: String,
    /// Last update time.
    pub updated_at: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PlanningProjectWire {
    schema_version: u8,
    id: String,
    title: String,
    purpose: String,
    mission_ready_date: String,
    status: ProjectStatus,
    progress_percent: u8,
    owner: String,
    linked_activity_ids: Vec<String>,
    assumptions: Vec<String>,
    created_at: String,
    updated_at: String,
}

/// Owner-authored leaf or summary task.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(
    rename_all = "camelCase",
    deny_unknown_fields,
    try_from = "PlanningTaskWire"
)]
pub struct PlanningTaskV1 {
    /// Contract version.
    pub schema_version: u8,
    /// Stable task identifier.
    pub id: String,
    /// Parent project identifier.
    pub project_id: String,
    /// Work-breakdown position.
    pub wbs: String,
    /// Optional summary-task parent.
    pub parent_task_id: Option<String>,
    /// Task title.
    pub title: String,
    /// Responsible owner.
    pub owner: String,
    /// Current task state.
    pub status: TaskStatus,
    /// Reported completion from 0 to 100.
    pub percent_complete: u8,
    /// Planned start date.
    pub planned_start: Option<String>,
    /// Current due date.
    pub due_date: Option<String>,
    /// Working-day duration for a leaf task.
    pub duration_workdays: Option<u32>,
    /// Finish-to-start predecessor task identifiers.
    pub dependency_ids: Vec<String>,
    /// Optional immovable start constraint.
    pub fixed_start: Option<String>,
    /// Optional linked capability identifier.
    pub linked_capability_id: Option<String>,
    /// Optional linked mission requirement identifier.
    pub linked_mission_requirement_id: Option<String>,
    /// Supporting notes.
    pub notes: Option<String>,
    /// Row, cell, page, or other source evidence.
    pub source_evidence: Option<String>,
    /// True when dates are derived from descendants.
    pub is_summary: bool,
    /// Record creation time.
    pub created_at: String,
    /// Last update time.
    pub updated_at: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PlanningTaskWire {
    schema_version: u8,
    id: String,
    project_id: String,
    wbs: String,
    parent_task_id: Option<String>,
    title: String,
    owner: String,
    status: TaskStatus,
    percent_complete: u8,
    planned_start: Option<String>,
    due_date: Option<String>,
    duration_workdays: Option<u32>,
    dependency_ids: Vec<String>,
    fixed_start: Option<String>,
    linked_capability_id: Option<String>,
    linked_mission_requirement_id: Option<String>,
    notes: Option<String>,
    source_evidence: Option<String>,
    is_summary: bool,
    created_at: String,
    updated_at: String,
}

/// Owner-authored condition that can affect mission achievement.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(
    rename_all = "camelCase",
    deny_unknown_fields,
    try_from = "MissionConstraintWire"
)]
pub struct MissionConstraintV1 {
    /// Contract version.
    pub schema_version: u8,
    /// Stable constraint identifier.
    pub id: String,
    /// Parent project identifier.
    pub project_id: String,
    /// Constraint category.
    #[serde(rename = "type")]
    pub constraint_type: ConstraintType,
    /// Description of the condition and effect.
    pub description: String,
    /// Responsible owner.
    pub owner: String,
    /// Operational significance.
    pub severity: ConstraintSeverity,
    /// Current disposition.
    pub status: ConstraintStatus,
    /// Optional linked mission requirement.
    pub linked_mission_requirement_id: Option<String>,
    /// Optional linked capability.
    pub linked_capability_id: Option<String>,
    /// Optional linked task.
    pub linked_task_id: Option<String>,
    /// Optional linked milestone.
    pub linked_milestone_id: Option<String>,
    /// Required resolution date when known.
    pub required_date: Option<String>,
    /// Mitigation or command disposition.
    pub disposition_note: Option<String>,
    /// Source evidence.
    pub source_evidence: Option<String>,
    /// Record creation time.
    pub created_at: String,
    /// Last update time.
    pub updated_at: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct MissionConstraintWire {
    schema_version: u8,
    id: String,
    project_id: String,
    #[serde(rename = "type")]
    constraint_type: ConstraintType,
    description: String,
    owner: String,
    severity: ConstraintSeverity,
    status: ConstraintStatus,
    linked_mission_requirement_id: Option<String>,
    linked_capability_id: Option<String>,
    linked_task_id: Option<String>,
    linked_milestone_id: Option<String>,
    required_date: Option<String>,
    disposition_note: Option<String>,
    source_evidence: Option<String>,
    created_at: String,
    updated_at: String,
}

fn bounded(value: &str, field: &str, max: usize) -> Result<(), String> {
    if value.trim().is_empty() || value.len() > max {
        Err(format!("{field} must be bounded nonempty text"))
    } else {
        Ok(())
    }
}

fn date(value: &str, field: &str) -> Result<NaiveDate, String> {
    NaiveDate::parse_from_str(value, "%Y-%m-%d").map_err(|_| format!("{field} must be YYYY-MM-DD"))
}

fn timestamp(value: &str, field: &str) -> Result<(), String> {
    chrono::DateTime::parse_from_rfc3339(value)
        .map(|_| ())
        .map_err(|_| format!("{field} must be RFC3339"))
}

impl TryFrom<PlanningProjectWire> for PlanningProjectV1 {
    type Error = String;

    fn try_from(value: PlanningProjectWire) -> Result<Self, Self::Error> {
        if value.schema_version != 1 || value.progress_percent > 100 {
            return Err("invalid project version or progress".to_owned());
        }
        bounded(&value.id, "id", 256)?;
        bounded(&value.title, "title", 512)?;
        bounded(&value.purpose, "purpose", 8_192)?;
        bounded(&value.owner, "owner", 512)?;
        date(&value.mission_ready_date, "missionReadyDate")?;
        timestamp(&value.created_at, "createdAt")?;
        timestamp(&value.updated_at, "updatedAt")?;
        Ok(Self {
            schema_version: value.schema_version,
            id: value.id,
            title: value.title,
            purpose: value.purpose,
            mission_ready_date: value.mission_ready_date,
            status: value.status,
            progress_percent: value.progress_percent,
            owner: value.owner,
            linked_activity_ids: value.linked_activity_ids,
            assumptions: value.assumptions,
            created_at: value.created_at,
            updated_at: value.updated_at,
        })
    }
}

impl TryFrom<PlanningTaskWire> for PlanningTaskV1 {
    type Error = String;

    fn try_from(value: PlanningTaskWire) -> Result<Self, Self::Error> {
        if value.schema_version != 1 || value.percent_complete > 100 {
            return Err("invalid task version or progress".to_owned());
        }
        for (field, item) in [
            ("id", value.id.as_str()),
            ("projectId", value.project_id.as_str()),
            ("wbs", value.wbs.as_str()),
            ("title", value.title.as_str()),
            ("owner", value.owner.as_str()),
        ] {
            bounded(item, field, 512)?;
        }
        if value.parent_task_id.as_deref() == Some(&value.id)
            || value.dependency_ids.iter().any(|id| id == &value.id)
        {
            return Err("task cannot reference itself".to_owned());
        }
        for (field, item) in [
            ("plannedStart", value.planned_start.as_deref()),
            ("dueDate", value.due_date.as_deref()),
            ("fixedStart", value.fixed_start.as_deref()),
        ] {
            if let Some(item) = item {
                date(item, field)?;
            }
        }
        if !value.is_summary
            && value.status != TaskStatus::Complete
            && value.status != TaskStatus::Cancelled
            && value.duration_workdays.unwrap_or(0) == 0
        {
            return Err("incomplete leaf task requires positive duration".to_owned());
        }
        timestamp(&value.created_at, "createdAt")?;
        timestamp(&value.updated_at, "updatedAt")?;
        Ok(Self {
            schema_version: value.schema_version,
            id: value.id,
            project_id: value.project_id,
            wbs: value.wbs,
            parent_task_id: value.parent_task_id,
            title: value.title,
            owner: value.owner,
            status: value.status,
            percent_complete: value.percent_complete,
            planned_start: value.planned_start,
            due_date: value.due_date,
            duration_workdays: value.duration_workdays,
            dependency_ids: value.dependency_ids,
            fixed_start: value.fixed_start,
            linked_capability_id: value.linked_capability_id,
            linked_mission_requirement_id: value.linked_mission_requirement_id,
            notes: value.notes,
            source_evidence: value.source_evidence,
            is_summary: value.is_summary,
            created_at: value.created_at,
            updated_at: value.updated_at,
        })
    }
}

impl TryFrom<MissionConstraintWire> for MissionConstraintV1 {
    type Error = String;

    fn try_from(value: MissionConstraintWire) -> Result<Self, Self::Error> {
        if value.schema_version != 1 {
            return Err("invalid constraint version".to_owned());
        }
        bounded(&value.id, "id", 256)?;
        bounded(&value.project_id, "projectId", 256)?;
        bounded(&value.description, "description", 8_192)?;
        bounded(&value.owner, "owner", 512)?;
        if value
            .required_date
            .as_deref()
            .is_some_and(|item| date(item, "requiredDate").is_err())
        {
            return Err("requiredDate must be YYYY-MM-DD".to_owned());
        }
        if value.linked_mission_requirement_id.is_none()
            && value.linked_capability_id.is_none()
            && value.linked_task_id.is_none()
            && value.linked_milestone_id.is_none()
        {
            return Err(
                "constraint requires a mission, capability, task, or milestone link".to_owned(),
            );
        }
        if matches!(
            value.status,
            ConstraintStatus::OplimCandidate | ConstraintStatus::RiskCandidate
        ) && value
            .disposition_note
            .as_deref()
            .is_none_or(|note| note.trim().is_empty())
        {
            return Err("candidate disposition requires a note".to_owned());
        }
        timestamp(&value.created_at, "createdAt")?;
        timestamp(&value.updated_at, "updatedAt")?;
        Ok(Self {
            schema_version: value.schema_version,
            id: value.id,
            project_id: value.project_id,
            constraint_type: value.constraint_type,
            description: value.description,
            owner: value.owner,
            severity: value.severity,
            status: value.status,
            linked_mission_requirement_id: value.linked_mission_requirement_id,
            linked_capability_id: value.linked_capability_id,
            linked_task_id: value.linked_task_id,
            linked_milestone_id: value.linked_milestone_id,
            required_date: value.required_date,
            disposition_note: value.disposition_note,
            source_evidence: value.source_evidence,
            created_at: value.created_at,
            updated_at: value.updated_at,
        })
    }
}

/// Working days and explicitly excluded dates.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkingCalendar {
    /// ISO weekday numbers where Monday is 1 and Sunday is 7.
    pub working_weekdays: BTreeSet<u32>,
    /// Non-working dates such as public holidays or planned stand-downs.
    pub excluded_dates: BTreeSet<NaiveDate>,
}

impl Default for WorkingCalendar {
    fn default() -> Self {
        Self {
            working_weekdays: [1, 2, 3, 4, 5].into_iter().collect(),
            excluded_dates: BTreeSet::new(),
        }
    }
}

/// Input to the deterministic planning schedule engine.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PlanningScheduleInput {
    /// Project being scheduled.
    pub project: PlanningProjectV1,
    /// Project tasks.
    pub tasks: Vec<PlanningTaskV1>,
    /// Working calendar.
    #[serde(default)]
    pub working_calendar: WorkingCalendar,
    /// Date used for overdue presentation.
    pub today: NaiveDate,
}

/// Calculated dates and float for one task.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScheduledTask {
    /// Task identifier.
    pub task_id: String,
    /// Calculated earliest start.
    pub earliest_start: NaiveDate,
    /// Calculated earliest finish.
    pub earliest_finish: NaiveDate,
    /// Calculated latest start without delaying the plan.
    pub latest_start: NaiveDate,
    /// Calculated latest finish without delaying the plan.
    pub latest_finish: NaiveDate,
    /// Available working-day float.
    pub total_float_workdays: i64,
    /// True when total float is zero.
    pub critical: bool,
    /// True for unfinished tasks whose due date is before `today`.
    pub overdue: bool,
}

/// Complete deterministic schedule result.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PlanningSchedule {
    /// Scheduled leaf tasks in input order.
    pub tasks: Vec<ScheduledTask>,
    /// Earliest calculated project start.
    pub project_start: NaiveDate,
    /// Earliest calculated project finish.
    pub project_finish: NaiveDate,
    /// Working-day duration along the project network.
    pub project_duration_workdays: i64,
    /// True when the calculated finish is after the desired mission-ready date.
    pub mission_ready_at_risk: bool,
}

impl PlanningSchedule {
    /// Find one scheduled task.
    pub fn task(&self, id: &str) -> Option<&ScheduledTask> {
        self.tasks.iter().find(|task| task.task_id == id)
    }
}

/// Structured schedule validation or graph error.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Error)]
#[error("{message}")]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PlanningScheduleError {
    /// Stable machine-readable code.
    pub code: String,
    /// Bounded task identifiers involved in the failure.
    pub task_ids: Vec<String>,
    /// Human-readable explanation.
    pub message: String,
}

fn schedule_error(
    code: &str,
    ids: impl IntoIterator<Item = String>,
    message: &str,
) -> PlanningScheduleError {
    PlanningScheduleError {
        code: code.to_owned(),
        task_ids: ids.into_iter().take(20).collect(),
        message: message.to_owned(),
    }
}

fn is_workday(day: NaiveDate, calendar: &WorkingCalendar) -> bool {
    calendar
        .working_weekdays
        .contains(&day.weekday().number_from_monday())
        && !calendar.excluded_dates.contains(&day)
}

fn next_workday(mut day: NaiveDate, calendar: &WorkingCalendar) -> NaiveDate {
    while !is_workday(day, calendar) {
        day += Duration::days(1);
    }
    day
}

fn previous_workday(mut day: NaiveDate, calendar: &WorkingCalendar) -> NaiveDate {
    while !is_workday(day, calendar) {
        day -= Duration::days(1);
    }
    day
}

fn add_workdays(start: NaiveDate, count: u32, calendar: &WorkingCalendar) -> NaiveDate {
    let mut day = next_workday(start, calendar);
    for _ in 1..count {
        day = next_workday(day + Duration::days(1), calendar);
    }
    day
}

fn subtract_workdays(finish: NaiveDate, count: u32, calendar: &WorkingCalendar) -> NaiveDate {
    let mut day = previous_workday(finish, calendar);
    for _ in 1..count {
        day = previous_workday(day - Duration::days(1), calendar);
    }
    day
}

fn working_distance(mut from: NaiveDate, to: NaiveDate, calendar: &WorkingCalendar) -> i64 {
    let mut count = 0;
    while from < to {
        from += Duration::days(1);
        if is_workday(from, calendar) {
            count += 1;
        }
    }
    count
}

/// Calculate a finish-to-start working-day critical path.
pub fn calculate_schedule(
    input: &PlanningScheduleInput,
) -> Result<PlanningSchedule, PlanningScheduleError> {
    if input.tasks.is_empty() {
        return Err(schedule_error(
            "no_tasks",
            [],
            "The plan contains no tasks.",
        ));
    }
    if input.working_calendar.working_weekdays.is_empty()
        || input
            .working_calendar
            .working_weekdays
            .iter()
            .any(|day| !(1..=7).contains(day))
    {
        return Err(schedule_error(
            "invalid_calendar",
            [],
            "The working calendar must contain ISO weekdays 1 through 7.",
        ));
    }
    let leaves: Vec<&PlanningTaskV1> = input.tasks.iter().filter(|task| !task.is_summary).collect();
    if leaves.is_empty() {
        return Err(schedule_error(
            "no_leaf_tasks",
            [],
            "The plan contains no schedulable leaf tasks.",
        ));
    }
    let mut by_id = BTreeMap::new();
    for task in &input.tasks {
        if task.project_id != input.project.id {
            return Err(schedule_error(
                "wrong_project",
                [task.id.clone()],
                "A task belongs to another project.",
            ));
        }
        if by_id.insert(task.id.as_str(), task).is_some() {
            return Err(schedule_error(
                "duplicate_task",
                [task.id.clone()],
                "Task identifiers must be unique.",
            ));
        }
    }
    let leaf_ids: BTreeSet<&str> = leaves.iter().map(|task| task.id.as_str()).collect();
    let mut indegree: BTreeMap<&str, usize> =
        leaves.iter().map(|task| (task.id.as_str(), 0)).collect();
    let mut successors: BTreeMap<&str, Vec<&str>> = leaves
        .iter()
        .map(|task| (task.id.as_str(), Vec::new()))
        .collect();
    for task in &leaves {
        if task.duration_workdays.unwrap_or(0) == 0 {
            return Err(schedule_error(
                "invalid_duration",
                [task.id.clone()],
                "Every leaf task requires a positive working-day duration.",
            ));
        }
        for dependency in &task.dependency_ids {
            let Some(predecessor) = by_id.get(dependency.as_str()) else {
                return Err(schedule_error(
                    "missing_dependency",
                    [task.id.clone(), dependency.clone()],
                    "A finish-to-start dependency does not exist.",
                ));
            };
            if predecessor.is_summary {
                return Err(schedule_error(
                    "summary_dependency",
                    [task.id.clone(), dependency.clone()],
                    "Summary tasks cannot be dependencies.",
                ));
            }
            if !leaf_ids.contains(dependency.as_str()) {
                continue;
            }
            let Some(degree) = indegree.get_mut(task.id.as_str()) else {
                return Err(schedule_error(
                    "invalid_graph",
                    [task.id.clone()],
                    "Task graph could not be constructed.",
                ));
            };
            *degree += 1;
            let Some(next) = successors.get_mut(dependency.as_str()) else {
                return Err(schedule_error(
                    "invalid_graph",
                    [dependency.clone()],
                    "Dependency graph could not be constructed.",
                ));
            };
            next.push(task.id.as_str());
        }
    }
    let mut queue: VecDeque<&str> = indegree
        .iter()
        .filter_map(|(id, degree)| (*degree == 0).then_some(*id))
        .collect();
    let mut order = Vec::with_capacity(leaves.len());
    while let Some(id) = queue.pop_front() {
        order.push(id);
        for successor in successors.get(id).into_iter().flatten() {
            let Some(degree) = indegree.get_mut(successor) else {
                return Err(schedule_error(
                    "invalid_graph",
                    [(*successor).to_owned()],
                    "Successor graph could not be constructed.",
                ));
            };
            *degree -= 1;
            if *degree == 0 {
                queue.push_back(successor);
            }
        }
    }
    if order.len() != leaves.len() {
        let cycle = indegree
            .into_iter()
            .filter_map(|(id, degree)| (degree > 0).then_some(id.to_owned()));
        return Err(schedule_error(
            "dependency_cycle",
            cycle,
            "The task dependency graph contains a cycle.",
        ));
    }
    let fallback_start = leaves
        .iter()
        .filter_map(|task| {
            task.fixed_start
                .as_deref()
                .or(task.planned_start.as_deref())
        })
        .filter_map(|value| date(value, "start").ok())
        .min()
        .unwrap_or(input.today);
    let mut earliest: BTreeMap<&str, (NaiveDate, NaiveDate)> = BTreeMap::new();
    for id in &order {
        let task = by_id[id];
        let dependency_start = task
            .dependency_ids
            .iter()
            .filter_map(|dependency| earliest.get(dependency.as_str()).map(|(_, finish)| *finish))
            .max()
            .map(|finish| next_workday(finish + Duration::days(1), &input.working_calendar));
        let requested = task
            .fixed_start
            .as_deref()
            .or(task.planned_start.as_deref())
            .and_then(|value| date(value, "start").ok())
            .unwrap_or(fallback_start);
        let start = next_workday(
            dependency_start.map_or(requested, |dependency| dependency.max(requested)),
            &input.working_calendar,
        );
        let finish = add_workdays(
            start,
            task.duration_workdays.unwrap_or(1),
            &input.working_calendar,
        );
        earliest.insert(id, (start, finish));
    }
    let Some(project_start) = earliest.values().map(|(start, _)| *start).min() else {
        return Err(schedule_error(
            "no_tasks",
            [],
            "The plan contains no scheduled tasks.",
        ));
    };
    let Some(project_finish) = earliest.values().map(|(_, finish)| *finish).max() else {
        return Err(schedule_error(
            "no_tasks",
            [],
            "The plan contains no scheduled tasks.",
        ));
    };
    let mission_ready = date(&input.project.mission_ready_date, "missionReadyDate")
        .map_err(|message| schedule_error("invalid_project", [], &message))?;
    for task in &leaves {
        if task
            .fixed_start
            .as_deref()
            .and_then(|value| date(value, "fixedStart").ok())
            .is_some_and(|fixed| fixed > mission_ready)
        {
            return Err(schedule_error(
                "mission_ready_before_fixed_task",
                [task.id.clone()],
                "Mission-ready date is earlier than a fixed task constraint.",
            ));
        }
    }
    let mut latest: BTreeMap<&str, (NaiveDate, NaiveDate)> = BTreeMap::new();
    for id in order.iter().rev() {
        let task = by_id[id];
        let successor_latest_starts: Vec<NaiveDate> = successors[id]
            .iter()
            .filter_map(|successor| latest.get(successor).map(|(start, _)| *start))
            .collect();
        let latest_finish = successor_latest_starts
            .iter()
            .map(|start| previous_workday(*start - Duration::days(1), &input.working_calendar))
            .min()
            .unwrap_or(project_finish);
        let latest_start = subtract_workdays(
            latest_finish,
            task.duration_workdays.unwrap_or(1),
            &input.working_calendar,
        );
        latest.insert(id, (latest_start, latest_finish));
    }
    let tasks = leaves
        .iter()
        .map(|task| {
            let (earliest_start, earliest_finish) = earliest[task.id.as_str()];
            let (latest_start, latest_finish) = latest[task.id.as_str()];
            let total_float_workdays =
                working_distance(earliest_start, latest_start, &input.working_calendar);
            let overdue = !matches!(task.status, TaskStatus::Complete | TaskStatus::Cancelled)
                && task
                    .due_date
                    .as_deref()
                    .and_then(|value| date(value, "dueDate").ok())
                    .is_some_and(|due| due < input.today);
            ScheduledTask {
                task_id: task.id.clone(),
                earliest_start,
                earliest_finish,
                latest_start,
                latest_finish,
                total_float_workdays,
                critical: total_float_workdays == 0,
                overdue,
            }
        })
        .collect();
    Ok(PlanningSchedule {
        tasks,
        project_start,
        project_finish,
        project_duration_workdays: working_distance(
            previous_workday(project_start, &input.working_calendar) - Duration::days(1),
            project_finish,
            &input.working_calendar,
        ),
        mission_ready_at_risk: project_finish > mission_ready,
    })
}
