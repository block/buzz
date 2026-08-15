import type {
  BriefRunState,
  BriefSection,
} from "@/features/command-console/domain/briefContracts";

export const SECTION_LABELS: Record<BriefSection, string> = {
  today: "Today at a glance",
  operations: "Operational priorities and risks",
  intelligence: "Intelligence and operating environment",
  logistics: "Logistics and sustainment",
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
  "intelligence",
  "logistics",
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

const OPERATIONAL_EVIDENCE_GAP =
  "Current operational evidence is incomplete; advisers lacked a fully current Battle Rhythm, command plan, task status, dependencies, or platform state.";

const GENERIC_EVIDENCE_GAP_PATTERNS = [
  /^Evidence (?:contains|is|set contains) (?:doctrine|largely doctrinal|doctrine and)/i,
  /^Most (?:supplied )?evidence is (?:general doctrine|doctrine|largely doctrinal)/i,
  /^No signed (?:Battle Rhythm|battle rhythm|command plan)/i,
  /^Provided evidence is largely doctrinal/i,
  /^Several sources describe general/i,
  /^Sources (?:describe general|do not identify any current)/i,
];

const TECHNICAL_NOTICE_PATTERNS = [
  /^Chief of Staff model consolidation was unavailable/i,
  /^Command-team discussion memory excluded/i,
  /^Potential credential values were redacted/i,
  /^RAG source unavailable:/i,
  /^Source .+ was truncated to the canonical source-size limit\.$/i,
  /^Unsigned trusted-LAN evidence was observed/i,
  /^\d+ additional (?:source|trusted) limitations omitted/i,
];

const MAX_PRIMARY_MISSING_ITEMS = 6;

export interface MissingInformationPresentation {
  readonly connectorNotices: readonly string[];
  readonly evidenceNotices: readonly string[];
  readonly primary: readonly string[];
}

/**
 * Keeps the quarterdeck view decision-facing while retaining collection and
 * adviser diagnostics inside the evidence disclosure.
 */
export function presentMissingInformation(
  items: readonly string[],
): MissingInformationPresentation {
  const primary: string[] = [];
  const connectorNotices: string[] = [];
  const evidenceNotices: string[] = [];
  let hasGenericEvidenceGap = false;

  for (const item of new Set(items)) {
    if (item.startsWith("World Monitor ")) {
      connectorNotices.push(item);
      continue;
    }
    if (GENERIC_EVIDENCE_GAP_PATTERNS.some((pattern) => pattern.test(item))) {
      hasGenericEvidenceGap = true;
      evidenceNotices.push(item);
      continue;
    }
    if (TECHNICAL_NOTICE_PATTERNS.some((pattern) => pattern.test(item))) {
      evidenceNotices.push(item);
      continue;
    }
    if (primary.length < MAX_PRIMARY_MISSING_ITEMS) {
      primary.push(item);
    } else {
      evidenceNotices.push(item);
    }
  }

  if (hasGenericEvidenceGap) {
    if (primary.length >= MAX_PRIMARY_MISSING_ITEMS) {
      evidenceNotices.push(primary.pop() as string);
    }
    primary.unshift(OPERATIONAL_EVIDENCE_GAP);
  }

  return { connectorNotices, evidenceNotices, primary };
}
