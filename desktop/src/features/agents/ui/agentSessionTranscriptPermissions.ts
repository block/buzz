/**
 * Pure helper functions and draft-mutating permission handlers extracted from
 * agentSessionTranscript.ts to keep that file under the line-count ratchet.
 *
 * Consumers: agentSessionTranscript.ts only. Do not import from elsewhere.
 */
import { asRecord, asString } from "./agentSessionUtils";
import type { TranscriptItem } from "./agentSessionTypes";

// ---------------------------------------------------------------------------
// Minimal draft slice — structural subset of TranscriptDraft that permission
// helpers operate on. TranscriptDraft satisfies this interface via TypeScript
// structural typing; no import from the main transcript file is required.
// ---------------------------------------------------------------------------
export type PermissionDraftSlice = {
  items: TranscriptItem[];
  itemsById: Map<string, TranscriptItem>;
  pendingPermissions: Map<
    string,
    { itemId: string; optionNames: Map<string, string> }
  >;
  pendingPermissionsByNonce: Map<string, string>;
  changed: boolean;
};

/** Replica of TranscriptItemContext — duplicated to avoid a circular import. */
type PermCtx = {
  sessionId: string | null;
  turnId: string | null;
};

/**
 * Inline replica of jsonRpcId — duplicated to avoid a circular import.
 * Converts a JSON-RPC id value to a stable string key, or null for
 * non-id types (null, undefined, object, boolean).
 */
function jsonRpcIdLocal(value: unknown): string | null {
  if (typeof value === "string") return JSON.stringify(value);
  if (typeof value === "number" && Number.isFinite(value))
    return JSON.stringify(value);
  return null;
}

/**
 * Mutate a draft in place, replacing the item at `id`. Copies items/itemsById
 * on the first mutation (copy-on-write semantics mirror the main draft helpers).
 */
function setPermissionItem(
  d: PermissionDraftSlice,
  id: string,
  updated: TranscriptItem,
) {
  if (!d.changed) {
    d.items = [...d.items];
    d.itemsById = new Map(d.itemsById);
    d.changed = true;
  }
  const idx = d.items.findIndex((it) => it.id === id);
  if (idx !== -1) d.items[idx] = updated;
  d.itemsById.set(id, updated);
}

// ---------------------------------------------------------------------------
// Pure description helpers
// ---------------------------------------------------------------------------

/**
 * Extract a human-readable title, body text, option name map, structured
 * options list, and activity descriptor from an ACP `session/request_permission`
 * payload.
 */
export function describePermissionRequest(payload: Record<string, unknown>) {
  const params = asRecord(payload.params);
  const title =
    asString(params.title) ??
    asString(params.message) ??
    asString(params.reason) ??
    "Permission requested";
  const toolCallId =
    asString(params.toolCallId) ?? asString(params.tool_call_id);

  // Build both the display-string list and the structured options list in
  // a single pass over params.options.
  const optionNames = new Map<string, string>();
  const structuredOptions: Array<{
    optionId: string;
    kind: string;
    label?: string;
  }> = [];
  const optionDisplayNames: string[] = [];
  if (Array.isArray(params.options)) {
    for (const option of params.options) {
      const rec = asRecord(option);
      const optionId = asString(rec.optionId);
      const kind = asString(rec.kind);
      const label = asString(rec.label) ?? asString(rec.name);
      const displayName =
        asString(rec.name) ?? asString(rec.kind) ?? asString(rec.optionId);
      if (displayName) optionDisplayNames.push(displayName);
      if (optionId && kind) {
        optionNames.set(optionId, kind);
        structuredOptions.push({
          optionId,
          kind,
          ...(label ? { label } : {}),
        });
      }
    }
  }

  const detail: string[] = [];
  if (title !== "Permission requested") detail.push(title);
  if (toolCallId) detail.push(`Tool call: ${toolCallId}`);
  if (optionDisplayNames.length > 0)
    detail.push(`Options: ${optionDisplayNames.join(", ")}`);

  return {
    title,
    text: detail.join("\n"),
    optionNames,
    options: structuredOptions,
    descriptor: {
      renderClass: "permission" as const,
      label: "Permission requested",
      preview: title,
      action: { verb: "Requested", object: title },
      tone: "admin" as const,
      operation: "session/request_permission",
      object: title,
      source: "acp" as const,
      groupKey: "permission:request",
    },
  };
}

