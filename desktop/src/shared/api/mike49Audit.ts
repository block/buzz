import { invokeTauri } from "@/shared/api/tauri";

/**
 * MIKE-49 checkpoint 1 (Desktop fixture-only audit command). The
 * `mike49_run_fixture_audit` Tauri command is fixture-only and
 * simulated-data-only: it takes no app state, never touches a real
 * signing key, the local archive, or a real relay. `simulated` and
 * `dataSource` are always present on the response so this can never be
 * mistaken for live data. See `desktop/src-tauri/src/commands/mike49_audit/`
 * for the Rust side.
 */

export type Mike49TokenCounts = {
  turnInputTokens: number | null;
  turnOutputTokens: number | null;
  turnTotalTokens: number | null;
  cumulativeInputTokens: number | null;
  cumulativeOutputTokens: number | null;
  cumulativeTotalTokens: number | null;
  deltaReliable: boolean | null;
};

export type Mike49Cost = {
  turnCostUsd: number | null;
  cumulativeCostUsd: number | null;
  note: string;
};

export type Mike49Record = {
  scenario: string;
  result: "verified" | "verify_error";
  verifyErrorCode: string | null;
  observeOutcomeCode: string | null;
  eventId: string | null;
  signer: string | null;
  sessionId: string | null;
  turnId: string | null;
  turnSeq: number | null;
  harness: string | null;
  model: string | null;
  stopReason: string | null;
  tokens: Mike49TokenCounts | null;
  cost: Mike49Cost | null;
};

export type Mike49PaginationSummary = {
  eventsExamined: number;
  stopReason:
    | "exhausted"
    | "reached_boundary"
    | "page_cap_reached"
    | "event_cap_reached";
  localTraversalComplete: boolean;
  pageLimit: number;
  maxPagesPerCall: number | null;
  note: string;
};

export type Mike49EmptySourceProbe = {
  eventsExamined: number;
  stopReason:
    | "exhausted"
    | "reached_boundary"
    | "page_cap_reached"
    | "event_cap_reached";
  note: string;
};

export type Mike49SanitizerSummary = {
  blockedCount: number;
  blockedByKind: Record<string, number>;
  droppedFieldCount: number;
};

export type Mike49AuditReport = {
  simulated: true;
  dataSource: "in_memory_fixture";
  records: Mike49Record[];
  pagination: Mike49PaginationSummary;
  emptySourceProbe: Mike49EmptySourceProbe;
  sanitizer: Mike49SanitizerSummary;
};

/**
 * Runs the fixture-only MIKE-49 audit demo. Rejects with a fixed,
 * content-free error code string (`"fixture_build_failed"` |
 * `"pagination_failed"` | `"reconciliation_failed"`) on failure — never a
 * raw library error.
 */
export function mike49RunFixtureAudit(): Promise<Mike49AuditReport> {
  return invokeTauri<Mike49AuditReport>("mike49_run_fixture_audit");
}
