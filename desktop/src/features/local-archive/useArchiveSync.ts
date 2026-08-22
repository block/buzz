import * as React from "react";

import { huddleWindowChannelId } from "@/features/huddle/lib/huddleWindow";
import {
  announceArchiveSyncEpoch,
  nextArchiveSyncLease,
  startArchiveSync,
  stopArchiveSync,
} from "@/shared/api/tauriArchive";

/**
 * This realm's epoch, announced once at first use.
 *
 * Per-realm, NOT per-effect: the epoch's whole job is to order successive
 * realms, so minting one per effect would make it order announcements instead
 * and leave remounts inside a realm racing exactly as before. Memoizing the
 * promise also means a remount cannot overtake its own predecessor's
 * announcement.
 */
let realmEpoch: Promise<number> | null = null;
const START_RETRY_INITIAL_MS = 100;
const START_RETRY_MAX_MS = 2_000;

function isStoppingConflict(err: unknown) {
  const message = err instanceof Error ? err.message : String(err);
  return message.includes("still stopping");
}

function archiveSyncEpoch(): Promise<number> {
  // Clear on failure so a remount can retry; a cached rejection would make one
  // transient IPC error permanent for the life of the realm.
  realmEpoch ??= announceArchiveSyncEpoch().catch((err: unknown) => {
    realmEpoch = null;
    throw err;
  });
  return realmEpoch;
}

/**
 * Starts the native archive sync task once `ready` is true and stops it on
 * unmount.
 *
 * The `ready` gate is load-bearing and cannot move into the backend: kind
 * 24200 is relay-ephemeral, so frames emitted before the listener opens are
 * permanently lost. Observer reconciliation must have seeded kind 24200 into
 * the saved subscription before any listener opens, and only the renderer
 * knows when that finished. The backend task is therefore not self-starting.
 *
 * Everything below this call — subscribing, buffering, batching, archiving —
 * now runs in Rust; the renderer sees no archive traffic at all.
 *
 * Ownership is main-window-only. `AppShell` is the root route, so a companion
 * window (`huddle::window`) mounts this same tree in a second JS realm — and
 * archive sync is app-global work, exactly like the microphone capture the
 * main window already owns. Two concurrent realms cannot be ordered by a
 * newest-wins clock: the companion's unmount cleanup would cancel the live
 * main-window task while the main realm sits there with nothing to re-issue.
 * So secondary realms never announce and never issue lifecycle commands.
 *
 * Within the owning window, `(epoch, lease)` orders everything else: the epoch
 * is minted by Rust and survives renderer reload (a reload resets the JS lease
 * counter but not the backend's mark), and the lease orders effects inside one
 * realm. The epoch must be resolved BEFORE any lifecycle command is sent — an
 * unawaited announcement is just another racing invoke.
 */
export function useArchiveSync(ready: boolean): void {
  React.useEffect(() => {
    if (!ready) return;
    // Companion realms do not participate; see the ownership rule above.
    if (huddleWindowChannelId() !== null) return;

    let stopped = false;

    const started = archiveSyncEpoch()
      .then(async (epoch) => {
        let retryDelayMs = START_RETRY_INITIAL_MS;
        while (!stopped) {
          // `start_archive_sync` records the attempted lease before it can
          // discover that an older task is still stopping. Every retry must
          // therefore outrank the rejected attempt, not reuse its lease.
          const lease = nextArchiveSyncLease();
          try {
            await startArchiveSync(epoch, lease);
            return { epoch, lease };
          } catch (err) {
            console.warn("[useArchiveSync] start_archive_sync failed:", err);
            if (!isStoppingConflict(err)) return null;
          }
          if (stopped) return null;
          await new Promise((resolve) =>
            window.setTimeout(resolve, retryDelayMs),
          );
          retryDelayMs = Math.min(retryDelayMs * 2, START_RETRY_MAX_MS);
        }
        return null;
      })
      .catch((err: unknown) => {
        console.warn("[useArchiveSync] archive sync epoch failed:", err);
        return null;
      });

    return () => {
      stopped = true;
      // Chained off the same promise so the stop cannot overtake its start:
      // the epoch is not knowable until the announcement resolves.
      void started
        .then(async (owner) => {
          if (owner === null) return;
          await stopArchiveSync(owner.epoch, owner.lease);
        })
        .catch((err: unknown) => {
          console.warn("[useArchiveSync] stop_archive_sync failed:", err);
        });
    };
  }, [ready]);
}
