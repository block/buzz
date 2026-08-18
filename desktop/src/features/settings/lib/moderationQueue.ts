// Domain logic for the community-moderation admin queue (U2 admin surface).
//
// Pure, hook-free transforms over the NIP-98 `/moderation/*` read contract so
// they can be unit-tested without a relay. The authoritative wire row shapes
// live in `@/shared/api/moderation` (Dawn's lane); this module owns only the
// triage math: severity ordering, grouping by target, and prior-actions
// correlation. It reuses those row types directly, narrowing just the two
// fields the triage math dispatches on (`reportType`, `status`) to the precise
// unions below — the shared types keep them as `string` so the wire can carry
// values the client doesn't yet model.
//
// Privacy invariant (locked, Tyler 2026-07-07): `reporterPubkey` is visible in
// this admin queue but MUST NEVER reach any surface the reported author can
// see. Nothing here is rendered author-side.

import type {
  ModerationAction as ApiModerationAction,
  ModerationReport as ApiModerationReport,
  ResolutionAction,
} from "@/shared/api/moderation";

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

/** Queue row: one accepted kind:1984 report (`/moderation/reports`).
 *
 * The shared `ApiModerationReport` shape verbatim, with `reportType` and
 * `status` narrowed to the client-modeled unions the triage math dispatches
 * on. `targetKind` is already the exact union upstream, so it passes through.
 */
export type ModerationReport = Omit<
  ApiModerationReport,
  "reportType" | "status"
> & {
  reportType: ReportType;
  status: ReportStatus;
};

/** Audit row: one accepted moderation action (`/moderation/audit`). The shared
 * shape needs no narrowing here — the triage math treats `action` opaquely. */
export type ModerationAction = ApiModerationAction;

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
  /**
   * Channel the target lives in, if any. An event target lives in exactly one
   * channel (all reports about it agree), so we take it from the first report;
   * pubkey/blob targets are not channel-scoped and carry `null`. Drives which
   * channel-scoped enforcements (delete/kick) are offerable.
   */
  channelId: string | null;
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
        channelId: report.channelId,
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

/** Human label for a NIP-56 report category. */
export function reportTypeLabel(reportType: ReportType): string {
  switch (reportType) {
    case "illegal":
      return "Illegal content";
    case "nudity":
      return "Nudity";
    case "malware":
      return "Malware";
    case "spam":
      return "Spam";
    case "impersonation":
      return "Impersonation";
    case "profanity":
      return "Profanity";
    case "other":
      return "Other";
  }
}

/**
 * Coarse severity tier for badge styling. `illegal` is `critical` (escalation
 * lane); malware/impersonation are `high`; the rest are `normal`. Kept separate
 * from the numeric `reportSeverity` rank so the visual tiers can be tuned
 * without perturbing sort order.
 */
export type SeverityTier = "critical" | "high" | "normal";

export function severityTier(reportType: ReportType): SeverityTier {
  if (reportType === "illegal") return "critical";
  if (reportType === "malware" || reportType === "impersonation") return "high";
  return "normal";
}

/** The most severe report type in a group (drives the group's badge). */
export function groupTopReportType(group: ModerationQueueGroup): ReportType {
  let top = group.reports[0]?.reportType ?? "other";
  for (const report of group.reports) {
    if (reportSeverity(report.reportType) > reportSeverity(top)) {
      top = report.reportType;
    }
  }
  return top;
}

/**
 * Which one-click resolutions can actually be *enforced* for a given target.
 *
 * A 9044 resolve only records the decision + DMs the reporter; the client must
 * compose the paired enforcement event (delete→9005, ban→9040, kick→9001).
 * Some pairings are structurally impossible, so we never offer them as buttons
 * (Eva's ruling: an action you can't complete shouldn't be clickable):
 *
 * - `delete` (9005) needs an event id + channel — only event-target reports.
 * - `kick` (9001) is channel-scoped — needs both an author and a channel, so
 *   only event-target reports (a pubkey report is not tied to a channel).
 * - `ban` (9040) needs only the author pubkey — event reports resolve it from
 *   the reported event's signer; pubkey reports carry it as the target.
 * - `escalate` / `dismiss` are decision-only and always available.
 *
 * `timeout` is intentionally excluded until the resolve flow can collect a
 * duration (a duration-less timeout would be a lie); it wires back in with the
 * duration picker as a follow-up.
 */
export function resolvableActions(
  targetKind: ReportTargetKind,
  hasChannel: boolean,
): ResolutionAction[] {
  const actions: ResolutionAction[] = [];
  if (targetKind === "event" && hasChannel) actions.push("delete");
  // ban needs only the author; event reports look it up from the signer.
  if (targetKind === "event" || targetKind === "pubkey") actions.push("ban");
  if (targetKind === "event" && hasChannel) actions.push("kick");
  actions.push("escalate", "dismiss");
  return actions;
}
