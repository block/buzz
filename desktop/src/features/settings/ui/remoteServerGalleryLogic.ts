/**
 * Pure logic for the Remote servers gallery (RemoteServersCard).
 *
 * Extracted for deterministic unit-testing — no React, no Tauri, no network —
 * exactly as `harnessGalleryLogic` is for the harness gallery it mirrors.
 */

import { backendProviderLabel } from "@/features/agents/lib/backendProviderLabel";
import type {
  BackendProviderCandidate,
  BackendProviderProbeResult,
} from "@/shared/api/types";

/** What a provider's `info` probe has told us so far. */
export type RemoteServerProbe =
  | { status: "loading" }
  | { status: "ok"; result: BackendProviderProbeResult }
  | { status: "failed"; error: string };

/** One row of the gallery. */
export type RemoteServerEntry = {
  id: string;
  binaryPath: string;
  /** The provider's own `info.name` once probed, else its binary-derived id. */
  label: string;
  /** `info.version`, or `null` while probing / when the provider omits it. */
  version: string | null;
  /** `info.description`, or `null`. */
  description: string | null;
  /**
   * `"probing"` while `info` is in flight, `"ready"` once it answered `ok`,
   * `"unavailable"` when the probe failed or the provider answered `ok: false`.
   *
   * `"ready"` means "this binary answers the provider protocol", NOT "the
   * server is reachable". `info` is the one op that opens no connection (see
   * docs/remote-agents.md) — reachability is a per-host question, and the host
   * is chosen per-agent in the create dialog, so this surface cannot answer it.
   */
  status: "probing" | "ready" | "unavailable";
  /** The probe's failure message, for an `"unavailable"` row. */
  error: string | null;
};

/**
 * What an unanswered `info` reads as.
 *
 * One constant because two different shapes of silence reach it — an explicit
 * `ok: false`, and a query that settles with no response body at all — and one
 * fact spelled two ways reads as two faults.
 */
export const PROVIDER_INFO_UNANSWERED =
  "The provider did not answer its info request.";

/**
 * The part of a probe query this projection reads.
 *
 * Structural rather than react-query's own result type so the projection stays
 * pure: it is the branch that decides whether a row spins, and `pnpm test` is
 * bare `node --test` with no hook infrastructure to reach it through a
 * component.
 */
export type RemoteServerProbeQuery = {
  isPending: boolean;
  error?: unknown;
  data?: BackendProviderProbeResult | null;
};

/**
 * Project each provider's probe query into the gallery's probe map.
 *
 * Every settled query lands somewhere. A query that resolves with no response
 * body — a provider binary that prints bare `null` and exits 0 parses as a
 * successful `Ok(Value::Null)` in `invoke_provider`, and its `ok` lookup is
 * `None` rather than `Some(false)`, so nothing upstream rejects it — must read
 * as a failure rather than fall through: an absent entry is indistinguishable
 * from a probe still in flight (see `remoteServerEntries`), so the row would
 * spin forever with no error, no timeout, and no way to learn the provider is
 * broken.
 */
export function remoteServerProbes(
  providers: readonly BackendProviderCandidate[],
  results: readonly (RemoteServerProbeQuery | undefined)[],
): Record<string, RemoteServerProbe> {
  const probes: Record<string, RemoteServerProbe> = {};
  providers.forEach((provider, index) => {
    const result = results[index];
    if (!result || result.isPending) {
      probes[provider.id] = { status: "loading" };
      return;
    }
    if (result.error) {
      probes[provider.id] = {
        status: "failed",
        error:
          result.error instanceof Error
            ? result.error.message
            : String(result.error),
      };
      return;
    }
    probes[provider.id] = result.data
      ? { status: "ok", result: result.data }
      : { status: "failed", error: PROVIDER_INFO_UNANSWERED };
  });
  return probes;
}

function probeError(probe: RemoteServerProbe | undefined): string | null {
  if (probe?.status === "failed") return probe.error;
  if (probe?.status === "ok" && !probe.result.ok) {
    return PROVIDER_INFO_UNANSWERED;
  }
  return null;
}

/**
 * Project discovered providers plus their probes into gallery rows.
 *
 * Ready-first then alphabetical, mirroring `sortedPresetEntries`: discovery
 * walks `PATH`, so leaving rows in discovery order would let the gallery
 * reshuffle itself between reads.
 */
export function remoteServerEntries(
  providers: readonly BackendProviderCandidate[],
  probes: Readonly<Record<string, RemoteServerProbe>>,
): RemoteServerEntry[] {
  const entries = providers.map((provider): RemoteServerEntry => {
    const probe = probes[provider.id];
    const info = probe?.status === "ok" ? probe.result : undefined;
    const error = probeError(probe);
    return {
      id: provider.id,
      binaryPath: provider.binaryPath,
      label: backendProviderLabel(provider.id, info?.ok ? info.name : null),
      version: (info?.ok && info.version?.trim()) || null,
      description: (info?.ok && info.description?.trim()) || null,
      status: error ? "unavailable" : info?.ok ? "ready" : "probing",
      error,
    };
  });

  return entries.sort((left, right) => {
    const leftReady = left.status === "ready" ? 0 : 1;
    const rightReady = right.status === "ready" ? 0 : 1;
    if (leftReady !== rightReady) return leftReady - rightReady;
    return left.label.localeCompare(right.label);
  });
}
