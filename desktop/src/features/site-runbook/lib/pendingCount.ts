/** Pure pending-procedure counters for sidebar / Runbook chip badges. */

import type { SiteRunbook, SiteRunbooksBlob } from "./types";

/** Pending proposals on one runbook. */
export function countPendingInRunbook(
  runbook: SiteRunbook | null | undefined,
): number {
  if (!runbook) return 0;
  let count = 0;
  for (const procedure of runbook.procedures) {
    if (procedure.status === "pending") count += 1;
  }
  return count;
}

/** Pending proposals across the whole site-runbook blob. */
export function countPendingProcedures(
  blob: SiteRunbooksBlob | null | undefined,
): number {
  if (!blob) return 0;
  let count = 0;
  for (const runbook of Object.values(blob.runbooks)) {
    count += countPendingInRunbook(runbook);
  }
  return count;
}
