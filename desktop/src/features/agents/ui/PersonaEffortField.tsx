import type {
  AgentModelsResponse,
  GlobalAgentConfig,
} from "@/shared/api/types";
import type { AgentConfigFieldDescriptor } from "../lib/agentConfigCore";
import { effortPersistenceKeys } from "../lib/agentConfigCore";
import { discoveredEffortValues, withEffortValue } from "./discoveredEffort";
import { EffortSelectField } from "./buzzAgentModelTuningFields";

export function PersonaEffortField({
  field,
  envVars,
  globalConfig,
  option,
  disabled,
  onChange,
}: {
  field: Extract<AgentConfigFieldDescriptor, { kind: "effort" }>;
  envVars: Record<string, string>;
  globalConfig: GlobalAgentConfig;
  option: AgentModelsResponse["effortOption"];
  disabled: boolean;
  onChange: (env: Record<string, string>) => void;
}) {
  const keys = effortPersistenceKeys(field);
  return (
    <EffortSelectField
      currentEffort={field.value ?? ""}
      disabled={disabled}
      effortDefault={null}
      effortValid={discoveredEffortValues(option)}
      htmlFor="persona-effort"
      testId="persona-effort"
      inheritedEffort={globalConfig.env_vars[keys[0]]}
      label="Effort"
      onChange={(value) => onChange(withEffortValue(envVars, keys, value))}
      showUnavailableOptions={false}
      useCustomSelect
    />
  );
}
