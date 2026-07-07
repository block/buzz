import { relayClient } from "@/shared/api/relayClient";
import { getRelayHttpUrl, signRelayEvent } from "@/shared/api/tauri";
import {
  KIND_MODERATION_BAN,
  KIND_MODERATION_RESOLVE_REPORT,
  KIND_MODERATION_TIMEOUT,
  KIND_MODERATION_UNBAN,
  KIND_MODERATION_UNTIMEOUT,
  KIND_REPORT,
} from "@/shared/constants/kinds";

// Community-moderation data layer. Writes are signed Nostr events published over
// the same WebSocket path as every other desktop write (mirrors relayMembers.ts);
// reads are NIP-98-authed HTTP GETs to /moderation/*, which have no WS equivalent.
//
// Wire contract is pinned by the relay/CLI (buzz-sdk builders, report.rs,
// api/bridge.rs). Command events (9040–9044) carry NO `h` tag — the relay binds
// the tenant from the connection host, and a stray `h` is rejected as
// channel-scoping a global-only command.

const NIP98_KIND = 27235;

/** NIP-56 report categories (report.rs `REPORT_TYPES`). */
export type ReportType =
  | "illegal"
  | "nudity"
  | "malware"
  | "spam"
  | "impersonation"
  | "profanity"
  | "other";

/** A moderator's disposition of a queued report (buzz-sdk resolve builder). */
export type ResolutionStatus = "resolved" | "dismissed";
export type ResolutionAction =
  | "delete"
  | "kick"
  | "ban"
  | "timeout"
  | "dismiss"
  | "escalate";

// --- Read row shapes (api/bridge.rs report_json / action_json / ban_json) ---

export type ModerationReport = {
  id: number;
  reportEventId: string;
  reporterPubkey: string;
  targetKind: "event" | "pubkey" | "blob";
  target: string;
  channelId: string | null;
  reportType: string;
  note: string | null;
  status: string;
  resolvedBy: string | null;
  resolvedAt: number | null;
  actionId: string | null;
  createdAt: number;
};

export type ModerationAction = {
  id: number;
  actorPubkey: string;
  action: string;
  targetPubkey: string | null;
  targetEventId: string | null;
  channelId: string | null;
  reasonCode: string | null;
  publicReason: string | null;
  privateReason: string | null;
  matchedPrincipal: string | null;
  createdAt: number;
};

export type CommunityRestriction = {
  pubkey: string;
  banned: boolean;
  banExpiresAt: number | null;
  banReason: string | null;
  mutedUntil: number | null;
  muteReason: string | null;
  actorPubkey: string;
  updatedAt: number;
};

function normalizePubkey(pubkey: string): string {
  return pubkey.trim().toLowerCase();
}

// --- Writes: signed events over the WS publish path ---

async function publishModerationEvent(
  kind: number,
  tags: string[][],
  timeoutMessage: string,
  errorMessage: string,
): Promise<void> {
  const event = await signRelayEvent({ kind, content: "", tags });
  await relayClient.publishEvent(event, timeoutMessage, errorMessage);
}

/**
 * Submit a NIP-56 report (kind:1984). Member-facing report entry always targets
 * a message, so both the author `p` tag and the `e` (event) tag are carried; the
 * report type rides the `e` tag's third element per report.rs `parse_report`.
 */
export async function submitReport(input: {
  authorPubkey: string;
  eventId: string;
  reportType: ReportType;
  note?: string;
}): Promise<void> {
  const tags: string[][] = [
    ["p", normalizePubkey(input.authorPubkey)],
    ["e", input.eventId, input.reportType],
  ];
  const event = await signRelayEvent({
    kind: KIND_REPORT,
    content: input.note?.trim() ? input.note.trim() : "",
    tags,
  });
  await relayClient.publishEvent(
    event,
    "Timed out while submitting the report.",
    "Failed to submit the report.",
  );
}

