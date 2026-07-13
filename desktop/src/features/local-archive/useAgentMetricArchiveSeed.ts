/**
 * First-run seeding for agent-turn-metric archive.
 *
 * When an internal build has `BUZZ_BUILD_AGENT_METRIC_ARCHIVE_DEFAULT` set and
 * the current identity has not yet made an explicit choice, this hook
 * auto-creates an `owner_p` save subscription including kind 44200 agent turn
 * metrics, scoped to the current identity's pubkey.
 *
 * Uses `mergeSaveSubscriptionKinds` (atomic DB-side merge) so a concurrently
 * running observer seed (24200) cannot clobber this kind — the union happens
 * under a single SQLite transaction regardless of await ordering.
 *
 * OSS builds return `false` from `agent_metric_archive_default_enabled` →
 * no-op. After any explicit user action (seeding or opt-out), the localStorage
 * flag prevents re-seeding on subsequent starts.
 */

import * as React from "react";

import { KIND_AGENT_TURN_METRIC } from "@/shared/constants/kinds";
import {
  mergeSaveSubscriptionKinds,
  agentMetricArchiveDefaultEnabled,
} from "@/shared/api/tauriArchive";
import {
  hasExplicitAgentMetricArchiveChoice,
  setExplicitAgentMetricArchiveChoice,
} from "./agentMetricArchivePreference";

/**
 * Deps interface for testing.  Production callers pass nothing.
 */
export interface AgentMetricArchiveSeedDeps {
  agentMetricArchiveDefaultEnabled: () => Promise<boolean>;
  mergeSaveSubscriptionKinds: (kind: number) => Promise<void>;
  hasExplicitChoice: (pubkey: string) => boolean;
  setExplicitChoice: (pubkey: string, enabled: boolean) => void;
}

const defaultDeps: AgentMetricArchiveSeedDeps = {
  agentMetricArchiveDefaultEnabled,
  mergeSaveSubscriptionKinds,
  hasExplicitChoice: hasExplicitAgentMetricArchiveChoice,
  setExplicitChoice: setExplicitAgentMetricArchiveChoice,
};

/**
 * Seed the agent-turn-metric archive subscription for `pubkey` once per
 * identity per device on internal builds.
 *
 * @param pubkey - current identity pubkey.  When undefined (identity not yet
 *   loaded), the hook waits until it becomes available.
 * @param deps - optional dep-injection for tests.
 */
export function useAgentMetricArchiveSeed(
  pubkey: string | undefined,
  deps: AgentMetricArchiveSeedDeps = defaultDeps,
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
        defaultOn = await deps.agentMetricArchiveDefaultEnabled();
      } catch (err) {
        console.warn("[useAgentMetricArchiveSeed] flag check failed:", err);
        return;
      }

      if (cancelled) return;

      if (!defaultOn) {
        // OSS build (flag off): don't persist a choice — leave null so seeding
        // can still fire if this identity later runs an internal build.
        return;
      }

      // Internal build + no prior choice → auto-seed via atomic DB merge.
      try {
        await deps.mergeSaveSubscriptionKinds(KIND_AGENT_TURN_METRIC);
      } catch (err) {
        console.warn(
          "[useAgentMetricArchiveSeed] mergeSaveSubscriptionKinds failed:",
          err,
        );
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