/**
 * Format a human-readable outcome label from a permission response.
 * kind values from ACP: allow_once, allow_always, reject_once, reject_always.
 * "reject_*" kinds are denials; anything else that is selected is an approval.
 *
 * `optionLabels` maps optionId → harness-provided display label (e.g. "Allow once").
 * `optionKinds`  maps optionId → ACP kind (e.g. "allow_once"), used only to
 * determine the deny/approve verb when no label is available. The raw kind
 * string is NEVER rendered to the user.
 */
export function describePermissionOutcome(
  outcome: string,
  optionId: string | null,
  optionLabels: Map<string, string>,
  optionKinds?: Map<string, string>,
): string {
  if (outcome === "cancelled") {
    return "Cancelled";
  }
  if (outcome === "timed_out") {
    return "Timed out";
  }
  if (outcome === "uncertain") {
    // Pinned verbatim copy — must never say "denied" or "failed closed".
    return "Approval outcome unknown; agent process stopped before it could continue.";
  }
  if (outcome === "selected" && optionId) {
    const label = optionLabels.get(optionId);
    const kind = optionKinds?.get(optionId) ?? optionId;
    const isDenial = kind.startsWith("reject");
    const verb = isDenial ? "Denied" : "Approved";
    // Render the harness-provided label, never the raw ACP kind string.
    return label ?? `${verb}`;
  }
  return outcome;
}

/**
 * Derive human-readable outcome copy from the `authorization.reason` field
 * that accompanies terminal `acp_write` events. This is preferred over
 * deriving copy from the ACP `result.outcome` field directly because the
 * `reason` values are harness-level semantics (applied / timed_out /
 * cancelled) whereas `result.outcome` is adapter-level (selected / reject_once
 * etc.) and does not distinguish timeout from explicit denial.
 *
 * Falls back to `describePermissionOutcome` when `reason` is absent (legacy
 * paths that predate the authorization envelope).
 */
export function describePermissionTerminalReason(
  reason: string | undefined,
  outcomeKind: string | null | undefined,
  optionId: string | null,
  options:
    | Array<{ optionId: string; kind: string; label?: string; name?: string }>
    | undefined,
): string {
  if (reason === "applied") {
    // Build label map (harness-provided display strings) and kind map (for
    // deny/approve verb fallback only). Labels are preferred; raw kind strings
    // are never rendered to the user.
    // `label` is used by sentinel-format options; `name` is used by ACP
    // JSON-RPC options. Fall back to undefined (verb-only) if neither is set.
    const optionLabels = new Map<string, string>(
      (options ?? [])
        .map(
          (o) =>
            [o.optionId, o.label ?? o.name] as [string, string | undefined],
        )
        .filter((entry): entry is [string, string] => entry[1] !== undefined),
    );
    const optionKinds = new Map(
      (options ?? []).map((o) => [o.optionId, o.kind]),
    );
    return describePermissionOutcome(
      outcomeKind ?? "selected",
      optionId,
      optionLabels,
      optionKinds,
    );
  }
  if (reason === "timed_out") return "Timed out";
  if (reason === "cancelled") return "Cancelled";
  if (reason === "uncertain") {
    return "Approval outcome unknown; agent process stopped before it could continue.";
  }
  // No reason: fall back to ACP outcome-level copy.
  const optionLabels = new Map<string, string>(
    (options ?? [])
      .map(
        (o) => [o.optionId, o.label ?? o.name] as [string, string | undefined],
      )
      .filter((entry): entry is [string, string] => entry[1] !== undefined),
  );
  const optionKinds = new Map((options ?? []).map((o) => [o.optionId, o.kind]));
  return describePermissionOutcome(
    outcomeKind ?? "",
    optionId,
    optionLabels,
    optionKinds,
  );
}

