/**
 * Utilities for the `buzz:todo-card` sentinel — an interactive to-do card
 * embedded in an ordinary stream message (kind:9 / kind:40002) body.
 *
 * Wire format (v1, authored by agents via buzz-sdk):
 *
 * ```
 * ```buzz:todo-card
 * {"v":1,"title":"…","items":[{"id":"…","text":"…","assignee":"<hex pubkey>"}]}
 * ```
 * ```
 *
 * The prose above the fence is a plaintext fallback for non-card clients.
 * The desktop detects the sentinel here, suppresses the prose, and renders a
 * `TodoCardAttachment`. Check-offs are kind:40009 responses signed by the
 * clicking user (tags: `e` → card event id, `item` → item id, `h` → channel;
 * content: `{"done":true|false}`); card state is a pure client-side fold over
 * those events — the relay stores raw events only.
 */

import type { RelayEvent } from "@/shared/api/types";

// ── Types ─────────────────────────────────────────────────────────────────────

/** A single to-do item in the card payload. */
export type TodoCardItem = {
  /** Card-unique item id, referenced by response `item` tags. */
  id: string;
  text: string;
  /** Hex pubkey of the assignee. Absent = anyone may complete the item. */
  assignee?: string;
};

/** The structured payload embedded in the `buzz:todo-card` sentinel block. */
export type TodoCardPayload = {
  v: 1;
  title?: string;
  items: TodoCardItem[];
};

/** Folded state of one item after replaying its kind:40009 responses. */
export type TodoItemState = {
  done: boolean;
  /** Pubkey whose response completed the item (may differ from assignee). */
  completedBy: string | null;
  /** `created_at` of the completing response. */
  completedAt: number | null;
};

// ── Constants ─────────────────────────────────────────────────────────────────

const FENCE_OPEN = "```buzz:todo-card";
const FENCE_CLOSE = "```";

/** MVP cap — payloads with more items are rejected (prose fallback). */
export const MAX_TODO_CARD_ITEMS = 20;

// ── Extractor ─────────────────────────────────────────────────────────────────

/**
 * Extract the `TodoCardPayload` from a message body, if present.
 *
 * Returns `null` when:
 * - the sentinel fence is absent
 * - the JSON inside is malformed
 * - the parsed value doesn't match the expected shape
 * - the payload exceeds `MAX_TODO_CARD_ITEMS` or has duplicate item ids
 *
 * Never throws — all errors are swallowed so this is safe to call in the
 * render path.
 */
export function extractTodoCard(content: string): TodoCardPayload | null {
  const openIdx = content.indexOf(FENCE_OPEN);
  if (openIdx === -1) return null;

  // The JSON starts on the line after the opening fence.
  const jsonStart = content.indexOf("\n", openIdx);
  if (jsonStart === -1) return null;

  // The JSON ends at the next closing ``` that appears on its own line.
  const closeIdx = content.indexOf(`\n${FENCE_CLOSE}`, jsonStart);
  if (closeIdx === -1) return null;

  const json = content.slice(jsonStart + 1, closeIdx).trim();
  if (!json) return null;

  try {
    const parsed: unknown = JSON.parse(json);
    return isTodoCardPayload(parsed) ? parsed : null;
  } catch {
    return null;
  }
}

/**
 * Strip the `buzz:todo-card` sentinel block (and any preceding blank line)
 * from a message body. Returns the original string unchanged when no sentinel
 * is present.
 *
 * Used so the prose fallback is rendered without the raw code block.
 */
export function stripTodoCardSentinel(content: string): string {
  const openIdx = content.indexOf(FENCE_OPEN);
  if (openIdx === -1) return content;

  const closeIdx = content.indexOf(`\n${FENCE_CLOSE}`, openIdx);
  if (closeIdx === -1) return content;

  const afterFence = closeIdx + `\n${FENCE_CLOSE}`.length;
  // Trim a preceding blank line so the prose doesn't gain a trailing gap.
  const prose = content.slice(0, openIdx).replace(/\n{2,}$/, "\n");
  return prose + content.slice(afterFence);
}

// ── Response fold ─────────────────────────────────────────────────────────────

function responseTargetsCard(event: RelayEvent, cardEventId: string): boolean {
  return event.tags.some((tag) => tag[0] === "e" && tag[1] === cardEventId);
}

function responseItemId(event: RelayEvent): string | null {
  const value = event.tags.find((tag) => tag[0] === "item")?.[1];
  return typeof value === "string" && value.length > 0 ? value : null;
}

