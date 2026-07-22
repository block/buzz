import type {
  AcpRuntime,
  AcpRuntimeCatalogEntry,
  GlobalAgentConfig,
} from "@/shared/api/types";
import {
  buildCustomAcpRuntime,
  isCustomRuntimeId,
  resolvePreferredCustomRuntime,
} from "./customHarness";

/**
 * Select the best default runtime from a catalog, using the same preference
 * order as the UI picker: buzz-agent first (bundled sidecar), then goose,
 * then the first available entry, then null when nothing is available.
 *
 * Generic so that passing AcpRuntime[] (the already-filtered start-path
 * list) returns AcpRuntime | null while passing AcpRuntimeCatalogEntry[]
 * (the full catalog) returns AcpRuntimeCatalogEntry | null.  Both call sites
 * share one preference-order implementation.
 *
 * When `preferredRuntimeId` is `"custom"`, returns null — callers that need
 * BYO resolution should use [`resolvePreferredHarness`].
 */
export function getDefaultPersonaRuntime<T extends AcpRuntimeCatalogEntry>(
  runtimes: readonly T[],
  preferredRuntimeId?: string | null,
): T | null {
  if (isCustomRuntimeId(preferredRuntimeId)) {
    return null;
  }
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
 * Resolve the user's preferred harness from global config, including a
 * bring-your-own ACP command outside the Rust catalog.
 */
export function resolvePreferredHarness(
  runtimes: readonly AcpRuntimeCatalogEntry[],
  config: Pick<
    GlobalAgentConfig,
    "preferred_runtime" | "preferred_agent_command" | "preferred_agent_args"
  >,
): AcpRuntime | null {
  const custom = resolvePreferredCustomRuntime(config);
  if (custom) return custom;
  const available = runtimes.filter(
    (runtime): runtime is AcpRuntime => runtime.availability === "available",
  );
  // Incomplete BYO (preferred_runtime custom, empty command) must not poison
  // catalog fallback — getDefaultPersonaRuntime returns null for "custom".
  return getDefaultPersonaRuntime(
    available,
    isCustomRuntimeId(config.preferred_runtime)
      ? null
      : config.preferred_runtime,
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
 */
export function resolvePersonaRuntime(
  personaRuntimeId: string | undefined | null,
  runtimes: readonly AcpRuntime[],
  defaultRuntime: AcpRuntime | null,
  forceOverride?: boolean,
): ResolvePersonaRuntimeResult {
  // Case 1: Persona has no runtime preference — use the default.
  if (!personaRuntimeId) {
    return {
      runtime: defaultRuntime,
      warnings: defaultRuntime
        ? []
        : [
            "No agent runtimes are available. Install a runtime or bring your own ACP command to deploy agents.",
          ],
      isOverridden: false,
    };
  }

  // Case 1b: Persona explicitly pins a custom/BYO command.
  if (isCustomRuntimeId(personaRuntimeId)) {
    if (defaultRuntime && isCustomRuntimeId(defaultRuntime.id)) {
      return {
        runtime: defaultRuntime,
        warnings: [],
        isOverridden: false,
      };
    }
    if (defaultRuntime) {
      return {
        runtime: defaultRuntime,
        warnings: [
          `This agent is configured for a custom harness but none is set globally. Using ${defaultRuntime.label} instead.`,
        ],
        isOverridden: true,
      };
    }
    return {
      runtime: null,
      warnings: [
        "This agent is configured for a custom harness, but no custom command is configured.",
      ],
      isOverridden: false,
    };
  }

  // Case 2: Persona's preferred runtime is available.
  const matched = runtimes.find((p) => p.id === personaRuntimeId);
  if (matched) {
    if (forceOverride && defaultRuntime && matched.id !== defaultRuntime.id) {
      return {
        runtime: defaultRuntime,
        warnings: [
          `Runtime override: using ${defaultRuntime.label} instead of ${matched.label}.`,
        ],
        isOverridden: true,
      };
    }
    return {
      runtime: forceOverride && defaultRuntime ? defaultRuntime : matched,
      warnings: [],
      isOverridden: false,
    };
  }

  // Case 3 & 4: Persona's runtime is not available — fall back.
  if (defaultRuntime) {
    return {
      runtime: defaultRuntime,
      warnings: [
        `This agent is configured for runtime "${personaRuntimeId}" but it is not available. Using ${defaultRuntime.label} instead.`,
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

export { buildCustomAcpRuntime, isCustomRuntimeId };
