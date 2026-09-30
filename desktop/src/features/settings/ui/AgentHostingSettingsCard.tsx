import { useMutation, useQueryClient } from "@tanstack/react-query";
import {
  deviceAgentPolicyQueryKey,
  saveDeviceAgentPolicy,
  useDeviceAgentPolicy,
} from "@/features/agents/useDeviceAgentPolicy";
import { Button } from "@/shared/ui/button";
import { Switch } from "@/shared/ui/switch";
import { SettingsOptionGroup, SettingsOptionRow } from "./SettingsOptionGroup";

export function AgentHostingSettingsCard() {
  const query = useDeviceAgentPolicy();
  const queryClient = useQueryClient();
  const save = useMutation({
    mutationFn: saveDeviceAgentPolicy,
    onSuccess: (status) =>
      queryClient.setQueryData(deviceAgentPolicyQueryKey, status),
  });
  const status = query.data;
  const error = save.error ?? query.error ?? status?.loadError;
  return (
    <SettingsOptionGroup title="Agent hosting">
      <SettingsOptionRow>
        <div className="min-w-0">
          <label htmlFor="agent-client-only" className="text-sm font-medium">
            Client-only mode
          </label>
          <p className="text-sm text-muted-foreground" data-settings-subcopy>
            Use agents running on another device. This device cannot create,
            start or deploy agents in client-only mode.
          </p>
        </div>
        <Switch
          id="agent-client-only"
          checked={status?.saved.client_only ?? false}
          disabled={!status || save.isPending}
          onCheckedChange={(clientOnly) => {
            if (status)
              save.mutate({ ...status.saved, client_only: clientOnly });
          }}
        />
      </SettingsOptionRow>
      <SettingsOptionRow>
        <div className="min-w-0">
          <label htmlFor="agent-unique-names" className="text-sm font-medium">
            Unique agent names
          </label>
          <p className="text-sm text-muted-foreground" data-settings-subcopy>
            Host new agents here with distinct names. Protected remote agents
            stay on their hosting device. Local agent definitions stay on this
            device; the agents remain available through the relay.
          </p>
        </div>
        <Switch
          id="agent-unique-names"
          checked={status?.saved.unique_names ?? false}
          disabled={!status || save.isPending}
          onCheckedChange={(uniqueNames) => {
            if (status)
              save.mutate({
                ...status.saved,
                unique_names: uniqueNames,
                client_only: uniqueNames ? false : status.saved.client_only,
              });
          }}
        />
      </SettingsOptionRow>
      <div className="space-y-2 px-4 pb-4 text-sm text-muted-foreground">
        {status && status.saved.preferred_agents.length === 0 && (
          <p>
            No existing agent identities are protected. Checking new names does
            not stop existing local agents. Use client-only mode to prevent all
            local starts, or configure protected identities in this device's
            agent policy.
          </p>
        )}
        {status?.activeClientOnly && (
          <p>Client-only mode is active on this device.</p>
        )}
        {status?.activeUniqueNames && !status.activeClientOnly && (
          <p>Unique-name hosting is active on this device.</p>
        )}
        {status?.restartRequired && (
          <p role="status">
            Restart Buzz to apply this change.
            {(status.activeClientOnly || status.activeUniqueNames) &&
            !status.saved.client_only &&
            !status.saved.unique_names
              ? " Enabling hosting resumes any retained local agent changes or deletions. Review this device's agent data before restarting."
              : " Your agents and their identities are preserved."}
          </p>
        )}
        {error != null && (
          <p role="alert">
            {String(error instanceof Error ? error.message : error)}
          </p>
        )}
        {status && status.saved.preferred_agents.length > 0 && (
          <>
            <p>
              Preferred existing agents:{" "}
              {status.saved.preferred_agents
                .map((agent) => agent.name)
                .join(", ")}
              .
            </p>
            {!status.saved.unique_names && !status.activeUniqueNames && (
              <Button
                variant="outline"
                size="sm"
                disabled={save.isPending}
                onClick={() =>
                  save.mutate({ ...status.saved, preferred_agents: [] })
                }
              >
                Show all existing identities
              </Button>
            )}
          </>
        )}
        {query.isError && (
          <Button
            variant="outline"
            size="sm"
            onClick={() => void query.refetch()}
          >
            Retry
          </Button>
        )}
        {status?.loadError && (
          <Button
            variant="outline"
            size="sm"
            disabled={save.isPending}
            onClick={() =>
              save.mutate({ client_only: true, preferred_agents: [] })
            }
          >
            Reset to client-only mode
          </Button>
        )}
      </div>
    </SettingsOptionGroup>
  );
}
