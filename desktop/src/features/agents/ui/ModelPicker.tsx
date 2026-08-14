import { ChevronDown } from "lucide-react";
import { toast } from "sonner";

import { Spinner } from "@/shared/ui/spinner";
import React from "react";
import { useQueryClient } from "@tanstack/react-query";

import type { AgentModelsResponse, ManagedAgent } from "@/shared/api/types";
import { getAgentModels, updateManagedAgent } from "@/shared/api/tauri";
import { switchManagedAgentModel } from "@/shared/api/agentControl";
import { awaitLiveSwitchOutcome } from "@/features/agents/lib/liveSwitchOutcome";
import { subscribeControlResults } from "@/features/agents/observerRelayStore";
import { useActiveAgentTurns } from "@/features/agents/activeAgentTurnsStore";
import {
  useAgentConfigSurface,
  managedAgentsQueryKey,
} from "@/features/agents/hooks";
import { Button } from "@/shared/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";

export function ModelPicker({
  agent,
  onModelChanged,
}: {
  agent: ManagedAgent;
  onModelChanged?: () => void;
}) {
  const [modelsData, setModelsData] =
    React.useState<AgentModelsResponse | null>(null);
  const [loading, setLoading] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  const [saving, setSaving] = React.useState(false);
  const [needsRestart, setNeedsRestart] = React.useState(false);
  const [hasRequestedModels, setHasRequestedModels] = React.useState(false);

  const { data: configSurface } = useAgentConfigSurface(agent.pubkey);
  const queryClient = useQueryClient();

  const isRunning = agent.status === "running" || agent.status === "deployed";
  const activeTurns = useActiveAgentTurns(agent.pubkey);
  // A live switch rides the agent's running session(s) instead of persisting a
  // new default. It applies only to a persona-linked running agent with at
  // least one active turn — those are the channels the desktop can name in the
  // `switch_model` frame (the ModelPicker has no other channel context). The
  // harness then routes each named channel itself: a channel still mid-turn
  // cancel-switch-requeues; one that finished between send and receipt takes
  // the idle invalidate-and-reapply path. A persona-linked agent that is
  // running but wholly idle has no nameable channel here, so it falls through
  // to persisting the default (the only reachable lever from this surface).
  const isLiveSwitch =
    agent.personaId !== null && isRunning && activeTurns.length > 0;

  const fetchModels = React.useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const data = await getAgentModels(agent.pubkey);
      setModelsData(data);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setLoading(false);
    }
  }, [agent.pubkey]);

  const handleOpenChange = React.useCallback(
    (open: boolean) => {
      if (!open || loading || modelsData) {
        return;
      }

      setHasRequestedModels(true);
      void fetchModels();
    },
    [fetchModels, loading, modelsData],
  );

  const currentValue = agent.model ?? modelsData?.agentDefaultModel ?? "";
  const displayLabel =
    agent.model ??
    (modelsData?.agentDefaultModel
      ? `${modelsData.agentDefaultModel} (default)`
      : hasRequestedModels && loading
        ? "Loading..."
        : "Auto");

  // Provenance label shown only for post-spawn agents where the model origin
  // is known from the config surface and the source is not a user-explicit
  // Buzz setting (which is already self-evident from the picker state).
  const modelOriginLabel = React.useMemo(() => {
    const origin = configSurface?.normalized.model?.origin;
    if (!origin || origin === "buzzExplicit") return null;
    const labels: Record<string, string> = {
      acpNativeRead: "from ACP",
      acpConfigOption: "from ACP config",
      envVar: "from env",
      configFile: "from config file",
      personaDefault: "template default",
      runtimeOverride: "live override",
    };
    return labels[origin] ?? null;
  }, [configSurface]);

  // Send a live `switch_model` frame to each channel the agent is working in
  // and wait for the harness to acknowledge. Any single `unsupported_model`
  // result rejects the whole pick immediately; all other statuses must arrive
  // from every channel before resolving success.
  const sendLiveSwitch = React.useCallback(
    (modelId: string) => {
      const channelIds = activeTurns.map((turn) => turn.channelId);
      return awaitLiveSwitchOutcome({
        channelCount: channelIds.length,
        modelId,
        subscribe: (listener) =>
          subscribeControlResults(agent.pubkey, listener),
        sendSwitches: async () => {
          await Promise.all(
            channelIds.map((channelId) =>
              switchManagedAgentModel(agent.pubkey, channelId, modelId),
            ),
          );
        },
        // No reply in time: treat as sent. The override still rides the
        // requeued/next session; we just can't confirm synchronously.
        scheduleTimeout: (onTimeout) => {
          const timeout = window.setTimeout(onTimeout, 8_000);
          return () => window.clearTimeout(timeout);
        },
      });
    },
    [activeTurns, agent.pubkey],
  );

  const handleModelChange = async (modelId: string) => {
    setSaving(true);
    setError(null);
    try {
      if (isLiveSwitch) {
        const outcome = await sendLiveSwitch(modelId);
        if (outcome === "unsupported") {
          toast.error("That model isn't available for this agent.");
          return;
        }
        toast.success("Model switched for this session.");
        onModelChanged?.();
        return;
      }

      // Non-live path (idle, stopped, or non-persona): persist the default.
      await updateManagedAgent({
        pubkey: agent.pubkey,
        model: modelId === modelsData?.agentDefaultModel ? null : modelId,
      });
      void queryClient.invalidateQueries({ queryKey: managedAgentsQueryKey });
      if (isRunning) {
        setNeedsRestart(true);
      }
      onModelChanged?.();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setSaving(false);
    }
  };

  return (
    <span className="inline-flex items-center gap-1.5">
      <DropdownMenu modal={false} onOpenChange={handleOpenChange}>
        <DropdownMenuTrigger asChild>
          <Button
            className="h-7 max-w-full justify-start gap-1.5 rounded-full border border-border/50 bg-muted/45 px-2.5 text-xs font-medium text-foreground shadow-none hover:bg-muted/70"
            disabled={saving}
            size="sm"
            type="button"
            variant="ghost"
          >
            <span className="truncate">{displayLabel}</span>
            {modelOriginLabel ? (
              <span className="shrink-0 text-2xs text-muted-foreground/70">
                ({modelOriginLabel})
              </span>
            ) : null}
            <ChevronDown className="h-4 w-4 text-muted-foreground" />
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent
          align="start"
          className="max-h-64 min-w-48 overflow-y-auto"
          onCloseAutoFocus={(event) => event.preventDefault()}
        >
          {loading ? (
            <div className="flex items-center gap-2 px-3 py-2 text-sm text-muted-foreground">
              <Spinner className="h-4 w-4 border-2" />
              Loading models...
            </div>
          ) : error ? (
            <div className="space-y-2 px-3 py-2 text-sm">
              <p className="text-destructive">Failed to load models.</p>
              <button
                className="text-xs text-muted-foreground underline underline-offset-2 hover:text-foreground"
                onClick={() => {
                  setHasRequestedModels(true);
                  void fetchModels();
                }}
                type="button"
              >
                Retry
              </button>
            </div>
          ) : !modelsData ? (
            <div className="px-3 py-2 text-sm text-muted-foreground">
              Open to load available models.
            </div>
          ) : !modelsData.supportsSwitching ? (
            <div className="px-3 py-2 text-sm text-muted-foreground">
              {agent.model ? (
                <>
                  <p className="font-medium text-foreground">{agent.model}</p>
                  <p className="mt-0.5 text-xs">
                    This runtime does not support switching models.
                  </p>
                </>
              ) : (
                "This agent uses the runtime's default model."
              )}
            </div>
          ) : (
            <DropdownMenuRadioGroup
              onValueChange={handleModelChange}
              value={currentValue}
            >
              {modelsData.models.map((model) => (
                <DropdownMenuRadioItem key={model.id} value={model.id}>
                  {model.name ?? model.id}
                </DropdownMenuRadioItem>
              ))}
            </DropdownMenuRadioGroup>
          )}
        </DropdownMenuContent>
      </DropdownMenu>
      {needsRestart ? (
        <span className="text-2xs text-warning">restart to apply</span>
      ) : null}
    </span>
  );
}
