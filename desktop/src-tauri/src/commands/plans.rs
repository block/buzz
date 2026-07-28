use buzz_core_pkg::planning::{
    calculate_schedule, PlanningSchedule, PlanningScheduleError, PlanningScheduleInput,
};

/// Calculate the deterministic working-day critical path for a planning project.
#[tauri::command]
pub fn calculate_plan_schedule(
    input: PlanningScheduleInput,
) -> Result<PlanningSchedule, PlanningScheduleError> {
    calculate_schedule(&input)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn malformed_schedule_input_is_rejected_before_command_execution() {
        let result = serde_json::from_value::<PlanningScheduleInput>(serde_json::json!({
            "project": {},
            "tasks": [],
            "workingCalendar": {},
            "today": "2026-07-29"
        }));
        assert!(result.is_err());
    }
}
