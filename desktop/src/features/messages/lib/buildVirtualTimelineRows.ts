/**
 * SPIKE (Phase 2 virtualization feasibility): flatten the day-grouped main
 * timeline into a single ordered list of "virtual rows".
 *
 * `@tanstack/react-virtual` measures and keys a FLAT, index-addressable list. But
 * the live timeline renders a NESTED shape — `<section>` per day, each wrapping a
 * `<DayDivider>` plus its message `<div>`s (see `TimelineMessageList.tsx`). A
 * virtualizer can't measure a nested-section tree; it needs every visually
 * distinct, separately-measured thing (each divider AND each message) as its own
 * top-level row.
 *
 * This helper performs exactly that flattening, off the SAME snapshot the rows
 * render from (the correctness property `timelineSnapshot.ts` documents). It
 * reuses `buildDayGroupBoundaries` so divider placement is byte-identical to the
 * current render — no second source of truth for "where does a day start".
 *
 * Row kinds come out, in render order:
 *   - { kind: "intro", key: "channel-intro" } — optional terminal header row
 *   - { kind: "day-divider", key, label-source timestamp, messageIndex: -1 }
 *   - { kind: "message", key, messageIndex } — index back into the snapshot
 *
 * `key` is the virtualizer's stable identity (`getItemKey`). For messages it
 * prefers `renderKey` (stable across optimistic send-ack, mirroring
 * `selectLatestMessageKey`) and falls back to `id`. Stable keys are what let the
 * virtualizer hold scroll position on PREPEND: when older pages splice in at the
 * top, every surviving row keeps its key, so the measurement cache survives and
 * the virtualizer re-anchors natively — the mechanism that lets us DELETE the
 * double-rAF `scrollTop` correction in `useLoadOlderOnScroll.ts`.
 */

import type { TimelineMessage } from "@/features/messages/types";
import { buildDayGroupBoundaries } from "./timelineSnapshot";

/** Options for flattening a snapshot into virtual rows. */
export type BuildVirtualTimelineRowsOptions = {
  /** When true, prepend a channel-intro row before dividers and messages. */
  includeIntro?: boolean;
};

/** The channel intro header — the terminal row at the true top of history. */
export type VirtualIntroRow = {
  kind: "intro";
  /** Stable virtualizer key. */
  key: "channel-intro";
  /** Always -1 — the intro does not map to a message. */
  messageIndex: -1;
};

/** A divider row — one per calendar-day boundary. Carries no message. */
export type VirtualDayDividerRow = {
  kind: "day-divider";
  /** Stable virtualizer key. */
  key: string;
  /** `createdAt` (unix seconds) of the first message in the day; drives the label. */
  headingTimestamp: number;
  /** Always -1 for dividers — they don't map to a message. */
  messageIndex: -1;
};

/** A message row — one per timeline message, in snapshot order. */
export type VirtualMessageRow = {
  kind: "message";
  /** Stable virtualizer key — prefers `renderKey`, falls back to `id`. */
  key: string;
  /** Index back into the source snapshot, for O(1) message lookup at render. */
  messageIndex: number;
};

export type VirtualTimelineRow =
  | VirtualIntroRow
  | VirtualDayDividerRow
  | VirtualMessageRow;

/**
 * Flatten a message snapshot into ordered virtual rows (intro + dividers + messages).
 *
 * Walks the snapshot once; emits a divider row at each day-group start index
 * (computed by `buildDayGroupBoundaries`, the same helper the live render uses),
 * then the message row. Order is identical to the current nested render read
 * top-to-bottom, so a virtualizer driven off this list paints the exact same
 * visual sequence.
 */
export function buildVirtualTimelineRows(
  messages: readonly TimelineMessage[],
  options?: BuildVirtualTimelineRowsOptions,
): VirtualTimelineRow[] {
  const rows: VirtualTimelineRow[] = [];

  if (options?.includeIntro) {
    rows.push({
      kind: "intro",
      key: "channel-intro",
      messageIndex: -1,
    });
  }

  const dayStartIndices = new Set(
    buildDayGroupBoundaries(messages).map((boundary) => boundary.startIndex),
  );

  for (let i = 0; i < messages.length; i++) {
    const message = messages[i];

    if (dayStartIndices.has(i)) {
      rows.push({
        kind: "day-divider",
        key: `day-${message.createdAt}`,
        headingTimestamp: message.createdAt,
        messageIndex: -1,
      });
    }

    rows.push({
      kind: "message",
      key: message.renderKey ?? message.id,
      messageIndex: i,
    });
  }

  return rows;
}

/**
 * Find the flat virtual-row index for a target message id. The virtualizer's
 * `scrollToIndex` needs a FLAT index, not a message index — and the two diverge
 * because divider rows are interleaved. This is the bridge that lets BOTH
 * jump-to-message deep links AND find-in-page (cmd+F replacement) drive
 * `scrollToIndex` against a virtualized list, since the target row may be
 * unmounted (the whole reason native find breaks under virtualization).
 *
 * Returns -1 when the target isn't in this snapshot — same "row not committed
 * yet, bail and wait for the next snapshot" contract as `resolveDeepLinkTarget`.
 */
export function findVirtualRowIndexForMessage(
  rows: readonly VirtualTimelineRow[],
  targetMessageId: string | null | undefined,
  messages: readonly TimelineMessage[],
): number {
  if (!targetMessageId) {
    return -1;
  }

  const messageIndex = messages.findIndex(
    (message) => message.id === targetMessageId,
  );
  if (messageIndex === -1) {
    return -1;
  }

  return rows.findIndex(
    (row) => row.kind === "message" && row.messageIndex === messageIndex,
  );
}
