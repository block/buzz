import { cn } from "@/shared/lib/cn";
import { Input } from "@/shared/ui/input";
import { Textarea } from "@/shared/ui/textarea";
import { EnvVarsEditor, type EnvVarsValue } from "./EnvVarsEditor";
import {
  PERSONA_FIELD_CONTROL_CLASS,
  PERSONA_FIELD_SHELL_CLASS,
  PERSONA_LABEL_OPTIONAL_CLASS,
} from "./personaDialogPickers";
import type { AgentPersona } from "@/shared/api/types";

export function EditAgentAdvancedFields({
  acpCommand,
  agentArgs,
  agentCommand,
  disabled,
  envVars,
  fileSatisfiedEnvKeys,
  inheritedEnvVars,
  inheritHarness,
  linkedPersona,
  mcpCommand,
  mcpToolsets,
  parallelism,
  relayUrl,
  requiredEnvKeys,
  selectedRuntimeId,
  systemPrompt,
  turnTimeoutSeconds,
  onAcpCommandChange,
  onAgentArgsChange,
  onAgentCommandChange,
  onEnvVarsChange,
  onInheritHarnessChange,
  onMcpCommandChange,
  onMcpToolsetsChange,
  onParallelismChange,
  onRelayUrlChange,
  onSystemPromptChange,
  onTurnTimeoutChange,
}: {
  acpCommand: string;
  agentArgs: string;
  agentCommand: string;
  disabled: boolean;
  envVars: EnvVarsValue;
  fileSatisfiedEnvKeys: readonly string[];
  inheritedEnvVars: Record<string, string>;
  inheritHarness: boolean;
  linkedPersona: AgentPersona | null;
  mcpCommand: string;
  mcpToolsets: string;
  parallelism: string;
  relayUrl: string;
  requiredEnvKeys: readonly string[];
  selectedRuntimeId: string;
  systemPrompt: string;
  turnTimeoutSeconds: string;
  onAcpCommandChange: (value: string) => void;
  onAgentArgsChange: (value: string) => void;
  onAgentCommandChange: (value: string) => void;
  onEnvVarsChange: (value: EnvVarsValue) => void;
  onInheritHarnessChange: (value: boolean) => void;
  onMcpCommandChange: (value: string) => void;
  onMcpToolsetsChange: (value: string) => void;
  onParallelismChange: (value: string) => void;
  onRelayUrlChange: (value: string) => void;
  onSystemPromptChange: (value: string) => void;
  onTurnTimeoutChange: (value: string) => void;
}) {
  return (
    <div className="space-y-5 pt-2">
      {/* Inherit runtime from template */}
      {linkedPersona ? (
        <div className="space-y-1.5">
          <label
            className="flex items-center gap-2 text-sm font-medium"
            htmlFor="edit-agent-inherit-harness"
          >
            <input
              checked={inheritHarness}
              id="edit-agent-inherit-harness"
              onChange={(event) => onInheritHarnessChange(event.target.checked)}
              type="checkbox"
            />
            Inherit runtime from template
          </label>
          <p className="text-xs text-muted-foreground">
            {inheritHarness
              ? `Uses the ${linkedPersona.displayName} template's runtime${
                  linkedPersona.runtime ? ` (${linkedPersona.runtime})` : ""
                }. Editing the template and respawning propagates the new runtime.`
              : "Pins this agent to a specific runtime command, overriding the template's runtime."}
          </p>
        </div>
      ) : null}

      {/* Custom agent command (when custom runtime) */}
      {selectedRuntimeId === "custom" && !inheritHarness ? (
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

      {/* Agent runtime args */}
      <div className="space-y-1.5">
        <label
          className="text-sm font-medium text-foreground"
          htmlFor="edit-agent-args"
        >
          Agent runtime args
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
            id="edit-agent-args"
            onChange={(event) => onAgentArgsChange(event.target.value)}
            placeholder="Comma-separated"
            value={agentArgs}
          />
        </div>
      </div>

      {/* MCP command */}
      <div className="space-y-1.5">
        <label
          className="text-sm font-medium text-foreground"
          htmlFor="edit-agent-mcp-command"
        >
          MCP command
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
            id="edit-agent-mcp-command"
            onChange={(event) => onMcpCommandChange(event.target.value)}
            placeholder="Optional MCP server command"
            value={mcpCommand}
          />
        </div>
      </div>

      {/* MCP toolsets */}
      <div className="space-y-1.5">
        <label
          className="text-sm font-medium text-foreground"
          htmlFor="edit-agent-mcp-toolsets"
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
            id="edit-agent-mcp-toolsets"
            onChange={(event) => onMcpToolsetsChange(event.target.value)}
            placeholder="default,canvas,forums,dms,media"
            value={mcpToolsets}
          />
        </div>
        <p className="text-xs text-muted-foreground">
          Comma-separated list of toolsets to expose via BUZZ_TOOLSETS.
        </p>
      </div>

      {/* Turn timeout + Parallelism side by side */}
      <div className="grid gap-4 md:grid-cols-2">
        <div className="space-y-1.5">
          <label
            className="text-sm font-medium text-foreground"
            htmlFor="edit-agent-timeout"
          >
            Turn timeout
            <span className={PERSONA_LABEL_OPTIONAL_CLASS}>seconds</span>
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
              id="edit-agent-timeout"
              onChange={(event) => onTurnTimeoutChange(event.target.value)}
              placeholder="300"
              value={turnTimeoutSeconds}
            />
          </div>
        </div>

        <div className="space-y-1.5">
          <label
            className="text-sm font-medium text-foreground"
            htmlFor="edit-agent-parallelism"
          >
            Parallelism
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
              id="edit-agent-parallelism"
              inputMode="numeric"
              onChange={(event) => onParallelismChange(event.target.value)}
              placeholder="1"
              value={parallelism}
            />
          </div>
        </div>
      </div>

      {/* Relay URL */}
      <div className="space-y-1.5">
        <label
          className="text-sm font-medium text-foreground"
          htmlFor="edit-agent-relay-url"
        >
          Relay URL
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
            id="edit-agent-relay-url"
            onChange={(event) => onRelayUrlChange(event.target.value)}
            placeholder="Leave blank to use the workspace relay"
            value={relayUrl}
          />
        </div>
      </div>

      {/* ACP command */}
      <div className="space-y-1.5">
        <label
          className="text-sm font-medium text-foreground"
          htmlFor="edit-agent-acp-command"
        >
          ACP command
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
            id="edit-agent-acp-command"
            onChange={(event) => onAcpCommandChange(event.target.value)}
            value={acpCommand}
          />
        </div>
      </div>

      {/* System prompt override */}
      <div className="space-y-1.5">
        <label
          className="text-sm font-medium text-foreground"
          htmlFor="edit-agent-system-prompt"
        >
          System prompt override
          <span className={PERSONA_LABEL_OPTIONAL_CLASS}>Optional</span>
        </label>
        <div className={PERSONA_FIELD_SHELL_CLASS}>
          <Textarea
            className={cn(
              "min-h-24 resize-y px-3 py-3 leading-5",
              PERSONA_FIELD_CONTROL_CLASS,
            )}
            disabled={disabled}
            id="edit-agent-system-prompt"
            onChange={(event) => onSystemPromptChange(event.target.value)}
            placeholder="Leave blank to send no ACP system prompt"
            value={systemPrompt}
          />
        </div>
      </div>

      {/* Env vars */}
      <EnvVarsEditor
        disabled={disabled}
        fileSatisfiedKeys={fileSatisfiedEnvKeys}
        helperText="Per-agent env vars. Override the template's vars on collision."
        inheritedFrom={inheritedEnvVars}
        inheritedLabel="template"
        onChange={onEnvVarsChange}
        requiredKeys={requiredEnvKeys}
        value={envVars}
      />
    </div>
  );
}
