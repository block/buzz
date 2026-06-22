import type { AcpRuntime, ManagedAgentPrereqs } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import { Input } from "@/shared/ui/input";
import { Textarea } from "@/shared/ui/textarea";
import { describeResolvedCommand } from "./agentUi";

export function CreateAgentBasicsFields({
  name,
  onNameChange,
}: {
  name: string;
  onNameChange: (value: string) => void;
}) {
  return (
    <div className="space-y-1.5">
      <label className="text-sm font-medium" htmlFor="agent-name">
        Agent name
      </label>
      <Input
        aria-describedby="help-agent-name"
        autoCapitalize="none"
        autoComplete="off"
        autoCorrect="off"
        data-testid="agent-name-input"
        id="agent-name"
        onChange={(event) => onNameChange(event.target.value)}
        placeholder="Support bot"
        spellCheck={false}
        value={name}
      />
      <p className="text-xs text-muted-foreground" id="help-agent-name">
        Used as the local label and synced to the agent profile display name
        when the relay accepts the create-time auth.
      </p>
    </div>
  );
}

export function CreateAgentRuntimeField({
  runtimes,
  runtimesLoading,
  selectedRuntime,
  selectedRuntimeId,
  unavailableCount,
  onRuntimeChange,
}: {
  runtimes: AcpRuntime[];
  runtimesLoading: boolean;
  selectedRuntime: AcpRuntime | null;
  selectedRuntimeId: string;
  unavailableCount: number;
  onRuntimeChange: (value: string) => void;
}) {
  return (
    <div className="space-y-1.5">
      <label className="text-sm font-medium" htmlFor="agent-runtime">
        Agent runtime
      </label>
      <select
        className="flex h-9 w-full rounded-md border border-input bg-background px-3 py-2 text-sm shadow-xs"
        id="agent-runtime"
        onChange={(event) => onRuntimeChange(event.target.value)}
        value={selectedRuntimeId}
      >
        {runtimes.map((runtime) => (
          <option key={runtime.id} value={runtime.id}>
            {runtime.label}
          </option>
        ))}
        <option value="custom">Custom command</option>
      </select>
      {selectedRuntime ? (
        <p className="text-xs text-muted-foreground">
          Detected via{" "}
          <span className="font-medium">
            {describeResolvedCommand(
              selectedRuntime.command,
              selectedRuntime.binaryPath,
            )}
          </span>
        </p>
      ) : runtimesLoading ? (
        <p className="text-xs text-muted-foreground">
          Looking for installed ACP runtimes...
        </p>
      ) : (
        <p className="text-xs text-muted-foreground">
          No known ACP runtime was detected. You can still enter a custom
          command in Advanced setup.
        </p>
      )}
      {unavailableCount > 0 ? (
        <p className="text-xs text-muted-foreground">
          {unavailableCount} additional{" "}
          {unavailableCount === 1 ? "runtime" : "runtimes"} available to
          install. Visit Settings &gt; Doctor to set{" "}
          {unavailableCount === 1 ? "it" : "them"} up.
        </p>
      ) : null}
    </div>
  );
}