// ---------------------------------------------------------------------------
// Draft-mutating permission helpers
// ---------------------------------------------------------------------------

/**
 * Retire all live (actionable) permission cards for a given channel.
 * Called on terminal turn/process events (`turn_error`, `agent_panic`,
 * `turn_completed`) as a backstop so cards do not remain clickable after
 * the turn that owned them has ended.
 */
export function retireAllLivePermissionCards(
  d: PermissionDraftSlice,
  channelId: string,
) {
  const prefix = `permission:${channelId}:`;
  let retired = false;
  for (const [id, item] of d.itemsById) {
    if (
      id.startsWith(prefix) &&
      item.type === "lifecycle" &&
      item.renderClass === "permission" &&
      item.actionable
    ) {
      if (!retired) {
        // Copy on first mutation.
        d.items = [...d.items];
        d.itemsById = new Map(d.itemsById);
        retired = true;
        d.changed = true;
      }
      const updated = { ...item, actionable: false };
      d.itemsById.set(id, updated);
      const idx = d.items.findIndex((i) => i.id === id);
      if (idx !== -1) d.items[idx] = updated;
      // Clean up nonce index if present.
      if (item.requestNonce) {
        d.pendingPermissionsByNonce = new Map(d.pendingPermissionsByNonce);
        d.pendingPermissionsByNonce.delete(item.requestNonce);
      }
    }
  }
  // Clean up all pendingPermissions entries scoped to this channel.
  // Keys use the compound format `ch:session:turn:id` — drop any that start
  // with the channel prefix.
  const chPrefix = `${channelId}:`;
  let permsMutated = false;
  for (const key of d.pendingPermissions.keys()) {
    if (key.startsWith(chPrefix)) {
      if (!permsMutated) {
        d.pendingPermissions = new Map(d.pendingPermissions);
        permsMutated = true;
      }
      d.pendingPermissions.delete(key);
    }
  }
}

/**
 * Retire live permission cards scoped to a specific turn, identified by
 * `turnId`. Cards whose `turnId` matches are retired (actionable → false)
 * and their nonce indexes removed.
 *
 * With concurrent thread-scoped turns, a channel-wide retirement on
 * `turn_completed` / `turn_error` would retire pending cards belonging to
 * still-running sibling threads. This function scopes the backstop to the
 * single terminating turn so siblings remain actionable.
 *
 * Falls back to `retireAllLivePermissionCards` when `turnId` is absent —
 * kept as a backstop for events (e.g. legacy archive frames) that carry no
 * turn identity.
 */
export function retireLivePermissionCardsForTurn(
  d: PermissionDraftSlice,
  channelId: string,
  turnId: string | null | undefined,
): void {
  if (!turnId) {
    // No turn identity available — fall back to channel-wide backstop.
    retireAllLivePermissionCards(d, channelId);
    return;
  }
  const prefix = `permission:${channelId}:`;
  let retired = false;
  for (const [id, item] of d.itemsById) {
    if (
      id.startsWith(prefix) &&
      item.type === "lifecycle" &&
      item.renderClass === "permission" &&
      item.actionable &&
      item.turnId === turnId
    ) {
      if (!retired) {
        d.items = [...d.items];
        d.itemsById = new Map(d.itemsById);
        retired = true;
        d.changed = true;
      }
      const updated = { ...item, actionable: false };
      d.itemsById.set(id, updated);
      const idx = d.items.findIndex((i) => i.id === id);
      if (idx !== -1) d.items[idx] = updated;
      // Clean up nonce index if present.
      if (item.requestNonce) {
        d.pendingPermissionsByNonce = new Map(d.pendingPermissionsByNonce);
        d.pendingPermissionsByNonce.delete(item.requestNonce);
      }
    }
  }
  // `pendingPermissions` keys use the compound format `ch:session:turn:id`
  // where `session` and `turn` are unrestricted strings that may themselves
  // contain `:`. Positional splitting is unsafe. Instead, look up the item
  // for each entry and match by its `turnId` — the item already carries the
  // authoritative identity, so no key parsing is needed.
  let permsMutated = false;
  for (const [key, { itemId }] of d.pendingPermissions) {
    const item = d.itemsById.get(itemId);
    if (item && item.turnId === turnId) {
      if (!permsMutated) {
        d.pendingPermissions = new Map(d.pendingPermissions);
        permsMutated = true;
      }
      d.pendingPermissions.delete(key);
    }
  }
}