/** Ban a member (kind:9040). `expiresAt` unix-secs ⇒ temporary; omit ⇒ permanent. */
export async function banMember(input: {
  pubkey: string;
  expiresAt?: number;
  reason?: string;
}): Promise<void> {
  const tags: string[][] = [["p", normalizePubkey(input.pubkey)]];
  if (input.expiresAt != null)
    tags.push(["expiration", String(input.expiresAt)]);
  if (input.reason?.trim()) tags.push(["reason", input.reason.trim()]);
  await publishModerationEvent(
    KIND_MODERATION_BAN,
    tags,
    "Timed out while banning the member.",
    "Failed to ban the member.",
  );
}

/** Lift a ban (kind:9041). */
export async function unbanMember(pubkey: string): Promise<void> {
  await publishModerationEvent(
    KIND_MODERATION_UNBAN,
    [["p", normalizePubkey(pubkey)]],
    "Timed out while lifting the ban.",
    "Failed to lift the ban.",
  );
}

/** Time out a member (kind:9042). `expiresAt` unix-secs is required by the relay. */
export async function timeoutMember(input: {
  pubkey: string;
  expiresAt: number;
  reason?: string;
}): Promise<void> {
  const tags: string[][] = [
    ["p", normalizePubkey(input.pubkey)],
    ["expiration", String(input.expiresAt)],
  ];
  if (input.reason?.trim()) tags.push(["reason", input.reason.trim()]);
  await publishModerationEvent(
    KIND_MODERATION_TIMEOUT,
    tags,
    "Timed out while applying the timeout.",
    "Failed to apply the timeout.",
  );
}

/** Lift a timeout (kind:9043). */
export async function untimeoutMember(pubkey: string): Promise<void> {
  await publishModerationEvent(
    KIND_MODERATION_UNTIMEOUT,
    [["p", normalizePubkey(pubkey)]],
    "Timed out while lifting the timeout.",
    "Failed to lift the timeout.",
  );
}

/**
 * Resolve a queued report (kind:9044). `dismiss` pairs with `dismissed`; every
 * other action pairs with `resolved` (the relay enforces the pairing). `reason`
 * is moderator-authored and reporter-readable — it lands in the public tombstone
 * and the reporter-notice DM.
 */
export async function resolveReport(input: {
  reportEventId: string;
  status: ResolutionStatus;
  action: ResolutionAction;
  reason?: string;
}): Promise<void> {
  const tags: string[][] = [
    ["report", normalizePubkey(input.reportEventId)],
    ["status", input.status],
    ["action", input.action],
  ];
  if (input.reason?.trim()) tags.push(["reason", input.reason.trim()]);
  await publishModerationEvent(
    KIND_MODERATION_RESOLVE_REPORT,
    tags,
    "Timed out while resolving the report.",
    "Failed to resolve the report.",
  );
}

// --- Reads: NIP-98-authed HTTP GETs ---

/**
 * Build the NIP-98 `Authorization` header for a GET.
 *
 * The relay verifies the signed `u` tag against the full request URL including
 * the query string (the read-auth fix), so the URL is finalized by the caller
 * *before* signing and this function never appends parameters afterward — the
 * signed `u` and the fetched URL are guaranteed identical.
 */
async function nip98GetHeader(url: string): Promise<string> {
  const authEvent = await signRelayEvent({
    kind: NIP98_KIND,
    content: "",
    tags: [
      ["u", url],
      ["method", "GET"],
      ["nonce", crypto.randomUUID()],
    ],
  });
  // NIP-98 events carry empty content and ASCII-only tags, so btoa is safe here.
  return `Nostr ${btoa(JSON.stringify(authEvent))}`;
}

async function moderationGet<T>(pathWithQuery: string): Promise<T> {
  const base = (await getRelayHttpUrl()).replace(/\/+$/, "");
  const url = `${base}${pathWithQuery}`;
  const authorization = await nip98GetHeader(url);
  const response = await fetch(url, {
    headers: { Authorization: authorization },
  });
  if (!response.ok) {
    throw new Error(
      `Moderation request failed (${response.status}): ${pathWithQuery}`,
    );
  }
  return (await response.json()) as T;
}

