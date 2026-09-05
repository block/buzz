/**
 * Shared helpers for the admin console panel sub-components.
 *
 * Exported from here to avoid duplication across AdminConsolePanel.tsx,
 * AdminConsoleFeedbackTab.tsx, and AdminConsoleStaffingTab.tsx.
 */

import { useEffect, useRef, useState } from "react";
import type { ReactNode } from "react";
import { AlertCircle, LoaderCircle } from "lucide-react";
import { formatRelativeTime } from "../forum/lib/time";

// ── Generic async state ───────────────────────────────────────────────────

export type AsyncState<T> =
  | { status: "idle" }
  | { status: "loading" }
  | { status: "ok"; data: T }
  | { status: "error"; message: string };

/**
 * Async load hook with effect-local active-flag cancellation.
 *
 * Each effect invocation sets `active = true` and flips it to `false` in the
 * cleanup function. Completions check `active` before calling setState, so a
 * result that arrives after the deps changed (or the component unmounted) is
 * silently discarded.
 *
 * `load` is stored in a ref so it is not a dependency of the effect — callers
 * create it inline and `deps` + `generation` are the explicit trigger list.
 */
export function useAsyncLoad<T>(
  load: () => Promise<T>,
  deps: unknown[],
  generation: number,
): AsyncState<T> {
  const [state, setState] = useState<AsyncState<T>>({ status: "idle" });
  const loadRef = useRef(load);
  loadRef.current = load;

  // biome-ignore lint/correctness/useExhaustiveDependencies: loadRef is a stable ref; deps and generation are the intentional trigger set
  useEffect(() => {
    let active = true;
    setState({ status: "loading" });
    loadRef.current().then(
      (data) => {
        if (!active) return;
        setState({ status: "ok", data });
      },
      (e: unknown) => {
        if (!active) return;
        setState({
          status: "error",
          message: e instanceof Error ? e.message : String(e),
        });
      },
    );
    return () => {
      active = false;
    };
  }, [...deps, generation]);

  return state;
}

// ── Admin error message parsing ───────────────────────────────────────────

/**
 * Extract a human-readable message from an admin mutation error.
 *
 * Native admin commands reject with `admin API error: {json}` where the JSON
 * is the relay's error envelope (`{"error":{"code","message","requestId"}}`).
 * This strips the prefix and returns the envelope's `message` field so the UI
 * can surface "action kick requires the report to have an associated channel"
 * instead of the raw JSON. Falls back to the raw string when the payload is
 * not the expected shape (network errors, non-JSON bodies).
 */
export function adminErrorMessage(e: unknown): string {
  const raw = e instanceof Error ? e.message : String(e);
  const jsonStart = raw.indexOf("{");
  if (jsonStart === -1) return raw;
  try {
    const parsed = JSON.parse(raw.slice(jsonStart));
    const message = parsed?.error?.message;
    return typeof message === "string" && message.length > 0 ? message : raw;
  } catch {
    return raw;
  }
}

/**
 * Extract the relay's HTTP status from a rejected admin mutation, or `null`.
 *
 * Native mutation commands reject with a typed `AdminMutationError`
 * (`{message, relayStatus, bodyComplete}`); the Tauri bridge surfaces it as
 * `TauriInvokeError` whose `payload` is that object. `relayStatus` is a number
 * only when the relay actually answered — `null`/absent for a transport or
 * pre-send failure where no relay verdict exists.
 */
export function adminMutationRelayStatus(e: unknown): number | null {
  if (e && typeof e === "object" && "payload" in e) {
    const payload = (e as { payload: unknown }).payload;
    if (payload && typeof payload === "object" && "relayStatus" in payload) {
      const status = (payload as { relayStatus: unknown }).relayStatus;
      if (typeof status === "number") return status;
    }
  }
  return null;
}

/**
 * Whether the relay's full response body was read — an authoritative verdict.
 *
 * `AdminMutationError.bodyComplete` is `true` only when the relay answered AND
 * its whole body was received. A status that arrives but whose body is lost
 * mid-stream (or rejected over the size cap) is `false`: the outcome is
 * unknown. Absent/non-boolean payloads (bare-string errors, non-typed
 * rejections) read `false`, which is fail-safe — an unknown outcome preserves
 * the idempotency key.
 */
