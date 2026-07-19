import { cn } from "@/shared/lib/cn";
import { Input } from "@/shared/ui/input";
import { Textarea } from "@/shared/ui/textarea";
import { EnvVarsEditor, type EnvVarsValue } from "./EnvVarsEditor";
import {
  PERSONA_FIELD_CONTROL_CLASS,
  PERSONA_FIELD_SHELL_CLASS,
  PERSONA_LABEL_OPTIONAL_CLASS,
} from "./agentConfigOptions";
import type { AgentPersona } from "@/shared/api/types";
import { BuzzAgentModelTuningFields } from "./buzzAgentModelTuningFields";
import { isBuzzAgentRuntime } from "./buzzAgentConfig";

export function EditAgentAdvancedFields({
  acpCommand,
  agentArgs,
  agentCommand,
  autoRestartOnConfigChange,
  disabled,
  envVars,
  fileSatisfiedEnvKeys,
  hiddenEnvKeys = [],
  focusKey,
  inheritedEnvVars,
  inheritHarness,
  linkedPersona,
  model,
  modelTuningRuntimeId,
  parallelism,
  provider,
  relayUrl,
  requiredEnvKeys,
  selectedRuntimeId,
  systemPrompt,
  onAcpCommandChange,
  onAgentArgsChange,
  onAgentCommandChange,
  onEnvVarsChange,
  onInheritHarnessChange,
  onParallelismChange,
  onRelayUrlChange,
  onAutoRestartChange,
  onSystemPromptChange,
}: {
  acpCommand: string;
  agentArgs: string;
  agentCommand: string;
  autoRestartOnConfigChange: boolean;
  disabled: boolean;
  envVars: EnvVarsValue;
  fileSatisfiedEnvKeys: readonly string[];
  hiddenEnvKeys?: readonly string[];
  /** When set, EnvVarsEditor scrolls and focuses this key's input on mount. */
  focusKey?: string;
  inheritedEnvVars: Record<string, string>;
  inheritHarness: boolean;
  linkedPersona: AgentPersona | null;
  /** Active LLM model — forwarded to BuzzAgentModelTuningFields for effort filtering. */
  model?: string;
  /**
   * The actual/prospective runtime id used to decide whether to show the
   * buzz-agent model-tuning fields. Uses `prospectiveRuntimeId` from
   * EditAgentDialog — the resolved runtime, not the "inherit"/"custom" sentinel.
   */
  modelTuningRuntimeId: string;
  parallelism: string;
  /** Active LLM provider id — forwarded to BuzzAgentModelTuningFields for effort filtering. */
  provider?: string;
  relayUrl: string;
  requiredEnvKeys: readonly string[];
  selectedRuntimeId: string;
  systemPrompt: string;
  onAcpCommandChange: (value: string) => void;
  onAgentArgsChange: (value: string) => void;
  onAgentCommandChange: (value: string) => void;
  onEnvVarsChange: (value: EnvVarsValue) => void;
  onInheritHarnessChange: (value: boolean) => void;
  onParallelismChange: (value: string) => void;
  onRelayUrlChange: (value: string) => void;
  onAutoRestartChange: (value: boolean) => void;
  onSystemPromptChange: (value: string) => void;
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

      {/* Auto-restart on config change (Chunk F) */}
      <div className="space-y-1.5">
        <label
          className="flex items-center gap-2 text-sm font-medium"
          htmlFor="edit-agent-auto-restart"
        >
          <input
            checked={autoRestartOnConfigChange}
            id="edit-agent-auto-restart"
            onChange={(event) => onAutoRestartChange(event.target.checked)}
            type="checkbox"
          />
          Auto-restart on config change
        </label>
        <p className="text-xs text-muted-foreground">
          {autoRestartOnConfigChange
            ? "Restarts this agent automatically when its configuration changes, once it is idle and connected."
            : "Configuration changes only show the restart badge; restart manually to apply them."}
        </p>
      </div>

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

      {/* Parallelism */}
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
            placeholder="Leave blank to use the community relay"
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
        hiddenKeys={hiddenEnvKeys}
        focusKey={focusKey}
        helperText="Per-agent env vars. Override the template's vars on collision."
        inheritedFrom={inheritedEnvVars}
        inheritedLabel="template / global defaults"
        onChange={onEnvVarsChange}
        requiredKeys={requiredEnvKeys}
        value={envVars}
      />

      {/* Tier-1 buzz-agent model-tuning knobs — only shown for buzz-agent. */}
      {isBuzzAgentRuntime(modelTuningRuntimeId) ? (
        <BuzzAgentModelTuningFields
          envVars={envVars}
          inheritedEnvVars={inheritedEnvVars}
          model={model}
          onEnvVarChange={(key, value) => {
            const next = { ...envVars };
            if (value === "") {
              delete next[key];
            } else {
              next[key] = value;
            }
            onEnvVarsChange(next);
          }}
          provider={provider}
        />
      ) : null}
    </div>
  );
}
