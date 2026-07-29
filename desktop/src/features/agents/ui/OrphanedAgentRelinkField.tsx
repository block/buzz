import * as React from "react";

import type {
  AgentPersona,
  ManagedAgent,
  UpdateManagedAgentInput,
} from "@/shared/api/types";
import { PersonaDropdownField } from "./PersonaDropdownField";
import type { PersonaDropdownOption } from "./agentConfigOptions";

export function useAgentRelink(
  agent: ManagedAgent,
  open: boolean,
  personas: readonly AgentPersona[],
  onRelink: (inherit: boolean) => void,
) {
  const [personaId, setPersonaId] = React.useState("");

  // biome-ignore lint/correctness/useExhaustiveDependencies: switching agents while the dialog stays open must clear a stale repair target
  React.useEffect(() => {
    if (open) setPersonaId("");
  }, [agent.pubkey, open]);

  const linkedPersona = React.useMemo(() => {
    const effectiveId = personaId || agent.personaId;
    return effectiveId
      ? (personas.find((persona) => persona.id === effectiveId) ?? null)
      : null;
  }, [agent.personaId, personaId, personas]);

  const inputPersonaId: UpdateManagedAgentInput["personaId"] =
    agent.personaOrphaned && personaId ? personaId : undefined;

  const options = React.useMemo<PersonaDropdownOption[]>(
    () =>
      personas
        .filter((persona) => persona.isActive)
        .sort((left, right) =>
          left.displayName.localeCompare(right.displayName),
        )
        .map((persona) => ({
          label: persona.displayName,
          value: persona.id,
        })),
    [personas],
  );

  const renderField = (disabled: boolean) =>
    agent.personaOrphaned ? (
      <div className="space-y-1.5 rounded-xl border border-warning/40 bg-warning/10 p-3">
        <label
          className="text-sm font-medium text-foreground"
          htmlFor="edit-agent-relink-persona"
        >
          Relink agent definition
        </label>
        <p className="text-xs text-muted-foreground">
          This agent&apos;s definition was deleted. Choose an existing
          definition to repair the link without changing the agent&apos;s
          identity.
        </p>
        <PersonaDropdownField
          disabled={disabled || options.length === 0}
          id="edit-agent-relink-persona"
          onValueChange={(value) => {
            setPersonaId(value);
            onRelink(true);
          }}
          options={options}
          placeholder={
            options.length === 0
              ? "No active definitions available"
              : "Choose a definition"
          }
          value={personaId}
        />
      </div>
    ) : null;

  return { inputPersonaId, linkedPersona, renderField };
}
