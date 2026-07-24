import * as React from "react";

import type { ManagedAgentBackend } from "@/shared/api/types";

import {
  backendIntentToManagedAgentBackend,
  resolveBackendIntent,
  type WhereToRunDraft,
} from "./whereToRunIntent";

function draftForBackend(backend: ManagedAgentBackend): WhereToRunDraft {
  if (backend.type === "local") {
    return {
      runOn: "local" as const,
      providerConfig: {},
      probedProvider: null,
    };
  }
  return {
    runOn: backend.id,
    providerConfig: Object.fromEntries(
      Object.entries(backend.config).map(([key, value]) => [
        key,
        String(value),
      ]),
    ),
    probedProvider: null,
    allowUnprobedProvider: true,
  };
}

export function useAgentBackendEdit(backend: ManagedAgentBackend) {
  const [draft, setDraft] = React.useState<WhereToRunDraft>(() =>
    draftForBackend(backend),
  );
  const reset = React.useCallback(
    () => setDraft(draftForBackend(backend)),
    [backend],
  );
  const selected = backendIntentToManagedAgentBackend(
    resolveBackendIntent(draft),
  );
  return {
    draft,
    setDraft,
    reset,
    update:
      JSON.stringify(selected) === JSON.stringify(backend)
        ? undefined
        : selected,
  };
}
