import { useMutation, useQueryClient } from "@tanstack/react-query";

import { agentConfigSurfaceQueryKey } from "@/features/agents/hooks";
import { persistAgentEffortLevel } from "@/shared/api/tauriManagedAgents";
import type { ManagedAgent, RuntimeConfigSurface } from "@/shared/api/types";
import { PERSONA_LABEL_OPTIONAL_CLASS } from "./agentConfigOptions";
import {
  effortPickerState,
  effortSelectionToPersistedValue,
} from "./effortPicker";
import { PersonaDropdownField } from "./PersonaDropdownField";

/**
 * Thinking-effort write control for the edit dialog (B5, v4 direct-write).
 *
 * Local-only by construction: the write calls `persistAgentEffortLevel`, which
 * the Rust command rejects for non-local backends (remote effort is set at
 * deploy time via `policy_env`). So the control renders only for a local
 * backend AND once the adapter has advertised a `thought_level` configId
 * (discovered from the running session — absent pre-first-session and for
 * runtimes/models without effort support). The read-only configured-vs-running
 * two-facts display lives in `AgentConfigPanel`; this is the write control.
 *
 * Direct-write: each selection persists immediately and invalidates the config
 * surface so the panel's canonical tier reflects the new next-spawn value.
 */
export function EffortPickerField({
  agent,
  config,
}: {
  agent: ManagedAgent;
  config: RuntimeConfigSurface | undefined;
}) {
  const queryClient = useQueryClient();
  const mutation = useMutation({
    mutationFn: (level: string | null) =>
      persistAgentEffortLevel(agent.pubkey, level),
    onSuccess: () =>
      queryClient.invalidateQueries({
        queryKey: agentConfigSurfaceQueryKey(agent.pubkey),
      }),
  });
  const { visible, options, selectValue } = effortPickerState({
    backend: agent.backend,
    effortConfigId: config?.effortConfigId,
    effortOptions: config?.effortOptions,
    currentEffort: config?.normalized.thinkingEffort?.value ?? null,
  });

  if (!visible) {
    return null;
  }

  return (
    <div className="space-y-1.5">
      <label
        className="text-sm font-medium text-foreground"
        htmlFor="edit-agent-effort"
      >
        Thinking effort
        <span className={PERSONA_LABEL_OPTIONAL_CLASS}>Optional</span>
      </label>
      <PersonaDropdownField
        disabled={mutation.isPending}
        id="edit-agent-effort"
        onValueChange={(value) =>
          mutation.mutate(effortSelectionToPersistedValue(value))
        }
        options={options}
        placeholder="Adapter default"
        value={selectValue}
      />
      <p className="text-xs text-muted-foreground">
        Applied at the next session start.
      </p>
      {mutation.error instanceof Error ? (
        <p className="text-xs text-destructive">{mutation.error.message}</p>
      ) : null}
    </div>
  );
}
