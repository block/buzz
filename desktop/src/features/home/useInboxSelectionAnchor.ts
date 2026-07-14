import * as React from "react";

import { getEventById } from "@/shared/api/tauri";
import type { FeedItem, HomeFeedResponse } from "@/shared/api/types";

type SelectionAnchorResult = {
  /**
   * Flat list of all FeedItems from the home feed across all categories,
   * memoized together so dependents share a single reference.
   */
  feedItems: FeedItem[];
  /**
   * The resolved FeedItem (or synthetic FeedItem for cold recovery) that
   * should drive thread-context loading, composer parent derivation, and
   * conversation-ID selection. Null when the anchor is unresolved.
   *
   * Priority (highest first):
   *  1. Direct match in feedItems for the current anchor ID.
   *  2. Committed same-anchor latch (survives anchor disappearing from feed).
   *  3. Cold-recovered synthetic FeedItem from getEventById (anchor absent from
   *     feed; validated against the active workspace channel set).
   */
  activeLatchedItem: FeedItem | null;
  /**
   * True when a non-null selectedEventId has no resolved context yet and a
   * cold fetch is in-flight (or is about to start on the first render).
   * Derived synchronously so the first render with a cold anchor is already
   * protected. Auto-selection must not overwrite selectedEventId while true.
   * Becomes false on success, terminal failure, or same-key membership rejection.
   */
  coldResolutionPending: boolean;
};

// Per-event-ID status tracked in the cold attempt map.
// "inflight"           — fetch in progress, no result yet.
// "success"            — fetch resolved; FeedItem committed to coldRecoveredItem.
// "terminal-failed"    — network/not-found; never retry.
// membership-rejected  — event found but h-tag not in channel set at resolve
//                        time; membershipKey snapshots the set so a changed
//                        channel set can trigger one retry (different key).
type ColdAttemptStatus =
  | "inflight"
  | "success"
  | "terminal-failed"
  | { kind: "membership-rejected"; membershipKey: string };

/**
 * Resolves the selection-anchor context for HomeView.
 *
 * Owns three layers of anchor resolution so HomeView.tsx stays under its
 * file-size ceiling:
 *
 *  Layer 1 — direct feed match: synchronously returns the FeedItem when it
 *  is present in the current feed snapshot.
 *
 *  Layer 2 — committed latch: once a direct match is found for a given anchor,
 *  that FeedItem is kept in state so it survives the representative advancing
 *  to a newer sibling (which evicts the original event from feedItems).
 *
 *  Layer 3 — cold recovery: when the anchor is absent from feedItems (e.g. a
 *  cold ?item= navigation or back/forward to a stale URL), fetches the event
 *  by ID, validates its `h` channel against the active workspace, and returns
 *  a synthetic FeedItem for context seeding.  The recovered event is never
 *  injected into the live feed snapshot.
 */
