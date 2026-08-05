import * as React from "react";

import type { AcpRuntimeCatalogEntry } from "@/shared/api/types";
import { useBakedBuildEnvQuery } from "../hooks";
import { useGlobalAgentConfig } from "../useGlobalAgentConfig";
import { getInheritedAgentDefaults } from "./bakedEnvHelpers";

export function useAgentDialogDefaults({
  inheritedEnvVars = {},
  open,
  runtime,
  nativeEffortKey,
  acceptedEffortValues,
  effortAliases,
}: {
  inheritedEnvVars?: Record<string, string>;
  open: boolean;
  /**
   * The selected runtime catalog entry. When provided, `nativeEffortKey`,
   * `acceptedEffortValues`, and `effortAliases` are derived from it.
   * Callers may still pass the three fields individually for backward compat.
   */
  runtime?: Pick<
    AcpRuntimeCatalogEntry,
    "thinkingEnvVar" | "acceptedEffortValues" | "effortAliases"
  > | null;
  /**
   * The native effort key for the current runtime
   * (e.g. `runtime.thinkingEnvVar`). Ignored when `runtime` is supplied.
   */
  nativeEffortKey?: string | null;
  /**
   * Canonical effort values for normalization (from `runtime.acceptedEffortValues`).
   * Ignored when `runtime` is supplied. Null for buzz-agent.
   */
  acceptedEffortValues?: readonly string[] | null;
  /**
   * Effort alias pairs from `runtime.effortAliases` — the single normalization authority.
   * Ignored when `runtime` is supplied.
   * Absent means no aliases; the fallback table has been removed.
   */
  effortAliases?: ReadonlyArray<readonly [string, string]> | null;
}) {
  const resolvedNativeEffortKey =
    runtime !== undefined ? runtime?.thinkingEnvVar : nativeEffortKey;
  const resolvedAcceptedEffortValues =
    runtime !== undefined
      ? (runtime?.acceptedEffortValues ?? null)
      : (acceptedEffortValues ?? null);
  const resolvedEffortAliases =
    runtime !== undefined
      ? (runtime?.effortAliases ?? null)
      : (effortAliases ?? null);
  const { globalConfig } = useGlobalAgentConfig();
  const { data: bakedEnv } = useBakedBuildEnvQuery({ enabled: open });
  const inheritedDefaults = getInheritedAgentDefaults(
    globalConfig,
    bakedEnv,
    resolvedNativeEffortKey
      ? {
          nativeEffortKey: resolvedNativeEffortKey,
          acceptedEffortValues: resolvedAcceptedEffortValues,
          effortAliases: resolvedEffortAliases,
        }
      : undefined,
  );
  const resolvedEffortKey =
    resolvedNativeEffortKey ?? "BUZZ_AGENT_THINKING_EFFORT";
  const effectiveInheritedEnvVars = React.useMemo(
    () => ({
      ...globalConfig.env_vars,
      ...inheritedEnvVars,
      ...(inheritedDefaults.effort.value
        ? { [resolvedEffortKey]: inheritedDefaults.effort.value }
        : {}),
    }),
    [
      globalConfig.env_vars,
      inheritedDefaults.effort.value,
      inheritedEnvVars,
      resolvedEffortKey,
    ],
  );
  return {
    globalConfig,
    inheritedDefaults,
    inheritedEnvVars: effectiveInheritedEnvVars,
  };
}