type RawReport = {
  id: number;
  report_event_id: string;
  reporter_pubkey: string;
  target_kind: "event" | "pubkey" | "blob";
  target: string;
  channel_id: string | null;
  report_type: string;
  note: string | null;
  status: string;
  resolved_by: string | null;
  resolved_at: number | null;
  action_id: string | null;
  created_at: number;
};

type RawAction = {
  id: number;
  actor_pubkey: string;
  action: string;
  target_pubkey: string | null;
  target_event_id: string | null;
  channel_id: string | null;
  reason_code: string | null;
  public_reason: string | null;
  private_reason: string | null;
  matched_principal: string | null;
  created_at: number;
};

type RawRestriction = {
  pubkey: string;
  banned: boolean;
  ban_expires_at: number | null;
  ban_reason: string | null;
  muted_until: number | null;
  mute_reason: string | null;
  actor_pubkey: string;
  updated_at: number;
};

function toReport(r: RawReport): ModerationReport {
  return {
    id: r.id,
    reportEventId: r.report_event_id,
    reporterPubkey: r.reporter_pubkey,
    targetKind: r.target_kind,
    target: r.target,
    channelId: r.channel_id,
    reportType: r.report_type,
    note: r.note,
    status: r.status,
    resolvedBy: r.resolved_by,
    resolvedAt: r.resolved_at,
    actionId: r.action_id,
    createdAt: r.created_at,
  };
}

function toAction(a: RawAction): ModerationAction {
  return {
    id: a.id,
    actorPubkey: a.actor_pubkey,
    action: a.action,
    targetPubkey: a.target_pubkey,
    targetEventId: a.target_event_id,
    channelId: a.channel_id,
    reasonCode: a.reason_code,
    publicReason: a.public_reason,
    privateReason: a.private_reason,
    matchedPrincipal: a.matched_principal,
    createdAt: a.created_at,
  };
}

function toRestriction(b: RawRestriction): CommunityRestriction {
  return {
    pubkey: b.pubkey,
    banned: b.banned,
    banExpiresAt: b.ban_expires_at,
    banReason: b.ban_reason,
    mutedUntil: b.muted_until,
    muteReason: b.mute_reason,
    actorPubkey: b.actor_pubkey,
    updatedAt: b.updated_at,
  };
}

/**
 * Fetch the moderation queue (`GET /moderation/reports`). Mod-authz gated —
 * ordinary members receive 403. `status` filters (e.g. "open"); `limit` is
 * clamped relay-side.
 */
export async function listReports(options?: {
  status?: string;
  limit?: number;
}): Promise<ModerationReport[]> {
  const params = new URLSearchParams();
  if (options?.limit != null) params.set("limit", String(options.limit));
  if (options?.status) params.set("status", options.status);
  const query = params.toString();
  const rows = await moderationGet<RawReport[]>(
    query ? `/moderation/reports?${query}` : "/moderation/reports",
  );
  return rows.map(toReport);
}

/** Fetch the audit log (`GET /moderation/audit`), newest-first. Mod-authz gated. */
export async function listAuditActions(
  limit?: number,
): Promise<ModerationAction[]> {
  const query = limit != null ? `?limit=${limit}` : "";
  const rows = await moderationGet<RawAction[]>(`/moderation/audit${query}`);
  return rows.map(toAction);
}

/**
 * Fetch active bans/timeouts (`GET /moderation/restricted`). Mod-authz gated —
 * this is the moderator's view of who is currently restricted, not a member's
 * self-state lookup.
 */
export async function listRestrictions(): Promise<CommunityRestriction[]> {
  const rows = await moderationGet<RawRestriction[]>("/moderation/restricted");
  return rows.map(toRestriction);
}