/**
 * Handle an observer-only `permission_terminal` event.
 * Emitted for uncertain outcomes (process poison, cancel-during-write) where
 * no confirmed ACP wire response is available.
 */
export function handlePermissionTerminal(
  d: PermissionDraftSlice,
  authorization: { requestNonce: string; reason?: string } | undefined | null,
  payload: unknown,
  ch: string,
  ctx: PermCtx,
) {
  const nonce = authorization?.requestNonce;
  if (!nonce) return;
  const itemId = d.pendingPermissionsByNonce.get(nonce);
  if (!itemId) return;
  const existing = d.itemsById.get(itemId);
  if (existing?.type === "lifecycle") {
    setPermissionItem(d, itemId, {
      ...existing,
      outcome:
        "Approval outcome unknown; agent process stopped before it could continue.",
      actionable: false,
    });
  }
  d.pendingPermissionsByNonce = new Map(d.pendingPermissionsByNonce);
  d.pendingPermissionsByNonce.delete(nonce);
  // Clean up any matching compound legacy entry.
  const responseId = jsonRpcIdLocal(asRecord(payload).id);
  if (responseId) {
    const legacyKey = `${ch}:${ctx.sessionId ?? ""}:${ctx.turnId ?? ""}:${responseId}`;
    if (d.pendingPermissions.has(legacyKey)) {
      d.pendingPermissions = new Map(d.pendingPermissions);
      d.pendingPermissions.delete(legacyKey);
    }
  }
}

/**
 * Handle an `acp_write` frame with no `method` — a permission response carrying
 * `result.outcome`. Correlates by nonce (primary) or legacy compound key (fallback).
 */