function responseDone(event: RelayEvent): boolean | null {
  try {
    const parsed = JSON.parse(event.content) as { done?: unknown };
    return typeof parsed.done === "boolean" ? parsed.done : null;
  } catch {
    return null;
  }
}

/**
 * Fold kind:40009 responses into per-item state.
 *
 * Semantics (MVP policy — display-side, the relay never interprets these):
 * - Only the latest response per `(item, pubkey)` counts, so a responder can
 *   un-check their own completion by publishing `{"done":false}` — and only
 *   their own.
 * - An assigned item follows its assignee's latest response when the assignee
 *   has responded; otherwise any responder's completion counts, attributed to
 *   the most recent completer ("completed by X" when X ≠ assignee).
 * - An unassigned item is done when any responder's latest response is
 *   `{"done":true}`, attributed to the most recent completer.
 *
 * Malformed responses (no `item` tag, unknown item id, non-boolean `done`,
 * wrong card `e` tag) are ignored. Never throws.
 */
export function reduceTodoResponses(
  card: TodoCardPayload,
  cardEventId: string,
  events: Iterable<RelayEvent>,
): Map<string, TodoItemState> {
  const itemsById = new Map(card.items.map((item) => [item.id, item]));

  // Latest response per (item, pubkey), replayed in deterministic order.
  const sorted = [...events]
    .filter((event) => responseTargetsCard(event, cardEventId))
    .sort(
      (left, right) =>
        left.created_at - right.created_at || left.id.localeCompare(right.id),
    );

  const latestByItemAndPubkey = new Map<
    string,
    Map<string, { done: boolean; createdAt: number }>
  >();
  for (const event of sorted) {
    const itemId = responseItemId(event);
    if (itemId === null || !itemsById.has(itemId)) continue;
    const done = responseDone(event);
    if (done === null || !event.pubkey) continue;

    let byPubkey = latestByItemAndPubkey.get(itemId);
    if (!byPubkey) {
      byPubkey = new Map();
      latestByItemAndPubkey.set(itemId, byPubkey);
    }
    byPubkey.set(event.pubkey, { done, createdAt: event.created_at });
  }

  const state = new Map<string, TodoItemState>();
  for (const item of card.items) {
    const byPubkey = latestByItemAndPubkey.get(item.id);
    const assigneeLatest = item.assignee
      ? byPubkey?.get(item.assignee)
      : undefined;

    if (assigneeLatest) {
      state.set(item.id, {
        done: assigneeLatest.done,
        completedBy: assigneeLatest.done ? (item.assignee ?? null) : null,
        completedAt: assigneeLatest.done ? assigneeLatest.createdAt : null,
      });
      continue;
    }

    // No assignee response — the most recent still-active completion wins.
    let completedBy: string | null = null;
    let completedAt: number | null = null;
    for (const [pubkey, latest] of byPubkey ?? []) {
      if (!latest.done) continue;
      if (completedAt === null || latest.createdAt > completedAt) {
        completedBy = pubkey;
        completedAt = latest.createdAt;
      }
    }
    state.set(item.id, {
      done: completedBy !== null,
      completedBy,
      completedAt,
    });
  }

  return state;
}

/** Count of done items for the card-level "n of m done" line. */
export function countDoneItems(
  card: TodoCardPayload,
  state: Map<string, TodoItemState>,
): number {
  return card.items.filter((item) => state.get(item.id)?.done).length;
}

// ── Type-guard ─────────────────────────────────────────────────────────────────

function isTodoCardItem(v: unknown): v is TodoCardItem {
  if (typeof v !== "object" || v === null) return false;
  const item = v as Record<string, unknown>;
  return (
    typeof item.id === "string" &&
    item.id.length > 0 &&
    typeof item.text === "string" &&
    (item.assignee === undefined ||
      (typeof item.assignee === "string" && item.assignee.length > 0))
  );
}

function isTodoCardPayload(v: unknown): v is TodoCardPayload {
  if (typeof v !== "object" || v === null) return false;
  const p = v as Record<string, unknown>;
  if (p.v !== 1) return false;
  if (p.title !== undefined && typeof p.title !== "string") return false;
  if (!Array.isArray(p.items) || p.items.length === 0) return false;
  if (p.items.length > MAX_TODO_CARD_ITEMS) return false;
  if (!p.items.every(isTodoCardItem)) return false;
  const ids = new Set(p.items.map((item) => (item as TodoCardItem).id));
  return ids.size === p.items.length;
}
