import { Input } from "@/shared/ui/input";
import { cn } from "@/shared/lib/cn";
import { EnvVarsEditor, type EnvVarsValue } from "./EnvVarsEditor";
import { CreateAgentRespondToField } from "./RespondToField";
import type { PersonaBehaviorDraft } from "./personaBehaviorDraft";
import {
  PERSONA_FIELD_CONTROL_CLASS,
  PERSONA_FIELD_SHELL_CLASS,
} from "./personaDialogPickers";

const PERSONA_LABEL_OPTIONAL_CLASS =
  "ml-1 text-xs font-normal text-muted-foreground/50";

export function PersonaAdvancedFields({
  behaviorDraft,
  disabled,
  envVars,
  namePoolText,
  onBehaviorDraftChange,
  onEnvVarsChange,
  onNamePoolTextChange,
  requiredEnvKeys = [],
  fileSatisfiedEnvKeys = [],
}: {
  behaviorDraft: PersonaBehaviorDraft;
  disabled: boolean;
  envVars: EnvVarsValue;
  namePoolText: string;
  onBehaviorDraftChange: (value: PersonaBehaviorDraft) => void;
  onEnvVarsChange: (value: EnvVarsValue) => void;
  onNamePoolTextChange: (value: string) => void;
  requiredEnvKeys?: readonly string[];
  fileSatisfiedEnvKeys?: readonly string[];
}) {
  return (
    <div className="space-y-5 pt-2">
      <CreateAgentRespondToField
        allowlist={behaviorDraft.respondToAllowlist}
        disabled={disabled}
        mode={behaviorDraft.respondTo ?? "owner-only"}
        onAllowlistChange={(allowlist) =>
          onBehaviorDraftChange({
            ...behaviorDraft,
            respondToAllowlist: allowlist,
          })
        }
        onModeChange={(mode) =>
          onBehaviorDraftChange({ ...behaviorDraft, respondTo: mode })
        }
        variant="persona"
      />

      <div className="grid gap-5 sm:grid-cols-2">
        <div className="space-y-1.5">
          <label
            className="text-sm font-medium text-foreground"
            htmlFor="persona-parallelism"
          >
            Parallelism
            <span className={PERSONA_LABEL_OPTIONAL_CLASS}>Optional</span>
          </label>
          <div
            className={cn(
              "flex min-h-11 items-center px-3",
              PERSONA_FIELD_SHELL_CLASS,
            )}
          >
            <Input
              className={cn(
                "h-8 px-0 py-0 leading-6",
                PERSONA_FIELD_CONTROL_CLASS,
              )}
              disabled={disabled}
              id="persona-parallelism"
              inputMode="numeric"
              max={32}
              min={1}
              onChange={(event) =>
                onBehaviorDraftChange({
                  ...behaviorDraft,
                  parallelism: event.target.value,
                })
              }
              placeholder="1"
              type="number"
              value={behaviorDraft.parallelism}
            />
          </div>
          <p className="text-xs text-muted-foreground">
            Concurrent turns each started instance handles (1-32).
          </p>
        </div>

        <div className="space-y-1.5">
          <label
            className="text-sm font-medium text-foreground"
            htmlFor="persona-mcp-toolsets"
          >
            MCP toolsets
            <span className={PERSONA_LABEL_OPTIONAL_CLASS}>Optional</span>
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
              id="persona-mcp-toolsets"
              onChange={(event) =>
                onBehaviorDraftChange({
                  ...behaviorDraft,
                  mcpToolsets: event.target.value,
                })
              }
              placeholder="developer,computercontroller"
              spellCheck={false}
              value={behaviorDraft.mcpToolsets}
            />
          </div>
          <p className="text-xs text-muted-foreground">
            Comma-separated toolset names passed to the MCP server.
          </p>
        </div>
      </div>

      <div className="space-y-1.5">
        <label
          className="text-sm font-medium text-foreground"
          htmlFor="persona-name-pool"
        >
          Instance name pool
          <span className={PERSONA_LABEL_OPTIONAL_CLASS}>Optional</span>
        </label>
        <div
          className={cn(
            "flex min-h-11 items-center px-3",
            PERSONA_FIELD_SHELL_CLASS,
          )}
        >
          <Input
            autoCapitalize="words"
            autoCorrect="off"
            className={cn(
              "h-8 px-0 py-0 leading-6",
              PERSONA_FIELD_CONTROL_CLASS,
            )}
            disabled={disabled}
            id="persona-name-pool"
            onChange={(event) => onNamePoolTextChange(event.target.value)}
            placeholder="Birch, Compass, Ridge, Thistle"
            spellCheck={false}
            value={namePoolText}
          />
        </div>
      </div>

      <EnvVarsEditor
        disabled={disabled}
        fileSatisfiedKeys={fileSatisfiedEnvKeys}
        onChange={onEnvVarsChange}
        requiredKeys={requiredEnvKeys}
        value={envVars}
      />
    </div>
  );
}
