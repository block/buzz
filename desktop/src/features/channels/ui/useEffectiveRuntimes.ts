import * as React from "react";

import {
  collectRuntimeWarnings,
  resolvePersonaRuntime,
} from "@/features/agents/lib/resolvePersonaRuntime";
import type { AcpRuntime, AgentPersona } from "@/shared/api/types";

type EffectiveRuntimeInfo = { label: string; isOverridden: boolean };

export function useEffectiveRuntimes(
  personas: readonly AgentPersona[],
  selectedPersonas: readonly AgentPersona[],
  providers: readonly AcpRuntime[],
  selectedRuntime: AcpRuntime | null,
  isOverrideActive: boolean,
) {
  const fallback = selectedRuntime ?? providers[0] ?? null;

  const runtimeWarnings = React.useMemo(
    () =>
      collectRuntimeWarnings(
        selectedPersonas,
        providers,
        fallback,
        isOverrideActive,
      ),
    [selectedPersonas, providers, fallback, isOverrideActive],
  );

  const effectiveRuntimes = React.useMemo(() => {
    const map = new Map<string, EffectiveRuntimeInfo>();
    for (const persona of personas) {
      const resolved = resolvePersonaRuntime(
        persona.runtime,
        providers,
        fallback,
        isOverrideActive,
      );
      if (resolved.runtime) {
        map.set(persona.id, {
          label: resolved.runtime.label,
          isOverridden: resolved.isOverridden,
        });
      }
    }
    return map;
  }, [personas, providers, fallback, isOverrideActive]);

  return { runtimeWarnings, effectiveRuntimes };
}
