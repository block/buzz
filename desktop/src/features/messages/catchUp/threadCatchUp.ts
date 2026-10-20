import { READ_STATE_HORIZON_SECONDS } from "@/features/channels/readState/readStateFormat";
import { normalizeRepoAddress } from "@/features/projects/fetchProjectTask";
import type { ProjectIssue } from "@/features/projects/projectIssues.mjs";
import type {
  CatchUpPoint,
  CatchUpRecap,
  CatchUpSettings,
} from "@/shared/api/tauriThreadCatchUp";
import { parseEntityLink } from "@/shared/lib/entityLink";

/**
 * Pure rules for the thread recap: which period it covers, how its coverage
 * is described, which cited sources still match the thread, and which task
 * links in its sources are verifiably assigned to the reader.
 */

export const CATCH_UP_LOOKBACK_SECONDS = {
  day: 24 * 60 * 60,
  week: 7 * 24 * 60 * 60,
} as const;

export type CatchUpLookback = keyof typeof CATCH_UP_LOOKBACK_SECONDS;

/**
 * The reader's own read markers frozen before this visit's mark-read effects.
 * Values are unix seconds or null when no marker existed. The `*Ready` flags
 * record whether read state had finished loading when each value was taken.
 */
export type CatchUpBaselineSnapshot = {
  channelId: string;
  rootEventId: string;
  channelReadAt: number | null;
  channelReady: boolean;
  threadReadAt: number | null;
  threadReady: boolean;
  /** Own `msg:<id>` marker of each reply, taken the first time it was seen. */
  replyReadAt: Map<string, number | null>;
  repliesReady: boolean;
};

export type CatchUpBaseline =
  | {
      kind: "marker";
      since: number;
      previousReadAt: number;
      unreadCount: number;
    }
  | { kind: "caught-up"; previousReadAt: number }
  | { kind: "expired"; previousReadAt: number }
  | { kind: "absent" }
  | { kind: "unresolved" };

type BaselineReply = { id: string; createdAt: number; pubkey?: string };

function maxMarker(values: Array<number | null | undefined>): number | null {
  let max: number | null = null;
  for (const value of values) {
    if (typeof value === "number" && (max === null || value > max)) {
      max = value;
    }
  }
  return max;
}

/**
 * Resolve the recap's lower bound from a frozen snapshot. A reply was unread
 * before this visit when it is newer than both the thread-level frontier and
 * its own marker. Unready, absent, or expired markers are never presented as
 * a previous read position; the caller offers an explicit lookback instead.
 */
export function resolveCatchUpBaseline({
  snapshot,
  replies,
  currentPubkey,
  nowSeconds,
  horizonSeconds = READ_STATE_HORIZON_SECONDS,
}: {
  snapshot: CatchUpBaselineSnapshot | null;
  replies: readonly BaselineReply[];
  currentPubkey?: string;
  nowSeconds: number;
  horizonSeconds?: number;
}): CatchUpBaseline {
  if (
    !snapshot?.channelReady ||
    !snapshot.threadReady ||
    !snapshot.repliesReady
  ) {
    return { kind: "unresolved" };
  }
  const frontier = maxMarker([snapshot.channelReadAt, snapshot.threadReadAt]);
  const previousReadAt = maxMarker([
    frontier,
    ...snapshot.replyReadAt.values(),
  ]);
  if (previousReadAt === null) return { kind: "absent" };
  if (previousReadAt < nowSeconds - horizonSeconds) {
    return { kind: "expired", previousReadAt };
  }
  const self = currentPubkey?.toLowerCase();
  let earliestUnread: number | null = null;
  let unreadCount = 0;
  for (const reply of replies) {
    if (reply.id === snapshot.rootEventId) continue;
    if (self && reply.pubkey?.toLowerCase() === self) continue;
    // A reply first observed after the snapshot has no frozen own marker; its
    // pre-visit state is the thread frontier alone.
    const readAt = maxMarker([frontier, snapshot.replyReadAt.get(reply.id)]);
    if (readAt !== null && reply.createdAt <= readAt) continue;
    unreadCount += 1;
    if (earliestUnread === null || reply.createdAt < earliestUnread) {
      earliestUnread = reply.createdAt;
    }
  }
  if (earliestUnread === null) return { kind: "caught-up", previousReadAt };
  return {
    kind: "marker",
    // Native's lower bound is inclusive.
    since: earliestUnread,
    previousReadAt,
    unreadCount,
  };
}

/** How a recap's period was chosen, kept with the recap for its label. */
export type CatchUpRangeChoice =
  | { kind: "baseline" }
  | { kind: "lookback"; lookback: CatchUpLookback };

export function resolveCatchUpRange(
  choice: CatchUpRangeChoice,
  baseline: CatchUpBaseline,
  nowSeconds: number,
): { since: number; until: number } | null {
  if (choice.kind === "lookback") {
    return {
      since: nowSeconds - CATCH_UP_LOOKBACK_SECONDS[choice.lookback],
      until: nowSeconds,
    };
  }
  if (baseline.kind !== "marker") return null;
  return { since: baseline.since, until: nowSeconds };
}

