/**
 * Shared constants, types, stores, and pure helpers extracted from
 * useUnreadChannels to keep that file within the repository size ratchet.
 * All exports are load-bearing for useUnreadChannels.ts.
 */
import * as React from "react";
import type { UseLiveChannelUpdatesOptions } from "@/features/channels/useLiveChannelUpdates";
import { makeRootIdStore } from "@/features/channels/unreadRootIdStore";
import {
  getThreadReference,
  isBroadcastReply,
} from "@/features/messages/lib/threading";
import type { RelayClient } from "@/shared/api/relayClientSession";
import type { Channel } from "@/shared/api/types";
import { CHANNEL_MESSAGE_EVENT_KINDS } from "@/shared/constants/kinds";
import { DM_NOTIFIABLE_EVENT_KINDS } from "./isDmNotifiableKind";
import {
  forcedUnreadStore,
  removeForcedUnreadSource,
  type ForcedUnreadEntry,
  type ForcedUnreadMap,
  type ForcedUnreadSource,
} from "@/features/channels/forcedUnreadStore";
import type { DrainOutcome } from "@/features/channels/readState/readStateDrain";

export type UseUnreadChannelsOptions = UseLiveChannelUpdatesOptions & {
  pubkey?: string;
  relayClient?: RelayClient;
  relayUrl?: string;
  mutedChannelIds?: ReadonlySet<string>;
};

// Per-channel cap on the catch-up REQ. We only consume the *max matching*
// event per channel, but the relay can return self-authored / non-trigger
// events that we discard client-side, so we need enough head-room for the
// filter to find one external trigger message. 1000 matches the live sub's
// per-channel limit elsewhere in the app.
export const CATCH_UP_LIMIT = 1000;

export function channelCatchUpEventKinds(
  channelType: Channel["channelType"] | undefined,
) {
  return channelType === "dm"
    ? DM_NOTIFIABLE_EVENT_KINDS
    : CHANNEL_MESSAGE_EVENT_KINDS;
}

export const participationStore = makeRootIdStore(
  "buzz-thread-participation.v1",
);
export const authoredStore = makeRootIdStore("buzz-thread-authored.v1");
// Thread roots where an external message @-mentioned the current user. The
// badge gate ORs this in so a mention recipient who never participated,
// authored, or followed still gets the thread-unread badge.
export const mentionedStore = makeRootIdStore("buzz-thread-mentioned.v1");
export const mutedStore = makeRootIdStore("buzz-thread-muted.v1");

function parseTimestamp(value: string | null | undefined) {
  if (!value) {
    return null;
  }

  const timestamp = Date.parse(value);
  return Number.isNaN(timestamp) ? null : timestamp;
}

function toUnixSeconds(isoOrMs: string | null | undefined): number | null {
  const ms = parseTimestamp(isoOrMs);
  return ms === null ? null : Math.floor(ms / 1_000);
}

// Resolve where the read marker should land when a channel is marked read.
// Folds the caller's timeline position together with the newest event this
// client has observed live (`observedLatest`), so an explicit "mark read" still
// covers messages that arrived faster than channel metadata — this fold is
// load-bearing for the Esc shortcut, sidebar mark-read, and empty-channel open,
// all of which pass a null/stale caller value. `clearObserved` reports whether
// the resulting marker covers the observed timestamp, signalling the caller to
// drop its observed refs so the unread memo sees `latest === undefined` until a
// genuinely newer event arrives.
export function resolveChannelReadMarker(
  callerReadAt: string | null | undefined,
  observedLatest: number | undefined,
): { markAt: number | null; clearObserved: boolean } {
  const callerUnix = toUnixSeconds(callerReadAt);
  const markAt = Math.max(callerUnix ?? 0, observedLatest ?? 0) || null;
  return {
    markAt,
    clearObserved:
      markAt !== null &&
      observedLatest !== undefined &&
      observedLatest <= markAt,
  };
}

export function resolveObservedUnreadRootId(tags: string[][]): string | null {
  return isBroadcastReply(tags) ? null : getThreadReference(tags).rootId;
}

/**
 * Pure factory that returns the drain outcome handler function.
 *
 * Extracted from `useDrainOutcomeCallback` so the identical logic can be
 * imported and used in tests without mounting the React hook.  The handler
 * owns outcome routing; the hook owns lifecycle wiring (useEffect / cleanup).
 *
 * Handles outcomes via exhaustive switch on the typed DrainOutcome union:
 *  - `genuine-refusal (unread)` → restore forced-unread entry to its pre-mark snapshot
 *  - `applied-unread`           → discard snapshot (intent succeeded)
 *  - `applied-read`             → remove forced-unread entry (or specific source)
 *  - `silent-inactive`          → same source cleanup as applied-read, no toast
 *  - `genuine-refusal (read)`   → toast already fired in drain; no entry mutation
 *
 * Genuine refusal toasts are fired in the drain itself and not repeated here.
 */
