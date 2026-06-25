import { AlertTriangle } from "lucide-react";
import * as React from "react";

import {
  useAvailableAcpRuntimes,
  useCreateChannelManagedAgentsMutation,
} from "@/features/agents/hooks";
import type { CreateChannelManagedAgentsResult } from "@/features/agents/channelAgents";
import {
  emptyResolvedTeamPersonas,
  resolveTeamPersonas,
} from "@/features/agents/lib/teamPersonas";
import {
  collectRuntimeWarnings,
  resolvePersonaRuntime,
} from "@/features/agents/lib/resolvePersonaRuntime";
import { useChannelsQuery } from "@/features/channels/hooks";
import { ProfileAvatar } from "@/features/profile/ui/ProfileAvatar";
import type {
  AgentPersona,
  AgentTeam,
  Channel,
  ChannelRole,
} from "@/shared/api/types";
import { Button } from "@/shared/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";

type AddTeamToChannelDialogProps = {
  team: AgentTeam | null;
  personas: AgentPersona[];
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onDeployed: (
    channel: Channel,
    result: CreateChannelManagedAgentsResult,
  ) => void;
};

export function AddTeamToChannelDialog({
  team,
  personas,
  open,
  onOpenChange,
  onDeployed,
}: AddTeamToChannelDialogProps) {
  const channelsQuery = useChannelsQuery();
  const providersQuery = useAvailableAcpRuntimes();
  const [channelId, setChannelId] = React.useState("");
  const [role, setRole] = React.useState<Exclude<ChannelRole, "owner">>("bot");
  const deployMutation = useCreateChannelManagedAgentsMutation(
    channelId || null,
  );

  const channels = React.useMemo(
    () =>
      (channelsQuery.data ?? []).filter(
        (channel) => channel.channelType !== "dm" && !channel.archivedAt,
      ),
    [channelsQuery.data],
  );

  const providers = providersQuery.data ?? [];
  const defaultProvider = providers[0] ?? null;

  const teamPersonaResolution = React.useMemo(
    () =>
      team ? resolveTeamPersonas(team, personas) : emptyResolvedTeamPersonas(),
    [team, personas],
  );
  const resolved = teamPersonaResolution.resolvedPersonas;
  const missingPersonaCount = teamPersonaResolution.missingPersonaCount;

  // Surface warnings when a persona's preferred runtime is unavailable.
  // This dialog has no runtime selector, so the fallback is always
  // `defaultProvider` (the first available runtime).
  const runtimeWarnings = React.useMemo(
    () => collectRuntimeWarnings(resolved, providers, defaultProvider),
    [resolved, providers, defaultProvider],
  );

  function reset() {
    setChannelId("");
    setRole("bot");
    deployMutation.reset();
  }

  function handleOpenChange(next: boolean) {
    if (!next) {
      reset();
    }
    onOpenChange(next);
  }

  React.useEffect(() => {
    if (!open) {
      return;
    }
    if (!channelId && channels.length > 0) {
      setChannelId(channels[0].id);
    }
  }, [channelId, channels, open]);

  const selectedChannel =
    channels.find((channel) => channel.id === channelId) ?? null;

  async function handleDeploy() {
    if (!team || !selectedChannel || !defaultProvider) {
      return;
    }

    try {
      // Resolve each persona's preferred runtime. This dialog has no
      // runtime selector, so the fallback is `defaultProvider` (first
      // available runtime). Warnings are computed separately via the
      // `runtimeWarnings` memo and rendered as inline alerts above.
      const inputs = resolved.map((persona) => {
        const { runtime: personaRuntime } = resolvePersonaRuntime(
          persona.runtime,
          providers,
          defaultProvider,
        );
        const runtimeToUse = personaRuntime ?? defaultProvider;
        return {
          runtime: {
            id: runtimeToUse.id,
            label: runtimeToUse.label,
            command: runtimeToUse.command,
            defaultArgs: runtimeToUse.defaultArgs,
            mcpCommand: runtimeToUse.mcpCommand,
          },
          name: persona.displayName,
          systemPrompt: persona.systemPrompt,
          avatarUrl: persona.avatarUrl ?? undefined,
          model: persona.model ?? undefined,
          personaId: persona.id,
          role,
        };
      });

      const result = await deployMutation.mutateAsync(inputs);
      onDeployed(selectedChannel, result);
      handleOpenChange(false);
    } catch {
      // React Query stores the error; keep the dialog open.
    }
  }

  return (
    <Dialog onOpenChange={handleOpenChange} open={open}>
      <DialogContent className="max-w-xl overflow-hidden p-0">
        <div className="flex max-h-[85vh] flex-col">
          <DialogHeader className="shrink-0 border-b border-border/60 px-6 py-5 pr-14">
            <DialogTitle>Deploy team to channel</DialogTitle>
            <DialogDescription>
              Create and attach one agent per persona in{" "}
              <strong>{team?.name ?? "this team"}</strong> to the selected
              channel.
            </DialogDescription>
          </DialogHeader>

          <div className="min-h-0 flex-1 space-y-5 overflow-y-auto px-6 py-5">
            {resolved.length > 0 ? (
              <div className="space-y-1.5">
                <span className="text-sm font-medium">
                  Personas ({resolved.length})
                </span>
                <div className="flex flex-wrap gap-2">
                  {resolved.map((persona) => (
                    <div
                      className="flex items-center gap-1.5 rounded-full border border-border/70 bg-muted/30 px-2 py-1"
                      key={persona.id}
                    >
                      <ProfileAvatar
                        avatarUrl={persona.avatarUrl}
                        className="h-5 w-5 text-2xs"
                        label={persona.displayName}
                      />
                      <span className="text-xs font-medium">
                        {persona.displayName}
                      </span>
                    </div>
                  ))}
                </div>
              </div>
            ) : null}

            <div className="space-y-1.5">
              <label className="text-sm font-medium" htmlFor="team-channel-id">
                Channel
              </label>
              <select
                className="flex h-9 w-full rounded-md border border-input bg-background px-3 py-2 text-sm shadow-xs"
                disabled={channels.length === 0 || deployMutation.isPending}
                id="team-channel-id"
                onChange={(event) => setChannelId(event.target.value)}
                value={channelId}
              >
                {channels.length === 0 ? (
                  <option value="">No channels available</option>
                ) : null}
                {channels.map((channel) => (
                  <option key={channel.id} value={channel.id}>
                    {channel.name} · {channel.visibility}
                  </option>
                ))}
              </select>
            </div>

            <div className="space-y-1.5">
              <label
                className="text-sm font-medium"
                htmlFor="team-channel-role"
              >
                Role
              </label>
              <select
                className="flex h-9 w-full rounded-md border border-input bg-background px-3 py-2 text-sm shadow-xs"
                disabled={deployMutation.isPending}
                id="team-channel-role"
                onChange={(event) =>
                  setRole(event.target.value as Exclude<ChannelRole, "owner">)
                }
                value={role}
              >
                <option value="bot">bot</option>
                <option value="member">member</option>
                <option value="guest">guest</option>
                <option value="admin">admin</option>
              </select>
            </div>

            {missingPersonaCount > 0 ? (
              <p className="rounded-2xl border border-destructive/30 bg-destructive/10 px-4 py-3 text-sm text-destructive">
                This team references {missingPersonaCount} persona
                {missingPersonaCount === 1 ? "" : "s"} that{" "}
                {missingPersonaCount === 1 ? "is" : "are"} no longer in My
                Agents. Add them back or edit the team before deploying.
              </p>
            ) : null}

            {!defaultProvider && !providersQuery.isLoading ? (
              <p className="rounded-2xl border border-destructive/30 bg-destructive/10 px-4 py-3 text-sm text-destructive">
                No ACP runtimes found. Make sure an agent runtime (e.g. Goose)
                is installed.
              </p>
            ) : null}

            {runtimeWarnings.length > 0
              ? runtimeWarnings.map((warning) => (
                  <div
                    className="flex gap-3 rounded-2xl border border-warning/30 bg-warning-bg px-4 py-3"
                    key={warning}
                  >
                    <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0 text-warning" />
                    <p className="text-sm text-warning">{warning}</p>
                  </div>
                ))
              : null}

            {channelsQuery.error instanceof Error ? (
              <p className="rounded-2xl border border-destructive/30 bg-destructive/10 px-4 py-3 text-sm text-destructive">
                {channelsQuery.error.message}
              </p>
            ) : null}

            {deployMutation.error instanceof Error ? (
              <p className="rounded-2xl border border-destructive/30 bg-destructive/10 px-4 py-3 text-sm text-destructive">
                {deployMutation.error.message}
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
              disabled={
                !team ||
                !selectedChannel ||
                !defaultProvider ||
                resolved.length === 0 ||
                missingPersonaCount > 0 ||
                channelsQuery.isLoading ||
                providersQuery.isLoading ||
                deployMutation.isPending
              }
              onClick={() => void handleDeploy()}
              size="sm"
              type="button"
            >
              {deployMutation.isPending
                ? "Deploying..."
                : `Deploy ${resolved.length} ${resolved.length === 1 ? "agent" : "agents"}`}
            </Button>
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}