export function adminMutationBodyComplete(e: unknown): boolean {
  if (e && typeof e === "object" && "payload" in e) {
    const payload = (e as { payload: unknown }).payload;
    if (payload && typeof payload === "object" && "bodyComplete" in payload) {
      const complete = (payload as { bodyComplete: unknown }).bodyComplete;
      if (typeof complete === "boolean") return complete;
    }
  }
  return false;
}

/**
 * Whether a failed mutation must reuse its idempotency `requestId` on retry.
 *
 * The id is preserved UNLESS the relay definitively rejected the request before
 * committing — a non-409 4xx whose full body was read. Those (bad action,
 * unauthorized, not found) refuse the input pre-commit, so a corrected
 * resubmission is a genuinely new command and a fresh id is safe.
 *
 * Everything else preserves the id so the relay can dedupe against a commit
 * that may have landed:
 *   - 409 — an idempotency claim or in-progress action already exists;
 *   - 5xx — the relay may have committed before failing;
 *   - a lost or truncated response body (status arrived, `bodyComplete` false —
 *     outcome unknown), including a truncated 4xx;
 *   - a transport or pre-send failure with no relay answer (`relayStatus` null).
 *
 * Status alone is insufficient: a truncated 4xx carries a definitive-looking
 * status without an authoritative body, so the `bodyComplete` bit gates the
 * reset. This replaces string-matching `"409"`/`"processing"` on the message,
 * which missed the native layer's transport errors and cleared the id on
 * exactly the ambiguous lost-response failures where reuse is required.
 */
export function preserveRequestIdOnError(e: unknown): boolean {
  const status = adminMutationRelayStatus(e);
  if (status === null) return true;
  if (status === 409) return true;
  if (status < 400 || status >= 500) return true;
  // A non-409 4xx resets only when the relay's full body confirmed the verdict.
  return !adminMutationBodyComplete(e);
}

// ── Shared UI helpers ─────────────────────────────────────────────────────

export function LoadingSpinner() {
  return (
    <div className="flex items-center gap-2 text-sm text-muted-foreground">
      <LoaderCircle className="h-4 w-4 animate-spin" />
      Loading…
    </div>
  );
}

export function ErrorMessage({ message }: { message: string }) {
  return (
    <div className="flex items-center gap-1.5 text-sm text-destructive">
      <AlertCircle className="h-4 w-4" />
      {message}
    </div>
  );
}

// ── Timestamp formatter ───────────────────────────────────────────────────