export function useInboxSelectionAnchor({
  feed,
  selectedEventId,
  availableChannelIds,
}: {
  feed: HomeFeedResponse | undefined;
  selectedEventId: string | null;
  availableChannelIds: ReadonlySet<string>;
}): SelectionAnchorResult {
  const feedItems = React.useMemo<FeedItem[]>(
    () =>
      feed
        ? [
            ...feed.feed.mentions,
            ...feed.feed.needsAction,
            ...feed.feed.activity,
            ...feed.feed.agentActivity,
          ]
        : [],
    [feed],
  );

  // ── Layer 2: committed same-anchor latch ─────────────────────────────────
  // Once a FeedItem is found for the current anchor, commit it to state so
  // subsequent feed snapshots that no longer contain the anchored event (after
  // a newer sibling becomes the representative) still drive the correct context.
  // Cleared synchronously when the anchor changes to prevent stale committed
  // state from bleeding into the new anchor's first render.
  const [latchedContextItem, setLatchedContextItem] = React.useState<{
    eventId: string;
    feedItem: FeedItem;
  } | null>(null);

  React.useEffect(() => {
    if (selectedEventId === null) {
      setLatchedContextItem(null);
      return;
    }
    const match = feedItems.find((fi) => fi.id === selectedEventId);
    if (match) {
      // Commit once per anchor — don't overwrite a latch that is already for
      // this anchor so live feed updates don't retrigger unnecessary state.
      setLatchedContextItem((prev) => {
        if (prev?.eventId === selectedEventId) return prev;
        return { eventId: selectedEventId, feedItem: match };
      });
    } else if (
      latchedContextItem !== null &&
      latchedContextItem.eventId !== selectedEventId
    ) {
      // Anchor changed and no direct match yet — clear the stale latch so the
      // next render does not see a mismatched committed item.
      setLatchedContextItem(null);
    }
  }, [feedItems, selectedEventId, latchedContextItem]);

  // ── Layer 3: cold getEventById recovery ──────────────────────────────────
  // When the anchor is absent from feedItems and no committed latch exists,
  // fetch the event by ID to recover its conversationId and default parent.
  //
  // Attempt-map design (coldAttempts: Map<eventId, ColdAttemptStatus>):
  //   coldResolutionPending is derived SYNCHRONOUSLY each render so the very
  //   first render where selectedEventId changes to a cold anchor is protected.
  //   An effect-only setColdResolutionPending(true) would fire too late because
  //   HomeView's auto-selection effect reads the closure from the same render.
  //
  //   State machine per anchor:
  //     absent           → pending=true,  effect starts fetch
  //     "inflight"       → pending=true,  effect skips (already running)
  //     "success"        → pending=false, effect skips
  //     "terminal-failed"→ pending=false, effect skips (network/not-found)
  //     membership-rejected, same membershipKey  → pending=false, effect skips
  //     membership-rejected, diff membershipKey  → pending=true,  effect retries
  //
  //   latestSelectedEventIdRef / latestAvailableChannelIdsRef: always-current
  //   mirrors for async callback use (anchor-supersession and h-tag validation).
  //   mountedRef: component-lifetime guard against post-unmount setState.
  const [coldAttempts, setColdAttempts] = React.useState<
    Map<string, ColdAttemptStatus>
  >(new Map());
  const [coldRecoveredItem, setColdRecoveredItem] = React.useState<{
    eventId: string;
    feedItem: FeedItem;
  } | null>(null);

  // Always-current refs — updated synchronously on every render.
  const latestSelectedEventIdRef = React.useRef<string | null>(selectedEventId);
  const latestAvailableChannelIdsRef =
    React.useRef<ReadonlySet<string>>(availableChannelIds);
  latestSelectedEventIdRef.current = selectedEventId;
  latestAvailableChannelIdsRef.current = availableChannelIds;

  // Component-lifetime unmount guard — never written by dep-change cleanups.
  const mountedRef = React.useRef(true);
  React.useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

  // Stable key for the current channel set — used to detect membership changes
  // so membership-rejected failures can be retried when channels load/change.
  const membershipKey = React.useMemo(
    () => [...availableChannelIds].sort().join(","),
    [availableChannelIds],
  );

  // Boolean flags for the current anchor.
  const hasMatchingLatch =
    latchedContextItem !== null &&
    latchedContextItem.eventId === selectedEventId;

  const coldMatchesAnchor =
    coldRecoveredItem !== null && coldRecoveredItem.eventId === selectedEventId;
  const coldChannelValid =
    coldMatchesAnchor &&
    coldRecoveredItem.feedItem.channelId !== null &&
    availableChannelIds.has(coldRecoveredItem.feedItem.channelId);
  const hasMatchingCold = coldMatchesAnchor && coldChannelValid;

  const directInFeed =
    selectedEventId !== null &&
    feedItems.some((fi) => fi.id === selectedEventId);

  // ── Synchronous coldResolutionPending derivation ─────────────────────────
  // Must be derived synchronously (not via setState) so the first render where
  // selectedEventId is a cold anchor is already protected from auto-selection.
  //
  //   pending=true  when: absent attempt OR inflight OR membership-rejected
  //                       with a DIFFERENT key (retry will fire this render).
  //   pending=false when: anchor null/in-feed/latched/cold-resolved,
  //                       terminal-failed, or membership-rejected with SAME key.
  let coldResolutionPending = false;
  if (
    selectedEventId !== null &&
    !directInFeed &&
    !hasMatchingLatch &&
    !hasMatchingCold
  ) {
    const attempt = coldAttempts.get(selectedEventId);
    if (attempt === undefined || attempt === "inflight") {
      coldResolutionPending = true;
    } else if (
      typeof attempt === "object" &&
      attempt.membershipKey !== membershipKey
    ) {
      // Membership-rejected with a STALE key → channels changed, retry pending.
      coldResolutionPending = true;
    }
    // "terminal-failed"                        → false (no retry)
    // membership-rejected, same membershipKey  → false (unblocks auto-select)
    // "success"                                → false (hasMatchingCold covers)
  }

  // ── Cold recovery effect ──────────────────────────────────────────────────
  React.useEffect(() => {
    if (!selectedEventId) {
      setColdAttempts((prev) => (prev.size > 0 ? new Map() : prev));
      setColdRecoveredItem(null);
      return;
    }

    if (directInFeed) {
      setColdRecoveredItem((prev) =>
        prev?.eventId === selectedEventId ? null : prev,
      );
      return;
    }

    if (hasMatchingLatch) return;
    if (hasMatchingCold) return;

    // NOTE: do NOT clear coldRecoveredItem when coldMatchesAnchor && !coldChannelValid.
    // The candidate is retained; hasMatchingCold gates its active projection via
    // coldChannelValid. Clearing here would trigger a duplicate getEventById that
    // cannot change the already-known h-tag, and membership returning would never
    // reactivate without a cleared attempt. The settled success/failure state is
    // the source of truth; membership-gating is purely a projection concern.

    const currentAttempt = coldAttempts.get(selectedEventId);

    // Skip: already in-flight, terminal failure, success, or same-key membership rejection.
    if (
      currentAttempt === "inflight" ||
      currentAttempt === "terminal-failed" ||
      currentAttempt === "success"
    ) {
      return;
    }
    if (
      typeof currentAttempt === "object" &&
      currentAttempt.membershipKey === membershipKey
    ) {
      return;
    }

    // Start a fetch (new, or retry after a membership change).
    const fetchAnchor = selectedEventId;
    setColdAttempts((prev) => {
      const next = new Map(prev);
      next.set(fetchAnchor, "inflight");
      return next;
    });

    void getEventById(fetchAnchor).then(
      (event) => {
        if (!mountedRef.current) return;
        // Anchor superseded — remove inflight status so a back-navigation can
        // restart the fetch rather than being silently deduped.
        if (latestSelectedEventIdRef.current !== fetchAnchor) {
          setColdAttempts((prev) => {
            if (prev.get(fetchAnchor) !== "inflight") return prev;
            const next = new Map(prev);
            next.delete(fetchAnchor);
            return next;
          });
          return;
        }
        const hTag = event.tags.find((t) => t[0] === "h")?.[1] ?? null;
        if (!hTag || !latestAvailableChannelIdsRef.current.has(hTag)) {
          // Event found but not in the current workspace — record membership
          // snapshot so a channel-set change triggers exactly one retry.
          const failedKey = [...latestAvailableChannelIdsRef.current]
            .sort()
            .join(",");
          setColdAttempts((prev) => {
            const next = new Map(prev);
            next.set(fetchAnchor, {
              kind: "membership-rejected",
              membershipKey: failedKey,
            });
            return next;
          });
          return;
        }
        const syntheticFeedItem: FeedItem = {
          id: event.id,
          kind: event.kind,
          pubkey: event.pubkey,
          content: event.content,
          createdAt: event.created_at,
          tags: event.tags,
          channelId: hTag,
          channelName: "",
          channelType: undefined,
          category: "mention",
        };
        setColdAttempts((prev) => {
          const next = new Map(prev);
          next.set(fetchAnchor, "success");
          return next;
        });
        setColdRecoveredItem({
          eventId: event.id,
          feedItem: syntheticFeedItem,
        });
      },
      () => {
        if (!mountedRef.current) return;
        // Network/not-found — terminal failure, never retry.
        setColdAttempts((prev) => {
          if (prev.get(fetchAnchor) !== "inflight") return prev;
          const next = new Map(prev);
          next.set(fetchAnchor, "terminal-failed");
          return next;
        });
      },
    );
    // No cleanup return: getEventById is not cancellable. mountedRef guards
    // post-unmount writes; latestSelectedEventIdRef detects anchor supersession.
  }, [
    selectedEventId,
    directInFeed,
    hasMatchingLatch,
    hasMatchingCold,
    coldAttempts,
    membershipKey,
  ]);

  // ── Active latch derivation ───────────────────────────────────────────────
  // Synchronous direct match wins; then committed latch (survives eviction from
  // feed); then cold recovery (absent-from-feed path). Non-direct cached layers
  // are gated by workspace membership so a shrinking availableChannelIds does
  // not serve a stale context seed.
  const directSelectedFeedItem = React.useMemo(
    () =>
      selectedEventId
        ? (feedItems.find((fi) => fi.id === selectedEventId) ?? null)
        : null,
    [feedItems, selectedEventId],
  );

  const latchedFeedItem =
    latchedContextItem?.eventId === selectedEventId &&
    latchedContextItem.feedItem.channelId !== null &&
    availableChannelIds.has(latchedContextItem.feedItem.channelId)
      ? latchedContextItem.feedItem
      : null;

  const coldFeedItem =
    hasMatchingCold && coldRecoveredItem !== null
      ? coldRecoveredItem.feedItem
      : null;

  const activeLatchedItem =
    directSelectedFeedItem ?? latchedFeedItem ?? coldFeedItem;

  return { feedItems, activeLatchedItem, coldResolutionPending };
}