export function createDrainOutcomeHandler(
  forcedUnreadRef: { current: ForcedUnreadMap },
  pendingSnapshots: Map<string, ForcedUnreadEntry | undefined>,
  pubkey: string | undefined,
  bumpLatestVersion: () => void,
): (outcome: DrainOutcome) => void {
  return (outcome: DrainOutcome) => {
    switch (outcome.kind) {
      case "genuine-refusal": {
        if (outcome.op !== "unread") break;
        // Roll back the optimistic forced-unread entry to its exact prior state.
        // For same-session rollbacks, use the in-memory snapshot.
        // For post-restart rollbacks, fall back to the persisted priorForcedEntry
        // from the intent (restart-safe rollback, fix for pass-2 finding 3).
        const inMemoryPrior = pendingSnapshots.get(outcome.channelId);
        pendingSnapshots.delete(outcome.channelId);
        // Pick: in-memory snapshot if present (same session); otherwise use
        // persisted priorForcedEntry; otherwise treat as "no prior entry".
        const hasPrior =
          inMemoryPrior !== undefined || outcome.priorForcedEntry !== undefined;
        const prior =
          inMemoryPrior !== undefined
            ? inMemoryPrior
            : outcome.priorForcedEntry;
        if (!hasPrior) {
          // No prior entry existed — remove the whole entry if present.
          if (Object.hasOwn(forcedUnreadRef.current, outcome.channelId)) {
            delete forcedUnreadRef.current[outcome.channelId];
            if (pubkey)
              forcedUnreadStore.write(pubkey, forcedUnreadRef.current);
            bumpLatestVersion();
          }
        } else if (prior === undefined) {
          // Prior was explicitly "no entry" — delete.
          if (Object.hasOwn(forcedUnreadRef.current, outcome.channelId)) {
            delete forcedUnreadRef.current[outcome.channelId];
            if (pubkey)
              forcedUnreadStore.write(pubkey, forcedUnreadRef.current);
            bumpLatestVersion();
          }
        } else {
          // Restore exact prior entry byte-for-byte.
          forcedUnreadRef.current[outcome.channelId] = prior;
          if (pubkey) forcedUnreadStore.write(pubkey, forcedUnreadRef.current);
          bumpLatestVersion();
        }
        break;
      }
      case "applied-unread": {
        pendingSnapshots.delete(outcome.channelId);
        break;
      }
      case "applied-read":
      case "silent-inactive": {
        // Both cases: remove the exact source (or whole entry) from forced store.
        const { channelId, sourceScope } = outcome;
        if (Object.hasOwn(forcedUnreadRef.current, channelId)) {
          if (sourceScope !== undefined) {
            const current = forcedUnreadRef.current[channelId];
            const next = removeForcedUnreadSource(
              current,
              sourceScope as ForcedUnreadSource,
            );
            if (next !== undefined) {
              forcedUnreadRef.current[channelId] = next;
            } else {
              delete forcedUnreadRef.current[channelId];
            }
          } else {
            delete forcedUnreadRef.current[channelId];
          }
          if (pubkey) forcedUnreadStore.write(pubkey, forcedUnreadRef.current);
          bumpLatestVersion();
        }
        break;
      }
      default: {
        // Exhaustiveness check — TypeScript compile error if a DrainOutcome
        // variant is added without updating this switch.
        const _exhaustive: never = outcome;
        void _exhaustive;
      }
    }
  };
}

/**
 * Hook that builds and wires the drain outcome callback into the ReadStateManager
 * via `setOnDrainOutcome`. Creates and owns the pending-unread-snapshots map.
 * Returns the snapshots ref for use in markChannelUnread.
 *
 * Delegates outcome routing to `createDrainOutcomeHandler`; this hook owns
 * only the React lifecycle wiring (useEffect / cleanup).
 */
export function useDrainOutcomeCallback(
  setOnDrainOutcome: (cb: ((outcome: DrainOutcome) => void) | null) => void,
  forcedUnreadRef: React.MutableRefObject<ForcedUnreadMap>,
  pubkey: string | undefined,
  bumpLatestVersion: () => void,
): React.MutableRefObject<Map<string, ForcedUnreadEntry | undefined>> {
  const pendingUnreadSnapshotsRef = React.useRef(
    new Map<string, ForcedUnreadEntry | undefined>(),
  );
  // biome-ignore lint/correctness/useExhaustiveDependencies: stable refs; setOnDrainOutcome is stable
  React.useEffect(() => {
    setOnDrainOutcome(
      createDrainOutcomeHandler(
        forcedUnreadRef,
        pendingUnreadSnapshotsRef.current,
        pubkey,
        bumpLatestVersion,
      ),
    );
    return () => {
      setOnDrainOutcome(null);
    };
  }, [pubkey, setOnDrainOutcome]);
  return pendingUnreadSnapshotsRef;
}
