// Domain logic for the community-moderation admin queue (U2 admin surface).
//
// Pure, hook-free transforms over the NIP-98 `/moderation/*` read contract so
// they can be unit-tested without a relay. The wire shapes below mirror the
// authoritative JSON emitted by the relay's `report_json` / `action_json` /
// `ban_json` (crates/buzz-relay/src/api/bridge.rs) — field names are pinned to
// that source. The queue view consumes these via the shared
// `features/moderation` hooks (Dawn's lane); this module owns only the
// triage math: severity ordering, grouping by target, and prior-actions
// correlation.
//
// Privacy invariant (locked, Tyler 2026-07-07): `reporterPubkey` is visible in
// this admin queue but MUST NEVER reach any surface the reported author can
// see. Nothing here is rendered author-side.

/** NIP-56 report categories accepted at ingest (relay `report.rs::REPORT_TYPES`). */
export type ReportType =
  | "illegal"
  | "nudity"
  | "malware"
  | "spam"
  | "impersonation"
  | "profanity"
  | "other";

/** Discriminant for what a report points at (`report_json.target_kind`). */
export type ReportTargetKind = "event" | "pubkey" | "blob";

/**
 * Report lifecycle status (DB CHECK on `moderation_reports.status`). `open` is
 * the default and the only actionable state; `escalated` routes out of
 * community discretion into the platform-safety lane.
 */
export type ReportStatus = "open" | "resolved" | "dismissed" | "escalated";

/** Queue row: one accepted kind:1984 report (`/moderation/reports`). */
export type ModerationReport = {
  id: string;
  reportEventId: string;
  /** Reporter identity — mod-only, never author-visible. */
  reporterPubkey: string;
  targetKind: ReportTargetKind;
  /** Hex event id / pubkey / blob sha, per `targetKind`. */
  target: string;
  channelId: string | null;
  reportType: ReportType;
  /** Reporter-supplied context; mod-only. */
  note: string | null;
  status: ReportStatus;
  resolvedBy: string | null;
  resolvedAt: string | null;
  actionId: string | null;
  createdAt: string;
};

/** Audit row: one accepted moderation action (`/moderation/audit`). */
export type ModerationAction = {
  id: string;
  actorPubkey: string;
  action: string;
  targetPubkey: string | null;
  targetEventId: string | null;
  channelId: string | null;
  reasonCode: string | null;
  /** Sanitized, tombstone-safe. */
  publicReason: string | null;
  /** Mod-only; never leaves the audit surface. */
  privateReason: string | null;
  matchedPrincipal: string | null;
  createdAt: string;
};

/**
 * Severity rank per report category — higher acts first. `illegal` tops the
 * queue because it routes to the platform-safety escalation lane, not
 * community discretion (Eva's two-layer model). The rest descend by typical
 * community harm. `other` sinks to the bottom as the catch-all.
 */
const SEVERITY_RANK: Record<ReportType, number> = {
  illegal: 6,
  malware: 5,
  impersonation: 4,
  nudity: 3,
  spam: 2,
  profanity: 1,
  other: 0,
};

export function reportSeverity(reportType: ReportType): number {
  return SEVERITY_RANK[reportType] ?? SEVERITY_RANK.other;
}

/**
 * Stable identity for the *thing* a report targets, so multiple reports about
 * the same message/user/blob collapse into one queue group. Kind-qualified to
 * keep an event id and a (hypothetical) identical pubkey hex from colliding.
 */
export function targetKey(report: ModerationReport): string {
  return `${report.targetKind}:${report.target}`;
}

export type ModerationQueueGroup = {
  targetKey: string;
  targetKind: ReportTargetKind;
  target: string;
  /** Reports about this target, newest first. */
  reports: ModerationReport[];
  /** Highest severity among the group's reports — drives group ordering. */
  maxSeverity: number;
  /** Most recent report timestamp in the group (ISO), for tie-breaks. */
  latestCreatedAt: string;
  /** Prior accepted actions already taken against this target (newest first). */
  priorActions: ModerationAction[];
};

/** Newest-first ISO timestamp comparator (descending). */
function byCreatedAtDesc(
  a: { createdAt: string },
  b: { createdAt: string },
): number {
  return b.createdAt.localeCompare(a.createdAt);
}

/**
 * Does an audit row concern the same target as a queue group? Reports point at
 * events, pubkeys, or blobs; audit rows carry `targetPubkey` / `targetEventId`
 * (blobs are not separately keyed in the audit shape, so blob groups surface no
 * prior-actions correlation — by design, not omission).
 */
function actionMatchesTarget(
  action: ModerationAction,
  targetKind: ReportTargetKind,
  target: string,
): boolean {
  if (targetKind === "event") return action.targetEventId === target;
  if (targetKind === "pubkey") return action.targetPubkey === target;
  return false;
}

/**
 * Build the triaged queue: reports grouped by target, each group carrying its
 * max severity, prior actions, and reports newest-first; groups sorted by
 * severity desc, then most-recent-report desc. `actions` is the audit log used
 * to attach prior-actions context (pass `[]` when unavailable).
 */
export function buildModerationQueue(
  reports: readonly ModerationReport[],
  actions: readonly ModerationAction[] = [],
): ModerationQueueGroup[] {
  const groups = new Map<string, ModerationQueueGroup>();

  for (const report of reports) {
    const key = targetKey(report);
    const existing = groups.get(key);
    if (existing) {
      existing.reports.push(report);
      existing.maxSeverity = Math.max(
        existing.maxSeverity,
        reportSeverity(report.reportType),
      );
    } else {
      groups.set(key, {
        targetKey: key,
        targetKind: report.targetKind,
        target: report.target,
        reports: [report],
        maxSeverity: reportSeverity(report.reportType),
        latestCreatedAt: report.createdAt,
        priorActions: [],
      });
    }
  }

  for (const group of groups.values()) {
    group.reports.sort(byCreatedAtDesc);
    group.latestCreatedAt =
      group.reports[0]?.createdAt ?? group.latestCreatedAt;
    group.priorActions = actions
      .filter((a) => actionMatchesTarget(a, group.targetKind, group.target))
      .sort(byCreatedAtDesc);
  }

  return [...groups.values()].sort((a, b) => {
    if (b.maxSeverity !== a.maxSeverity) return b.maxSeverity - a.maxSeverity;
    return b.latestCreatedAt.localeCompare(a.latestCreatedAt);
  });
}

/** Reports still awaiting a decision (`status === "open"`). */
export function isOpenReport(report: ModerationReport): boolean {
  return report.status === "open";
}