const RANGE_FORMAT = new Intl.DateTimeFormat(undefined, {
  month: "short",
  day: "numeric",
  hour: "numeric",
  minute: "2-digit",
});

export function formatCatchUpTime(seconds: number): string {
  return RANGE_FORMAT.format(new Date(seconds * 1_000));
}

export function describeCatchUpRange(
  choice: CatchUpRangeChoice,
  recap: Pick<CatchUpRecap, "since" | "until">,
): string {
  const window = `${formatCatchUpTime(recap.since)} – ${formatCatchUpTime(recap.until)}`;
  if (choice.kind === "baseline") {
    return `Since your previous read position · ${window}`;
  }
  return `${choice.lookback === "day" ? "Last 24 hours" : "Last 7 days"} · ${window}`;
}

/** Plain-language coverage line; never implies an exhaustive history. */
export function describeCatchUpCoverage(
  coverage: CatchUpRecap["coverage"],
): string {
  const { inputMessageCount, messageCount } = coverage;
  const noun = messageCount === 1 ? "message" : "messages";
  if (coverage.truncated || inputMessageCount < messageCount) {
    return `Partial: read ${inputMessageCount} of ${messageCount} retrieved ${noun}`;
  }
  return `Read ${inputMessageCount} retrieved ${noun}`;
}

export const CATCH_UP_SECTION_LABELS: Record<CatchUpPoint["category"], string> =
  {
    decision: "Decisions",
    question: "Still open",
  };

export function groupCatchUpPoints<T extends { category: string }>(
  points: readonly T[],
) {
  return {
    decision: points.filter((point) => point.category === "decision"),
    question: points.filter((point) => point.category === "question"),
  };
}

// --- Local model setup ---

const MAX_MODEL_CHARS = 200;

function isNumericLoopbackHost(hostname: string): boolean {
  if (hostname === "[::1]") return true;
  const octets = hostname.split(".");
  return (
    octets.length === 4 &&
    octets[0] === "127" &&
    octets.every((octet) => /^\d{1,3}$/.test(octet) && Number(octet) <= 255)
  );
}

/**
 * Client-side check of the explicit local endpoint. Native enforces the same
 * rule on the real transport; this only gives the person an early, specific
 * message. Host names such as `localhost` are refused because they can
 * resolve elsewhere.
 */
export function validateCatchUpSettings(
  settings: CatchUpSettings,
): string | null {
  let url: URL;
  try {
    url = new URL(settings.baseUrl.trim());
  } catch {
    return "Enter the full endpoint address, like http://127.0.0.1:11434/v1.";
  }
  if (url.protocol !== "http:") {
    return "The endpoint must start with http:// (local models do not use TLS).";
  }
  if (url.username || url.password || url.search || url.hash) {
    return "Remove credentials, query strings, or fragments from the endpoint.";
  }
  if (!isNumericLoopbackHost(url.hostname)) {
    return "Use a numeric loopback address (127.0.0.1 or [::1]) so the recap stays on this computer.";
  }
  const model = settings.model.trim();
  if (model.length === 0) return "Enter the name of the local model to use.";
  if (model.length > MAX_MODEL_CHARS) return "That model name is too long.";
  if (!settings.confirmedLocal) {
    return "Confirm that this endpoint runs the model on this computer.";
  }
  return null;
}

export function isCatchUpConfigured(
  settings: CatchUpSettings | undefined,
): settings is CatchUpSettings {
  return settings !== undefined && validateCatchUpSettings(settings) === null;
}

// --- Corrections ---

export const MAX_CATCH_UP_CORRECTION_BYTES = 2_000;
// Control characters other than tab and newline, which native refuses.
// biome-ignore lint/suspicious/noControlCharactersInRegex: matching them is the point
const DISALLOWED_CONTROL_RE = /[\u0000-\u0008\u000b-\u001f\u007f]/;
const textEncoder = new TextEncoder();

export function catchUpCorrectionBytes(text: string): number {
  return textEncoder.encode(text.trim()).length;
}

/** Mirrors native correction rules so a draft is checked before signing. */
export function validateCatchUpCorrection(
  text: string,
  originalText: string,
): string | null {
  const trimmed = text.trim();
  if (trimmed.length === 0) return "Write the corrected point first.";
  if (trimmed === originalText.trim()) {
    return "The correction is the same as the current text.";
  }
  if (catchUpCorrectionBytes(trimmed) > MAX_CATCH_UP_CORRECTION_BYTES) {
    return "Corrections are limited to 2,000 bytes.";
  }
  if (DISALLOWED_CONTROL_RE.test(trimmed)) {
    return "Remove control characters from the correction.";
  }
  return null;
}

/** Native reports a lost `prev` race with a stable `conflict:` prefix. */
export function isCatchUpConflictError(error: unknown): boolean {
  return error instanceof Error && error.message.startsWith("conflict:");
}

/**
 * Native refuses a share with a `source_changed` prefix when a cited source no
 * longer matches the revision recorded at generation. The recap stays as is.
 */
export function isCatchUpSourceChangedError(error: unknown): boolean {
  return error instanceof Error && /^source_changed\b/.test(error.message);
}