export function formatTimestamp(raw: string | null | undefined): string {
  if (!raw) return "—";
  const date = new Date(raw);
  if (Number.isNaN(date.getTime())) return raw;
  const absolute = date.toLocaleString(undefined, {
    month: "short",
    day: "numeric",
    year: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
  const rel = formatRelativeTime(Math.floor(date.getTime() / 1000));
  // Render relative label with the absolute value inline in parentheses.
  return `${rel} (${absolute})`;
}

// ── Structured detail row ─────────────────────────────────────────────────

export function DetailRow({
  label,
  value,
  mono,
}: {
  label: string;
  value: string | null | undefined;
  mono?: boolean;
}) {
  return (
    <div className="flex gap-2 text-xs">
      <span className="w-28 shrink-0 text-muted-foreground">{label}</span>
      <span className={mono ? "font-mono break-all" : "break-words"}>
        {value ?? "—"}
      </span>
    </div>
  );
}

// ── Attachment meta ───────────────────────────────────────────────────────

export type AttachmentMeta = {
  /** Lowercase 64-hex SHA-256 as stored/returned by the relay. */
  sha256: string;
  /** MIME type from the `m` imeta field. */
  mime: string;
  /** Byte size from the `size` imeta field. */
  size: number;
};

/**
 * Parse imeta attachment metadata from the relay's `tags: string[][]` wire
 * format. Matches the reference SPA implementation in `admin-web/src/App.tsx`.
 *
 * Each `imeta` tag looks like:
 *   `["imeta", "url https://...", "m image/png", "x <sha256>", "size 12345"]`
 * Each entry after `"imeta"` is a singleton `"key value"` string.
 *
 * Rejected: missing x/m/size, non-lowercase-hex x, non-positive size.
 */
export function parseImetaAttachments(tags: unknown): AttachmentMeta[] {
  if (!Array.isArray(tags)) return [];
  const result: AttachmentMeta[] = [];
  for (const tag of tags) {
    if (!Array.isArray(tag) || tag[0] !== "imeta") continue;
    const values = new Map<string, string>();
    for (const entry of (tag as string[]).slice(1)) {
      const sep = typeof entry === "string" ? entry.indexOf(" ") : -1;
      if (sep > 0) {
        values.set(entry.slice(0, sep), entry.slice(sep + 1));
      }
    }
    const sha256 = values.get("x") ?? "";
    const mime = values.get("m") ?? "";
    const rawSize = values.get("size") ?? "";
    const size = Number(rawSize);
    // Require exactly 64 lowercase hex chars for the hash (relay stores lowercase;
    // uppercase returns 404). Require a non-empty MIME type and a positive size.
    if (
      sha256.length !== 64 ||
      !/^[0-9a-f]{64}$/.test(sha256) ||
      !mime ||
      !Number.isFinite(size) ||
      size <= 0
    ) {
      continue;
    }
    result.push({ sha256, mime, size });
  }
  return result;
}

// ── Community grouping ─────────────────────────────────────────────────────

/** A run of rows that share one community, tagged with its display host. */
export type CommunityGroup<T> = {
  /** Stable community identifier — used as the React key. */
  communityId: string;
  /** Human-facing host label rendered as the group heading. */
  communityHost: string;
  items: T[];
};

/** React key + heading for rows whose source community has been purged. */
const SEVERED_COMMUNITY_KEY = "__severed__";
const SEVERED_COMMUNITY_HOST = "(source community removed)";

/**
 * Group deployment-wide rows by community, preserving each community's
 * first-seen order and the server's row order within it.
 *
 * The admin API returns reports and feedback across every community on the
 * deployment; operators triage per community, so rows are bucketed by
 * `communityId` (stable) and labelled by `communityHost` (display). A blank
 * host falls back to the id so a group is never headed by an empty string.
 *
 * Feedback whose source community was purged carries a `null` `communityId`
 * (tenant provenance severed, the row retained as operator evidence). Those
 * rows bucket into a single "source community removed" group so a null key
 * never collapses distinct rows or heads a group with an empty string.
 */
export function groupByCommunity<
  T extends { communityId: string | null; communityHost: string | null },
>(items: T[]): CommunityGroup<T>[] {
  const groups: CommunityGroup<T>[] = [];
  const byId = new Map<string, CommunityGroup<T>>();
  for (const item of items) {
    const key = item.communityId ?? SEVERED_COMMUNITY_KEY;
    let group = byId.get(key);
    if (!group) {
      group = {
        communityId: key,
        communityHost:
          item.communityId == null
            ? SEVERED_COMMUNITY_HOST
            : item.communityHost || item.communityId,
        items: [],
      };
      byId.set(key, group);
      groups.push(group);
    }
    group.items.push(item);
  }
  return groups;
}

/**
 * Render community-grouped rows under per-community headings.
 *
 * A single community collapses to a flat list (no redundant heading); two or
 * more render a labelled section each. `renderItem` produces the row for one
 * entry — the caller owns row markup so navigation/testids are unchanged.
 */
export function CommunityGroupedList<
  T extends { communityId: string | null; communityHost: string | null },
>({ items, renderItem }: { items: T[]; renderItem: (item: T) => ReactNode }) {
  const groups = groupByCommunity(items);
  if (groups.length <= 1) {
    return <ul className="space-y-1">{items.map(renderItem)}</ul>;
  }
  return (
    <div className="space-y-4">
      {groups.map((group) => (
        <section key={group.communityId} data-testid="community-group">
          <h4
            className="mb-1.5 text-xs font-semibold text-muted-foreground"
            data-testid="community-group-host"
          >
            {group.communityHost}
          </h4>
          <ul className="space-y-1">{group.items.map(renderItem)}</ul>
        </section>
      ))}
    </div>
  );
}
