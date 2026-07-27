import type {
  BriefRunState,
  BriefSection,
} from "@/features/command-console/domain/briefContracts";

export const SECTION_LABELS: Record<BriefSection, string> = {
  today: "Today at a glance",
  operations: "Operational priorities and risks",
  navigation: "Navigation considerations",
  daily_routine: "Daily routine and calendar",
  reports: "Reports and returns due",
  planning_30_60_90: "30, 60 and 90-day outlook",
  decisions: "Decisions and approvals required",
  conflicts_and_gaps: "Conflicts and gaps",
  sources: "Source notes",
};

export const COMMAND_READING_ORDER: readonly BriefSection[] = [
  "decisions",
  "today",
  "operations",
  "navigation",
  "daily_routine",
  "reports",
  "planning_30_60_90",
];

export const STATE_LABELS: Record<BriefRunState, string> = {
  queued: "Queued",
  collecting_sources: "Collecting sources",
  running_specialists: "Running specialists",
  consolidating: "Consolidating",
  persisting: "Securing brief",
  completed: "Complete",
  degraded: "Complete with limitations",
  cancelled: "Cancelled",
  failed: "Failed",
};
