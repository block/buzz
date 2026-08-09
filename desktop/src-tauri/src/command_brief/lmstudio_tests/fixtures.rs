use std::collections::BTreeSet;

use super::*;

pub(super) fn parse_specialist(value: Value, adviser: AdviserId) -> AdviserContribution {
    AdviserContribution::parse_for_adviser(
        value,
        adviser,
        &BTreeSet::from(["ledger-1".to_string()]),
    )
    .expect("specialist contribution")
}

pub(super) fn specialist_contributions() -> Vec<AdviserContribution> {
    [
        (
            AdviserId::Operations,
            "operations",
            "operations",
            "Machinery is within limits.",
        ),
        (
            AdviserId::Intelligence,
            "intelligence",
            "intelligence",
            "The operating environment is assessed.",
        ),
        (
            AdviserId::Logistics,
            "logistics",
            "logistics",
            "Sustainment dependencies are identified.",
        ),
        (
            AdviserId::Navigation,
            "navigation",
            "navigation",
            "Navigation considerations are bounded.",
        ),
        (
            AdviserId::DailyRoutine,
            "daily_routine",
            "daily_routine",
            "Daily routine is supported.",
        ),
        (
            AdviserId::Reporting,
            "reporting",
            "reports",
            "Reporting is current.",
        ),
        (
            AdviserId::Plans,
            "plans",
            "planning_30_60_90",
            "Plans are source-backed.",
        ),
    ]
    .into_iter()
    .map(|(adviser, wire_adviser, section, text)| {
        parse_specialist(
            contribution_value(wire_adviser, section, text, &["ledger-1"]),
            adviser,
        )
    })
    .collect()
}
