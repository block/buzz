import * as React from "react";

import { Input } from "@/shared/ui/input";
import {
  SLASH_COMMAND_NAME_PATTERN,
  type TriggerConfig,
} from "./workflowFormTypes";

export function WorkflowSlashCommandFields({
  disabled,
  onUpdate,
  trigger,
}: {
  disabled?: boolean;
  onUpdate: (trigger: TriggerConfig) => void;
  trigger: TriggerConfig;
}) {
  const inputId = React.useId();
  const helpId = `${inputId}-help`;
  const command = trigger.command ?? "";
  const commandValid = SLASH_COMMAND_NAME_PATTERN.test(command);

  return (
    <div className="space-y-1.5">
      <label className="text-xs font-medium text-foreground" htmlFor={inputId}>
        Command
      </label>
      <div className="flex items-center gap-2">
        <span aria-hidden="true" className="text-sm text-muted-foreground">
          /
        </span>
        <Input
          aria-describedby={helpId}
          aria-invalid={!commandValid}
          aria-label="Slash command name"
          autoCapitalize="none"
          autoComplete="off"
          disabled={disabled}
          id={inputId}
          maxLength={64}
          onChange={(event) =>
            onUpdate({ ...trigger, command: event.target.value })
          }
          placeholder="new-task"
          spellCheck={false}
          value={command}
        />
      </div>
      {commandValid ? (
        <p className="text-xs text-muted-foreground" id={helpId}>
          Bare /{command} messages run this workflow; commands after an @mention
          still go to that agent.
        </p>
      ) : (
        <p className="text-xs text-destructive" id={helpId}>
          Use 1–64 lowercase letters or digits with optional internal hyphens.
        </p>
      )}
    </div>
  );
}
