/**
 * First-run seeding for observer-feed archive.
 *
 * When an internal build has `BUZZ_BUILD_OBSERVER_ARCHIVE_DEFAULT` set and the
 * current identity has not yet made an explicit choice, this hook auto-creates
 * an `owner_p` save subscription (kind 24200 observer frames, scoped to the
 * current identity's pubkey).
 *
 * OSS builds return `false` from `observer_archive_default_enabled` → no-op.
 * After any explicit user action (seeding or opt-out), the localStorage flag
 * prevents re-seeding on subsequent starts.
 */

import * as React from "react";

import { KIND_AGENT_OBSERVER_FRAME } from "@/shared/constants/kinds";
import {
  createSaveSubscription,
  observerArchiveDefaultEnabled,
} from "@/shared/api/tauriArchive";
import {
  hasExplicitObserverArchiceChoice,
  setExplicitObserverArchiveChoice,
} from "./observerArchivePreference";

/**
 * Deps interface for testing.  Production callers pass nothing.
 */
export interface ObserverArchiveSeedDeps {
  observerArchiveDefaultEnabled: () => Promise<boolean>;
  createSaveSubscription: (
    scopeType: "owner_p",
    scopeValue: string,
    kinds: number[],
  ) => Promise<void>;
  hasExplicitChoice: (pubkey: string) => boolean;
  setExplicitChoice: (pubkey: string, enabled: boolean) => void;
}

const defaultDeps: ObserverArchiveSeedDeps = {
  observerArchiveDefaultEnabled,
  createSaveSubscription,
  hasExplicitChoice: hasExplicitObserverArchiceChoice,
  setExplicitChoice: setExplicitObserverArchiveChoice,
};

/**
 * Seed the observer-feed archive subscription for `pubkey` once per identity
 * per device on internal builds.
 *
 * @param pubkey - current identity pubkey.  When undefined (identity not yet
 *   loaded), the hook waits until it becomes available.
 * @param deps - optional dep-injection for tests.
 */
export function useObserverArchiveSeed(
  pubkey: string | undefined,
  deps: ObserverArchiveSeedDeps = defaultDeps,
): void {
  React.useEffect(() => {
    if (!pubkey) return;

    // Already made an explicit choice for this identity — never re-seed.
    if (deps.hasExplicitChoice(pubkey)) return;

    let cancelled = false;

    async function maybeSeed(): Promise<void> {
      // pubkey is checked above but TypeScript doesn't narrow across the async
      // boundary — re-guard here so the call below is type-safe.
      if (!pubkey) return;

      let defaultOn: boolean;
      try {
        defaultOn = await deps.observerArchiveDefaultEnabled();
      } catch (err) {
        console.warn("[useObserverArchiveSeed] flag check failed:", err);
        return;
      }

      if (cancelled) return;

      if (!defaultOn) {
        // OSS build — record the explicit choice as "off" so we never query
        // again, then return.  This way an OSS user who later receives an
        // internal build via some side channel won't get auto-seeded without
        // warning.  Actually — don't persist on OSS: keep null so if the user
        // somehow ends up on an internal build later the seeding can still
        // fire.  Just return without setting.
        return;
      }

      // Internal build + no prior choice → auto-seed.
      try {
        await deps.createSaveSubscription("owner_p", pubkey, [
          KIND_AGENT_OBSERVER_FRAME,
        ]);
      } catch (err) {
        console.warn("[useObserverArchiveSeed] createSaveSubscription failed:", err);
        // Do NOT set the localStorage flag — a transient failure (relay
        // unreachable, archive DB not yet initialized) should retry on next
        // startup rather than permanently suppress seeding.
        return;
      }

      if (cancelled) return;

      // Persist the explicit choice so this never re-fires.
      deps.setExplicitChoice(pubkey, true);
    }

    void maybeSeed();

    return () => {
      cancelled = true;
    };
  }, [pubkey, deps]);
}
