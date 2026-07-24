import type { AcpRuntime, AcpRuntimeCatalogEntry } from "@/shared/api/types";
import {
  filterEnabledAcpRuntimes,
  getDisabledAcpRuntimeIdsSnapshot,
} from "./runtimeVisibilityPreference";

/**
 * Select the best default runtime from a catalog, using the same preference
 * order as the UI picker: buzz-agent first (bundled sidecar), then goose,
 * then the first available entry, then null when nothing is available.
 *
 * Generic so that passing AcpRuntime[] (the already-filtered start-path
 * list) returns AcpRuntime | null while passing AcpRuntimeCatalogEntry[]
 * (the full catalog) returns AcpRuntimeCatalogEntry | null.  Both call sites
 * share one preference-order implementation.
 */
export function getDefaultPersonaRuntime<T extends AcpRuntimeCatalogEntry>(
  runtimes: readonly T[],
  preferredRuntimeId?: string | null,
): T | null {
  const available = runtimes.filter(
    (runtime) => runtime.availability === "available",
  );
  return (
    available.find((runtime) => runtime.id === preferredRuntimeId) ??
    available.find((runtime) => runtime.id === "buzz-agent") ??
    available.find((runtime) => runtime.id === "goose") ??
    available[0] ??
    null
  );
}

/**
 * Result of resolving a persona's preferred runtime against the set of
 * currently-available ACP runtimes.
 *
 * `runtime` is the runtime that should be used for deployment.
 * `warnings` contains user-visible messages when the resolved runtime
 * differs from what the persona requested (e.g. the configured runtime
 * was uninstalled) or when no runtime is available at all.
 * `isOverridden` is true when the resolved runtime differs from what the
 * persona originally requested (either via explicit override or fallback).
 */
export type ResolvePersonaRuntimeResult = {
  runtime: AcpRuntime | null;
  warnings: string[];
  isOverridden: boolean;
};

/**
 * Resolve which ACP runtime to use when deploying an agent from a persona.
 *
 * Resolution order:
 * 1. If the persona has no `runtimeId` → use `defaultRuntime`, no warnings.
 * 2. If the persona's `runtimeId` matches an available runtime → use it,
 *    unless `forceOverride` is true and `defaultRuntime` is set, in which case
 *    `defaultRuntime` is used instead (with an info warning if they differ).
 * 3. If the persona's `runtimeId` is set but not found in `runtimes` →
 *    fall back to `defaultRuntime` and emit a warning.
 * 4. If there is no `defaultRuntime` either → return `null` with an error
 *    warning so the UI can block deployment.
 *
 * Hidden runtimes remain eligible when explicitly pinned by a persona. They
 * are removed only from the implicit fallback set, at this shared boundary,
 * so every provisioning surface observes the device visibility preference.
 */
export function resolvePersonaRuntime(
  personaRuntimeId: string | undefined | null,
  runtimes: readonly AcpRuntime[],
  defaultRuntime: AcpRuntime | null,
  forceOverride?: boolean,
  disabledRuntimeIds: readonly string[] = getDisabledAcpRuntimeIdsSnapshot(),
): ResolvePersonaRuntimeResult {
  const implicitDefaultRuntime = forceOverride
    ? defaultRuntime
    : resolveVisibleDefaultRuntime(
        runtimes,
        defaultRuntime,
        disabledRuntimeIds,
      );

  // Case 1: Persona has no runtime preference — use the default.
  if (!personaRuntimeId) {
    return {
      runtime: implicitDefaultRuntime,
      warnings: implicitDefaultRuntime
        ? []
        : [
            "No agent runtimes are available. Install a runtime (e.g. Goose) to deploy agents.",
          ],
      isOverridden: false,
    };
  }

  // Case 2: Persona's preferred runtime is available.
  const matched = runtimes.find((p) => p.id === personaRuntimeId);
  if (matched) {
    if (
      forceOverride &&
      implicitDefaultRuntime &&
      matched.id !== implicitDefaultRuntime.id
    ) {
      return {
        runtime: implicitDefaultRuntime,
        warnings: [
          `Runtime override: using ${implicitDefaultRuntime.label} instead of ${matched.label}.`,
        ],
        isOverridden: true,
      };
    }
    return {
      runtime:
        forceOverride && implicitDefaultRuntime
          ? implicitDefaultRuntime
          : matched,
      warnings: [],
      isOverridden: false,
    };
  }

  // Case 3 & 4: Persona's runtime is not available — fall back.
  if (implicitDefaultRuntime) {
    return {
      runtime: implicitDefaultRuntime,
      warnings: [
        `This agent is configured for runtime "${personaRuntimeId}" but it is not available. Using ${implicitDefaultRuntime.label} instead.`,
      ],
      isOverridden: true,
    };
  }

  return {
    runtime: null,
    warnings: [
      `This agent is configured for runtime "${personaRuntimeId}" but it is not available, and no other runtimes were found.`,
    ],
    isOverridden: false,
  };
}

function resolveVisibleDefaultRuntime(
  runtimes: readonly AcpRuntime[],
  defaultRuntime: AcpRuntime | null,
  disabledRuntimeIds: readonly string[],
): AcpRuntime | null {
  if (!defaultRuntime) return null;

  const visibleRuntimes = filterEnabledAcpRuntimes(
    runtimes,
    disabledRuntimeIds,
  );
  return getDefaultPersonaRuntime(visibleRuntimes, defaultRuntime.id);
}

/**
 * Collect runtime-resolution warnings for a list of personas.
 *
 * Used by deploy dialogs to surface inline alerts when one or more
 * personas reference a runtime that isn't currently available.
 */
export function collectRuntimeWarnings(
  personas: readonly { runtime: string | null }[],
  runtimes: readonly AcpRuntime[],
  fallbackRuntime: AcpRuntime | null,
  forceOverride?: boolean,
): string[] {
  // When no fallback runtime exists, the caller's UI is responsible for
  // showing the global "no runtimes found" state. Per-persona warnings
  // would be redundant noise alongside that.
  if (!fallbackRuntime) return [];
  const warnings: string[] = [];
  for (const persona of personas) {
    const { warnings: w } = resolvePersonaRuntime(
      persona.runtime,
      runtimes,
      fallbackRuntime,
      forceOverride,
    );
    warnings.push(...w);
  }
  return warnings;
}

/** Whether every definition can resolve with the supplied optional fallback. */
export function canResolveAllPersonaRuntimes(
  personas: readonly { runtime: string | null }[],
  runtimes: readonly AcpRuntime[],
  fallbackRuntime: AcpRuntime | null,
  disabledRuntimeIds: readonly string[] = getDisabledAcpRuntimeIdsSnapshot(),
): boolean {
  return personas.every(
    (persona) =>
      resolvePersonaRuntime(
        persona.runtime,
        runtimes,
        fallbackRuntime,
        false,
        disabledRuntimeIds,
      ).runtime !== null,
  );
}
