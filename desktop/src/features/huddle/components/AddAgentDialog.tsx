import { LoaderCircle } from "lucide-react";
import * as React from "react";

import { useRelayAgentsQuery } from "@/features/agents/hooks";
import { getSharedChannelIds } from "@/features/agents/lib/agentAutocompleteEligibility";
import { availableRelayAgents } from "@/features/agents/lib/availableRelayAgents";
import { useChannelsQuery } from "@/features/channels/hooks";
import { useUsersBatchQuery } from "@/features/profile/hooks";
import { ProfileAvatar } from "@/features/profile/ui/ProfileAvatar";
import { useIdentityQuery } from "@/shared/api/hooks";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { ChooserDialogContent } from "@/shared/ui/chooser-dialog-content";
import { Dialog } from "@/shared/ui/dialog";

type AgentAddResult = {
  ephemeral_added: boolean;
  parent_added: boolean;
  parent_error: string | null;
};

type AddAgentDialogProps = {
  open: boolean;
  onClose: () => void;
  onAdd: (pubkey: string) => Promise<AgentAddResult>;
  currentAgentPubkeys: string[];
};

export function AddAgentDialog({
  open,
  onClose,
  onAdd,
  currentAgentPubkeys,
}: AddAgentDialogProps) {
  const identityQuery = useIdentityQuery();
  const channelsQuery = useChannelsQuery({ enabled: open });
  const relayAgentsQuery = useRelayAgentsQuery({ enabled: open });
  const [adding, setAdding] = React.useState<string | null>(null);
  const [error, setError] = React.useState<string | null>(null);
  const [warning, setWarning] = React.useState<string | null>(null);

  React.useEffect(() => {
    if (!open) return;
    setAdding(null);
    setError(null);
    setWarning(null);
  }, [open]);

  const sharedChannelIds = React.useMemo(
    () => getSharedChannelIds(channelsQuery.data),
    [channelsQuery.data],
  );
  const agents = React.useMemo(
    () =>
      availableRelayAgents(
        relayAgentsQuery.data,
        sharedChannelIds,
        identityQuery.data?.pubkey,
      ),
    [identityQuery.data?.pubkey, relayAgentsQuery.data, sharedChannelIds],
  );
  const profilesQuery = useUsersBatchQuery(
    agents.map(({ pubkey }) => pubkey),
    { enabled: open && agents.length > 0 },
  );
  const currentAgentSet = React.useMemo(
    () => new Set(currentAgentPubkeys.map(normalizePubkey)),
    [currentAgentPubkeys],
  );
  const loading =
    identityQuery.isPending ||
    channelsQuery.isPending ||
    relayAgentsQuery.isPending ||
    (agents.length > 0 && profilesQuery.isPending);

  async function handleAdd(pubkey: string) {
    if (adding || currentAgentSet.has(normalizePubkey(pubkey))) return;
    setAdding(pubkey);
    setError(null);
    setWarning(null);
    try {
      const result = await onAdd(pubkey);
      if (result.parent_error) {
        setWarning(
          `Added to huddle, but parent channel failed: ${result.parent_error}`,
        );
      } else {
        onClose();
      }
    } catch (cause: unknown) {
      const message = cause instanceof Error ? cause.message : String(cause);
      setError(`Failed to add agent: ${message}`);
      console.error("Failed to add relay agent to huddle:", cause);
    } finally {
      setAdding(null);
    }
  }

  return (
    <Dialog
      onOpenChange={(nextOpen) => {
        if (!nextOpen) onClose();
      }}
      open={open}
    >
      <ChooserDialogContent
        className="max-w-xl"
        data-testid="add-huddle-agent-dialog"
        headerSubtitle="Choose one of your relay agents to join this huddle."
        scrollAreaClassName="space-y-5"
        title="Add agents"
      >
        {error ? (
          <p className="rounded-lg border border-destructive/30 bg-destructive/10 px-4 py-3 text-sm text-destructive">
            {error}
          </p>
        ) : null}

        {warning ? (
          <p className="rounded-lg border border-warning/30 bg-warning-bg px-4 py-3 text-sm text-warning">
            {warning}
          </p>
        ) : null}

        {loading ? (
          <p className="py-4 text-center text-sm text-muted-foreground">
            Loading agents…
          </p>
        ) : agents.length === 0 ? (
          <p className="py-4 text-center text-sm text-muted-foreground">
            No relay agents are available to your identity.
          </p>
        ) : (
          <ul className="flex flex-col gap-1">
            {agents.map((agent) => {
              const normalizedPubkey = normalizePubkey(agent.pubkey);
              const isAdding = adding === agent.pubkey;
              const isCurrent = currentAgentSet.has(normalizedPubkey);
              const profile = profilesQuery.data?.profiles[normalizedPubkey];
              return (
                <li key={agent.pubkey}>
                  <button
                    className="flex w-full items-center gap-3 rounded-lg px-3 py-2.5 text-left transition-colors hover:bg-accent hover:text-accent-foreground disabled:opacity-50"
                    disabled={adding !== null || isCurrent}
                    onClick={() => void handleAdd(agent.pubkey)}
                    type="button"
                  >
                    <ProfileAvatar
                      avatarUrl={profile?.avatarUrl ?? null}
                      className="h-9 w-9 shrink-0 text-xs"
                      label={agent.name}
                    />
                    <span className="min-w-0 flex-1 truncate text-sm font-medium">
                      {agent.name}
                    </span>
                    {isCurrent ? (
                      <span className="text-xs text-muted-foreground">
                        Already in huddle
                      </span>
                    ) : isAdding ? (
                      <LoaderCircle
                        aria-label={`Adding ${agent.name}`}
                        className="h-4 w-4 shrink-0 animate-spin text-muted-foreground"
                      />
                    ) : (
                      <span className="text-xs capitalize text-muted-foreground">
                        {agent.status}
                      </span>
                    )}
                  </button>
                </li>
              );
            })}
          </ul>
        )}
      </ChooserDialogContent>
    </Dialog>
  );
}

export type { AgentAddResult };
