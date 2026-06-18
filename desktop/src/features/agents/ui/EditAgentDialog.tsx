import * as React from "react";

import {
  usePersonasQuery,
  useUpdateManagedAgentMutation,
} from "@/features/agents/hooks";
import type {
  ManagedAgent,
  RespondToMode,
  UpdateManagedAgentInput,
} from "@/shared/api/types";
import { Button } from "@/shared/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
import {
  CreateAgentBasicsFields,
  CreateAgentRuntimeFields,
} from "./CreateAgentDialogSections";
import { EnvVarsEditor, type EnvVarsValue } from "./EnvVarsEditor";
import { CreateAgentRespondToField } from "./RespondToField";

export function EditAgentDialog({
  agent,
  open,
  onOpenChange,
  onUpdated,
}: {
  agent: ManagedAgent;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onUpdated?: (agent: ManagedAgent) => void;
}) {
  const updateMutation = useUpdateManagedAgentMutation();

  const [name, setName] = React.useState(agent.name);
  const [relayUrl, setRelayUrl] = React.useState(agent.relayUrl);
  const [acpCommand, setAcpCommand] = React.useState(agent.acpCommand);
  const [agentCommand, setAgentCommand] = React.useState(agent.agentCommand);
  const [agentArgs, setAgentArgs] = React.useState(agent.agentArgs.join(","));
  const [mcpCommand, setMcpCommand] = React.useState(agent.mcpCommand);
  const [mcpToolsets, setMcpToolsets] = React.useState(agent.mcpToolsets ?? "");
  const [turnTimeoutSeconds, setTurnTimeoutSeconds] = React.useState(
    String(agent.turnTimeoutSeconds),
  );
  const [parallelism, setParallelism] = React.useState(
    String(agent.parallelism),
  );
  const [systemPrompt, setSystemPrompt] = React.useState(
    agent.systemPrompt ?? "",
  );
  const [envVars, setEnvVars] = React.useState<EnvVarsValue>(agent.envVars);
  const personasQuery = usePersonasQuery();
  const inheritedEnvVars = React.useMemo(() => {
    if (!agent.personaId) return {};
    const persona = personasQuery.data?.find((p) => p.id === agent.personaId);
    return persona?.envVars ?? {};
  }, [agent.personaId, personasQuery.data]);
  const [respondTo, setRespondTo] = React.useState<RespondToMode>(
    agent.respondTo,
  );
  const [respondToAllowlist, setRespondToAllowlist] = React.useState<string[]>(
    agent.respondToAllowlist,
  );

  // Reset form state only when the dialog opens or when switching to a different
  // agent. Omitting the full agent object and its array fields from deps prevents
  // the effect from firing on every 5s background poll (arrays are never
  // reference-equal across renders), which would wipe in-progress user edits.
  // biome-ignore lint/correctness/useExhaustiveDependencies: intentional — including agent fields would re-fire on every 5s poll and wipe edits
  React.useEffect(() => {
    if (open) {
      setName(agent.name);
      setRelayUrl(agent.relayUrl);
      setAcpCommand(agent.acpCommand);
      setAgentCommand(agent.agentCommand);
      setAgentArgs(agent.agentArgs.join(","));
      setMcpCommand(agent.mcpCommand);
      setMcpToolsets(agent.mcpToolsets ?? "");
      setTurnTimeoutSeconds(String(agent.turnTimeoutSeconds));
      setParallelism(String(agent.parallelism));
      setSystemPrompt(agent.systemPrompt ?? "");
      setEnvVars(agent.envVars);
      setRespondTo(agent.respondTo);
      setRespondToAllowlist(agent.respondToAllowlist);
      updateMutation.reset();
    }
  }, [open, agent.pubkey]);

  function handleOpenChange(next: boolean) {
    onOpenChange(next);
  }

  const parallelismValid =
    parallelism.trim() === "" ||
    !Number.isNaN(Number.parseInt(parallelism, 10));
  const timeoutValid =
    turnTimeoutSeconds.trim() === "" ||
    !Number.isNaN(Number.parseInt(turnTimeoutSeconds, 10));
  // Block clearing a previously-set command to empty — sending an empty string
  // for a required command field would cause a runtime failure at spawn.
  const acpCommandValid = !(agent.acpCommand && acpCommand.trim() === "");
  // Allowlist mode requires at least one entry — mirrors the harness's own
  // validation. The backend would reject the request anyway; we block early
  // so the user sees the disabled button instead of a round-tripped error.
  const respondToValid =
    respondTo !== "allowlist" || respondToAllowlist.length > 0;

  const canSubmit =
    name.trim().length > 0 &&
    parallelismValid &&
    timeoutValid &&
    acpCommandValid &&
    respondToValid &&
    !updateMutation.isPending;

  async function handleSubmit() {
    try {
      const parsedParallelism = Number.parseInt(parallelism, 10);
      const parsedTimeout = Number.parseInt(turnTimeoutSeconds, 10);
      const parsedArgs = agentArgs
        .split(",")
        .map((v) => v.trim())
        .filter((v) => v.length > 0);

      const input: UpdateManagedAgentInput = {
        pubkey: agent.pubkey,
        name: name.trim() !== agent.name ? name.trim() : undefined,
        relayUrl:
          relayUrl.trim() !== agent.relayUrl ? relayUrl.trim() : undefined,
        acpCommand:
          acpCommand.trim() !== agent.acpCommand
            ? acpCommand.trim()
            : undefined,
        agentCommand:
          agentCommand.trim() !== agent.agentCommand
            ? agentCommand.trim()
            : undefined,
        agentArgs:
          parsedArgs.join(",") !== agent.agentArgs.join(",")
            ? parsedArgs
            : undefined,
        mcpCommand:
          mcpCommand.trim() !== agent.mcpCommand
            ? mcpCommand.trim()
            : undefined,
        mcpToolsets:
          (mcpToolsets.trim() || null) !== agent.mcpToolsets
            ? mcpToolsets.trim() || null
            : undefined,
        turnTimeoutSeconds:
          parsedTimeout > 0 && parsedTimeout !== agent.turnTimeoutSeconds
            ? parsedTimeout
            : undefined,
        parallelism:
          parsedParallelism > 0 && parsedParallelism !== agent.parallelism
            ? parsedParallelism
            : undefined,
        // Use tri-state: send null to clear, value to set, omit if unchanged.
        systemPrompt:
          (systemPrompt.trim() || null) !== agent.systemPrompt
            ? systemPrompt.trim() || null
            : undefined,
        envVars: envVarsChanged(envVars, agent.envVars) ? envVars : undefined,
        respondTo: respondTo !== agent.respondTo ? respondTo : undefined,
        // The allowlist is preserved across mode toggles in local UI state
        // (so a user can flip away from allowlist and back without losing
        // their entries), but we only send it on the wire when (a) it
        // actually changed, AND (b) the saved mode will need it. Sending
        // an allowlist while switching to a non-allowlist mode would be
        // harmless server-side, but it's noise in the persisted record.
        respondToAllowlist:
          respondTo === "allowlist" &&
          respondToAllowlist.join(",") !== agent.respondToAllowlist.join(",")
            ? respondToAllowlist
            : undefined,
      };

      const result = await updateMutation.mutateAsync(input);
      if (result.profileSyncError) {
        console.warn("Relay profile sync failed:", result.profileSyncError);
      }
      handleOpenChange(false);
      onUpdated?.(result.agent);
    } catch {
      // React Query stores the error; keep dialog open and render it inline.
    }
  }

  return (
    <Dialog onOpenChange={handleOpenChange} open={open}>
      <DialogContent className="max-w-3xl overflow-hidden p-0">
        <div className="flex max-h-[85vh] flex-col">
          <DialogHeader className="shrink-0 border-b border-border/60 px-6 py-5 pr-14">
            <DialogTitle>Edit agent</DialogTitle>
            <DialogDescription>
              Update configuration for{" "}
              <span className="font-medium">{agent.name}</span>. Changes take
              effect on the next start.
            </DialogDescription>
          </DialogHeader>

          <div className="min-h-0 flex-1 space-y-5 overflow-y-auto px-6 py-5">
            <CreateAgentBasicsFields name={name} onNameChange={setName} />

            <CreateAgentRespondToField
              allowlist={respondToAllowlist}
              mode={respondTo}
              onAllowlistChange={setRespondToAllowlist}
              onModeChange={setRespondTo}
            />

            <CreateAgentRuntimeFields
              acpCommand={acpCommand}
              agentArgs={agentArgs}
              agentCommand={agentCommand}
              mcpCommand={mcpCommand}
              mcpToolsets={mcpToolsets}
              onAcpCommandChange={setAcpCommand}
              onAgentArgsChange={setAgentArgs}
              onAgentCommandChange={setAgentCommand}
              onMcpCommandChange={setMcpCommand}
              onMcpToolsetsChange={setMcpToolsets}
              onParallelismChange={setParallelism}
              onRelayUrlChange={setRelayUrl}
              onSystemPromptChange={setSystemPrompt}
              onTurnTimeoutChange={setTurnTimeoutSeconds}
              parallelism={parallelism}
              relayUrl={relayUrl}
              selectedRuntimeId="custom"
              systemPrompt={systemPrompt}
              turnTimeoutSeconds={turnTimeoutSeconds}
            />

            <EnvVarsEditor
              disabled={updateMutation.isPending}
              helperText="Per-agent env vars. Override the persona's vars on collision."
              inheritedFrom={inheritedEnvVars}
              inheritedLabel="persona"
              onChange={setEnvVars}
              value={envVars}
            />

            {updateMutation.error instanceof Error ? (
              <p className="rounded-2xl border border-destructive/30 bg-destructive/10 px-4 py-3 text-sm text-destructive">
                {updateMutation.error.message}
              </p>
            ) : null}
          </div>

          <div className="flex shrink-0 justify-end gap-2 border-t border-border/60 px-6 py-4">
            <Button
              onClick={() => handleOpenChange(false)}
              size="sm"
              type="button"
              variant="outline"
            >
              Cancel
            </Button>
            <Button
              disabled={!canSubmit}
              onClick={() => void handleSubmit()}
              size="sm"
              type="button"
            >
              {updateMutation.isPending ? "Saving..." : "Save changes"}
            </Button>
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}

function envVarsChanged(
  a: Record<string, string>,
  b: Record<string, string>,
): boolean {
  const aKeys = Object.keys(a);
  const bKeys = Object.keys(b);
  if (aKeys.length !== bKeys.length) return true;
  for (const k of aKeys) {
    if (a[k] !== b[k]) return true;
  }
  return false;
}