// --- Sources ---

export type CatchUpSourceState = "current" | "changed" | "unloaded";

/**
 * Compare a cited source against the thread as currently loaded. `snapshot`
 * holds the cited bodies as they were when this client generated the recap;
 * shared recaps from elsewhere have none, so only absence is detectable.
 */
export function resolveCatchUpSourceState(
  sourceId: string,
  currentBodies: ReadonlyMap<string, string>,
  snapshot: ReadonlyMap<string, string> | null,
): CatchUpSourceState {
  const body = currentBodies.get(sourceId);
  if (body === undefined) return "unloaded";
  const frozen = snapshot?.get(sourceId);
  if (frozen !== undefined && frozen !== body) return "changed";
  return "current";
}

export function snapshotCitedBodies(
  recap: CatchUpRecap,
  currentBodies: ReadonlyMap<string, string>,
): Map<string, string> {
  const snapshot = new Map<string, string>();
  for (const point of recap.points) {
    for (const id of point.sourceIds) {
      const body = currentBodies.get(id);
      if (body !== undefined) snapshot.set(id, body);
    }
  }
  return snapshot;
}

// --- Assigned to you ---

export type CatchUpTaskReference = {
  issueId: string;
  owner: string;
  dtag: string;
  /** NIP-34 repository coordinate the link names: `30617:<owner>:<d>`. */
  repoAddress: string;
  /** Recap source messages that contain this exact link, in thread order. */
  sourceIds: string[];
};

const ISSUE_LINK_RE = /buzz:\/\/issue\?[^\s<>"'`()[\]{}]+/g;
const TRAILING_PUNCTUATION_RE = /[.,;:!?]+$/;
export const MAX_CATCH_UP_TASK_REFERENCES = 20;

/**
 * Explicit task links in the recap's own sources. Only canonical
 * `buzz://issue` links that pass the strict entity-link parser count; prose,
 * mentions, and origin-channel tags never imply an assignment.
 */
export function extractCatchUpTaskReferences(
  messages: ReadonlyArray<{ id: string; body: string }>,
  allowedSourceIds: ReadonlySet<string>,
): CatchUpTaskReference[] {
  const byIssue = new Map<string, CatchUpTaskReference>();
  for (const message of messages) {
    if (!allowedSourceIds.has(message.id)) continue;
    for (const match of message.body.matchAll(ISSUE_LINK_RE)) {
      const parsed = parseEntityLink(
        match[0].replace(TRAILING_PUNCTUATION_RE, ""),
      );
      if (!parsed.ok || parsed.value.type !== "issue") continue;
      const { id, owner, dtag } = parsed.value;
      const existing = byIssue.get(id);
      if (existing) {
        if (!existing.sourceIds.includes(message.id)) {
          existing.sourceIds.push(message.id);
        }
        continue;
      }
      if (byIssue.size >= MAX_CATCH_UP_TASK_REFERENCES) continue;
      byIssue.set(id, {
        issueId: id,
        owner,
        dtag,
        repoAddress: `30617:${owner}:${dtag}`,
        sourceIds: [message.id],
      });
    }
  }
  return [...byIssue.values()];
}

export type CatchUpAssignedTask = {
  reference: CatchUpTaskReference;
  issue: ProjectIssue;
};

const FINISHED_TASK_STATUSES = new Set(["Done", "Closed"]);

/**
 * Keep only tasks whose reduced assignment state names the reader, whose own
 * repository coordinate matches the link, and whose repository belongs to a
 * project the reader can see. Finished tasks are not outstanding work.
 */
export function selectCatchUpAssignedTasks({
  references,
  tasks,
  eligibleRepoAddresses,
  currentPubkey,
}: {
  references: readonly CatchUpTaskReference[];
  tasks: ReadonlyMap<string, ProjectIssue | null>;
  eligibleRepoAddresses: ReadonlySet<string>;
  currentPubkey: string;
}): CatchUpAssignedTask[] {
  const self = currentPubkey.toLowerCase();
  const assigned: CatchUpAssignedTask[] = [];
  for (const reference of references) {
    const issue = tasks.get(reference.issueId);
    if (!issue || issue.id.toLowerCase() !== reference.issueId) continue;
    if (normalizeRepoAddress(issue.repoAddress) !== reference.repoAddress) {
      continue;
    }
    if (!eligibleRepoAddresses.has(reference.repoAddress)) continue;
    if (FINISHED_TASK_STATUSES.has(issue.status)) continue;
    if (!issue.assignees.some((pubkey) => pubkey.toLowerCase() === self)) {
      continue;
    }
    assigned.push({ reference, issue });
  }
  return assigned;
}

/** Canonical repository coordinates from the reader's visible projects. */
export function eligibleCatchUpRepoAddresses(
  projects: ReadonlyArray<{
    repositories: ReadonlyArray<{ repoAddress: string }>;
  }>,
): Set<string> {
  const addresses = new Set<string>();
  for (const project of projects) {
    for (const repository of project.repositories) {
      const address = normalizeRepoAddress(repository.repoAddress);
      if (address) addresses.add(address);
    }
  }
  return addresses;
}