export function CreateAgentRuntimeFields({
  acpCommand,
  agentArgs,
  agentCommand,
  mcpCommand,
  mcpToolsets,
  parallelism,
  relayUrl,
  selectedRuntimeId,
  systemPrompt,
  turnTimeoutSeconds,
  onAcpCommandChange,
  onAgentArgsChange,
  onAgentCommandChange,
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
  mcpCommand: string;
  mcpToolsets: string;
  parallelism: string;
  relayUrl: string;
  selectedRuntimeId: string;
  systemPrompt: string;
  turnTimeoutSeconds: string;
  onAcpCommandChange: (value: string) => void;
  onAgentArgsChange: (value: string) => void;
  onAgentCommandChange: (value: string) => void;
  onMcpCommandChange: (value: string) => void;
  onMcpToolsetsChange: (value: string) => void;
  onParallelismChange: (value: string) => void;
  onRelayUrlChange: (value: string) => void;
  onSystemPromptChange: (value: string) => void;
  onTurnTimeoutChange: (value: string) => void;
}) {
  return (
    <>
      <div className="grid gap-4 md:grid-cols-2">
        {/* Relay URL pins the agent to a specific relay; blank falls back to
            the active workspace relay. Shown for all agents. */}
        <div className="space-y-1.5">
          <label className="text-sm font-medium" htmlFor="agent-relay-url">
            Relay URL
          </label>
          <Input
            aria-describedby="help-agent-relay-url"
            autoComplete="off"
            id="agent-relay-url"
            onChange={(event) => onRelayUrlChange(event.target.value)}
            placeholder="Leave blank to use the workspace relay"
            value={relayUrl}
          />
          <p
            className="text-xs text-muted-foreground"
            id="help-agent-relay-url"
          >
            WebSocket URL of the relay this agent connects to. Leave blank to
            use the active workspace relay.
          </p>
        </div>

        <div className="space-y-1.5">
          <label className="text-sm font-medium" htmlFor="agent-acp-command">
            ACP command
          </label>
          <Input
            aria-describedby="help-agent-acp-command"
            autoComplete="off"
            id="agent-acp-command"
            onChange={(event) => onAcpCommandChange(event.target.value)}
            value={acpCommand}
          />
          <p
            className="text-xs text-muted-foreground"
            id="help-agent-acp-command"
          >
            The buzz-acp binary path or alias used to launch the ACP harness
            process.
          </p>
        </div>
      </div>

      {selectedRuntimeId === "custom" ? (
        <div className="space-y-1.5">
          <label
            className="text-sm font-medium"
            htmlFor="agent-runtime-command"
          >
            Custom agent runtime command
          </label>
          <Input
            aria-describedby="help-agent-runtime-command"
            autoComplete="off"
            id="agent-runtime-command"
            onChange={(event) => onAgentCommandChange(event.target.value)}
            value={agentCommand}
          />
          <p
            className="text-xs text-muted-foreground"
            id="help-agent-runtime-command"
          >
            Full path or shell command for the agent binary when no known ACP
            runtime was detected.
          </p>
        </div>
      ) : null}

      <div className="grid gap-4 md:grid-cols-[1.5fr,1.2fr,0.8fr,0.8fr]">
        <div className="space-y-1.5">
          <label className="text-sm font-medium" htmlFor="agent-runtime-args">
            Agent runtime args
          </label>
          <Input
            aria-describedby="help-agent-runtime-args"
            autoComplete="off"
            id="agent-runtime-args"
            onChange={(event) => onAgentArgsChange(event.target.value)}
            placeholder="Comma-separated"
            value={agentArgs}
          />
          <p
            className="text-xs text-muted-foreground"
            id="help-agent-runtime-args"
          >
            buzz-acp splits args on commas, matching the testing guide.
          </p>
        </div>

        <div className="space-y-1.5">
          <label className="text-sm font-medium" htmlFor="agent-mcp-command">
            MCP command
          </label>
          <Input
            aria-describedby="help-agent-mcp-command"
            autoComplete="off"
            id="agent-mcp-command"
            onChange={(event) => onMcpCommandChange(event.target.value)}
            value={mcpCommand}
          />
          <p
            className="text-xs text-muted-foreground"
            id="help-agent-mcp-command"
          >
            Optional. Only needed for agents that use a custom MCP server (e.g.
            buzz-agent with buzz-dev-mcp). Leave blank for most agents.
          </p>
        </div>

        <div className="space-y-1.5">
          <label className="text-sm font-medium" htmlFor="agent-timeout">
            Turn timeout
          </label>
          <Input
            aria-describedby="help-agent-timeout"
            autoComplete="off"
            id="agent-timeout"
            onChange={(event) => onTurnTimeoutChange(event.target.value)}
            placeholder="300"
            value={turnTimeoutSeconds}
          />
          <p className="text-xs text-muted-foreground" id="help-agent-timeout">
            Seconds before an agent turn is cancelled. Defaults to 300.
          </p>
        </div>

        <div className="space-y-1.5">
          <label className="text-sm font-medium" htmlFor="agent-parallelism">
            Parallelism
          </label>
          <Input
            aria-describedby="help-agent-parallelism"
            autoComplete="off"
            data-testid="agent-parallelism-input"
            id="agent-parallelism"
            inputMode="numeric"
            max="32"
            min="1"
            onChange={(event) => onParallelismChange(event.target.value)}
            placeholder="1"
            step="1"
            type="number"
            value={parallelism}
          />
          <p
            className="text-xs text-muted-foreground"
            id="help-agent-parallelism"
          >
            Number of ACP worker subprocesses. buzz-acp allows 1-32.
          </p>
        </div>
      </div>

      <div className="space-y-1.5">
        <label className="text-sm font-medium" htmlFor="agent-mcp-toolsets">
          MCP toolsets
        </label>
        <Input
          autoComplete="off"
          id="agent-mcp-toolsets"
          onChange={(event) => onMcpToolsetsChange(event.target.value)}
          placeholder="default,canvas,forums,dms,media"
          value={mcpToolsets}
        />
        <p className="text-xs text-muted-foreground">
          Comma-separated list of toolsets to expose via BUZZ_TOOLSETS.
          Available: default, channel_admin, dms, canvas, workflow_admin,
          identity, forums, social, media. Leave blank for default toolsets
          (default, canvas, forums, dms, media).
        </p>
      </div>

      <div className="space-y-1.5">
        <label className="text-sm font-medium" htmlFor="agent-system-prompt">
          System prompt override
        </label>
        <Textarea
          aria-describedby="help-agent-system-prompt"
          data-testid="agent-system-prompt-input"
          id="agent-system-prompt"
          onChange={(event) => onSystemPromptChange(event.target.value)}
          placeholder="Leave blank to send no ACP system prompt"
          value={systemPrompt}
        />
        <p
          className="text-xs text-muted-foreground"
          id="help-agent-system-prompt"
        >
          Blank means no override. buzz-acp will not add a [System] prompt.
        </p>
      </div>
    </>
  );
}