export function handlePermissionWrite(
  d: PermissionDraftSlice,
  authorization:
    | { requestNonce?: string | null; reason?: string }
    | undefined
    | null,
  payload: Record<string, unknown>,
  ch: string,
  ctx: PermCtx,
) {
  const nonce = authorization?.requestNonce;
  const terminalReason = authorization?.reason;
  const responseId = jsonRpcIdLocal(payload.id);
  const result = asRecord(asRecord(payload.result).outcome);
  const outcomeKind = asString(result.outcome);

  if (nonce !== undefined && nonce !== null) {
    // Nonce present: nonce-only path. Do NOT fall back on unknown nonce.
    const itemIdByNonce = d.pendingPermissionsByNonce.get(nonce);
    if (itemIdByNonce) {
      const existing = d.itemsById.get(itemIdByNonce);
      if (existing?.type === "lifecycle") {
        const outcomeText = describePermissionTerminalReason(
          terminalReason,
          outcomeKind,
          asString(result.optionId) ?? null,
          existing.options,
        );
        setPermissionItem(d, itemIdByNonce, {
          ...existing,
          outcome: outcomeText,
          actionable: false,
        });
      }
      // Clean up nonce index.
      d.pendingPermissionsByNonce = new Map(d.pendingPermissionsByNonce);
      d.pendingPermissionsByNonce.delete(nonce);
      // Clean up compound legacy key if it matches.
      if (responseId) {
        const legacyKey = `${ch}:${ctx.sessionId ?? ""}:${ctx.turnId ?? ""}:${responseId}`;
        if (d.pendingPermissions.has(legacyKey)) {
          d.pendingPermissions = new Map(d.pendingPermissions);
          d.pendingPermissions.delete(legacyKey);
        }
      }
    }
    // Unknown nonce: drop frame — do not mutate any card.
  } else if (outcomeKind && responseId) {
    // No nonce: legacy compound-key fallback for non-ask paths.
    const legacyKey = `${ch}:${ctx.sessionId ?? ""}:${ctx.turnId ?? ""}:${responseId}`;
    const pendingById = d.pendingPermissions.get(legacyKey);
    if (pendingById) {
      const optionId = asString(result.optionId) ?? null;
      const outcomeText = describePermissionOutcome(
        outcomeKind,
        optionId,
        // Legacy path: no harness labels available (non-ask path).
        // Pass an empty labels map so the verb-only fallback ("Approved" /
        // "Denied") renders rather than a raw kind string.
        new Map(),
        pendingById.optionNames,
      );
      const existing = d.itemsById.get(pendingById.itemId);
      if (existing?.type === "lifecycle") {
        setPermissionItem(d, pendingById.itemId, {
          ...existing,
          outcome: outcomeText,
          actionable: false,
        });
      }
      d.pendingPermissions = new Map(d.pendingPermissions);
      d.pendingPermissions.delete(legacyKey);
    }
  }
}

/**
 * Handle a `control_result` frame for a `permission_decision` delivery.
 *
 * Three statuses are treated as authoritative failures — the harness cannot
 * route the decision at all, so the card's `deliveryFailed` token is
 * incremented and the buttons re-enable for a manual retry:
 *   `no_active_turn`, `channel_closed`, `no_channel`
 *
 * Two statuses are success and must not fail the card:
 *   `sent` — the harness forwarded the decision to the live read loop.
 *   `already_decided` — a retransmit matched a nonce the harness had already
 *     applied (suppressed); incrementing `deliveryFailed` here would flip a
 *     correctly-resolved card back to clickable, the exact P1 this exists to
 *     prevent.
 *
 * `channel_full` is a transient queue-saturation condition (one or more
 * eligible queues could not accept the frame). The retransmit orchestrator
 * (`retransmitPermissionDecision.ts`) stays subscribed and keeps resending
 * until the harness accepts or the deadline expires. The card must remain
 * DISABLED while the automatic retry is in progress — do NOT increment
 * `deliveryFailed` here.
 */
export function handlePermissionDecisionResult(
  d: PermissionDraftSlice,
  payload: Record<string, unknown>,
) {
  const frameType = asString(payload.type);
  if (frameType !== "permission_decision") return;
  const deliveryStatus = asString(payload.status);
  // Success statuses — no card update needed.
  if (deliveryStatus === "sent" || deliveryStatus === "already_decided") return;
  // Transient queue-saturation — retransmit orchestrator keeps retrying;
  // do not re-enable the buttons mid-retry.
  if (deliveryStatus === "channel_full") return;
  // Authoritative failure — find the card by nonce and mark it retryable.
  const nonce = asString(payload.requestNonce);
  if (!nonce) return;
  const itemId = d.pendingPermissionsByNonce.get(nonce);
  if (!itemId) return;
  const existing = d.itemsById.get(itemId);
  if (
    existing?.type === "lifecycle" &&
    existing.renderClass === "permission" &&
    existing.actionable
  ) {
    setPermissionItem(d, itemId, {
      ...existing,
      // Increment the failure token so the effect in
      // PermissionDecisionButtons re-fires even when a prior
      // failure already set deliveryFailed (a sticky boolean
      // value would not change on the second failure and the
      // useEffect dependency would not trigger).
      deliveryFailed: (existing.deliveryFailed ?? 0) + 1,
    });
  }
}
