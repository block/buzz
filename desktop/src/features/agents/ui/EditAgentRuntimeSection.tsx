import type { AcpRuntimeCatalogEntry } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import { Input } from "@/shared/ui/input";
import {
  PERSONA_FIELD_CONTROL_CLASS,
  PERSONA_FIELD_SHELL_CLASS,
  type PersonaDropdownOption,
} from "./agentConfigOptions";
import { PersonaDropdownField } from "./PersonaDropdownField";

/**
 * Harness picker for a locally-run agent, plus the custom-command escape hatch.
 *
 * Rendered only for agents this device runs: on a spawner the harness is the
 * host's decision, so the Edit dialog replaces this whole section with the
 * "Runs on ..." banner.
 */
export function EditAgentRuntimeSection({
  agentCommand,
  disabled,
  onAgentCommandChange,
  onRuntimeChange,
  runtimeDropdownOptions,
  runtimeDropdownValue,
  selectedRuntime,
  showCommandField,
}: {
  agentCommand: string;
  disabled: boolean;
  onAgentCommandChange: (next: string) => void;
  onRuntimeChange: (next: string) => void;
  runtimeDropdownOptions: PersonaDropdownOption[];
  runtimeDropdownValue: string;
  selectedRuntime: AcpRuntimeCatalogEntry | undefined;
  showCommandField: boolean;
}) {
  return (
    <>
      <div className="space-y-1.5">
        <label
          className="text-sm font-medium text-foreground"
          htmlFor="edit-agent-runtime"
        >
          Provider
        </label>
        <PersonaDropdownField
          disabled={disabled}
          id="edit-agent-runtime"
          onValueChange={onRuntimeChange}
          options={runtimeDropdownOptions}
          placeholder="Choose a provider"
          value={runtimeDropdownValue}
        />
        {selectedRuntime ? (
          <p className="text-xs text-muted-foreground">
            Detected at{" "}
            <span className="font-medium">
              {selectedRuntime.binaryPath ??
                selectedRuntime.command ??
                selectedRuntime.id}
            </span>
          </p>
        ) : null}
      </div>
      {showCommandField ? (
        <div className="space-y-1.5">
          <label
            className="text-sm font-medium text-foreground"
            htmlFor="edit-agent-command"
          >
            Agent command
          </label>
          <div
            className={cn(
              "flex min-h-11 items-center px-3",
              PERSONA_FIELD_SHELL_CLASS,
            )}
          >
            <Input
              autoCorrect="off"
              className={cn(
                "h-8 px-0 py-0 leading-6",
                PERSONA_FIELD_CONTROL_CLASS,
              )}
              disabled={disabled}
              id="edit-agent-command"
              onChange={(event) => onAgentCommandChange(event.target.value)}
              placeholder="Full path or shell command"
              value={agentCommand}
            />
          </div>
        </div>
      ) : null}
    </>
  );
}