export function CreateAgentOptionToggles({
  isSpawnSupported,
  prereqs,
  startOnAppLaunch,
  startOnAppLaunchDisabled,
  spawnAfterCreate,
  spawnToggleDisabled,
  onToggleStartOnAppLaunch,
  onToggleSpawnAfterCreate,
}: {
  isSpawnSupported: boolean;
  prereqs: ManagedAgentPrereqs | null;
  startOnAppLaunch: boolean;
  /** When true, the toggle is disabled (e.g. remote agents don't support auto-start). */
  startOnAppLaunchDisabled?: boolean;
  spawnAfterCreate: boolean;
  spawnToggleDisabled: boolean;
  onToggleStartOnAppLaunch: () => void;
  onToggleSpawnAfterCreate: () => void;
}) {
  return (
    <div className="grid gap-3 md:grid-cols-2">
      <button
        aria-pressed={startOnAppLaunch}
        className={cn(
          "rounded-2xl border px-4 py-3 text-left transition-colors",
          startOnAppLaunchDisabled && "cursor-not-allowed opacity-60",
          startOnAppLaunch
            ? "border-primary bg-primary/10"
            : "border-border/70 bg-background/70",
        )}
        disabled={startOnAppLaunchDisabled}
        onClick={onToggleStartOnAppLaunch}
        type="button"
      >
        <p className="text-sm font-semibold tracking-tight">
          Start on app launch
        </p>
        <p className="mt-1 text-sm text-foreground/70">
          {startOnAppLaunchDisabled
            ? "Remote agents are managed by their provider and don\u2019t auto-start with the desktop app."
            : "Reopen this local ACP harness automatically when the desktop app starts."}
        </p>
      </button>

      <button
        aria-pressed={spawnAfterCreate}
        className={cn(
          "rounded-2xl border px-4 py-3 text-left transition-colors",
          spawnToggleDisabled && "cursor-not-allowed opacity-60",
          spawnAfterCreate
            ? "border-primary bg-primary/10"
            : "border-border/70 bg-background/70",
        )}
        disabled={spawnToggleDisabled}
        onClick={onToggleSpawnAfterCreate}
        type="button"
      >
        <p className="text-sm font-semibold tracking-tight">
          Spawn after create
        </p>
        <p className="mt-1 text-sm text-foreground/70">
          {prereqs !== null && !isSpawnSupported
            ? "Requires the ACP harness binary."
            : "Start the local ACP harness immediately after the profile is saved."}
        </p>
      </button>
    </div>
  );
}
