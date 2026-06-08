import { Plus, Settings2, Users } from "lucide-react";
import * as React from "react";
import { useQueryClient } from "@tanstack/react-query";
import { useHuddle } from "@/features/huddle";
import { HuddleIndicator } from "@/features/huddle/components/HuddleIndicator";
import {
  useAvailableAcpRuntimes,
  useBackendProvidersQuery,
  useManagedAgentsQuery,
  useRelayAgentsQuery,
} from "@/features/agents/hooks";
import { useChannelMembersQuery } from "@/features/channels/hooks";
import type { Channel } from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { Button } from "@/shared/ui/button";
import { AddChannelBotDialog } from "./AddChannelBotDialog";

type ChannelMembersBarProps = {
  channel: Channel;
  currentPubkey?: string;
  onManageChannel: () => void;
  onToggleMembers: () => void;
};

export function ChannelMembersBar({
  channel,
  currentPubkey,
  onManageChannel,
  onToggleMembers,
}: ChannelMembersBarProps) {
  const [isAddBotOpen, setIsAddBotOpen] = React.useState(false);
  const { startHuddle, isStarting: isStartingHuddle } = useHuddle();
  const queryClient = useQueryClient();
  const membersQuery = useChannelMembersQuery(channel.id);
  const providersQuery = useAvailableAcpRuntimes();
  const backendProvidersQuery = useBackendProvidersQuery();
  const managedAgentsQuery = useManagedAgentsQuery();
  const relayAgentsQuery = useRelayAgentsQuery();
  const members = membersQuery.data ?? [];
  const memberCount = membersQuery.data?.length ?? channel.memberCount;
  const providers = React.useMemo(
    () =>
      [...(providersQuery.data ?? [])].sort((left, right) => {
        const leftPriority = left.id === "goose" ? 0 : 1;
        const rightPriority = right.id === "goose" ? 0 : 1;
        if (leftPriority !== rightPriority) {
          return leftPriority - rightPriority;
        }

        return left.label.localeCompare(right.label);
      }),
    [providersQuery.data],
  );
  const normalizedCurrentPubkey = currentPubkey
    ? normalizePubkey(currentPubkey)
    : null;
  const selfMember =
    members.find(
      (member) => normalizePubkey(member.pubkey) === normalizedCurrentPubkey,
    ) ?? null;
  const canAddAgents =
    channel.channelType !== "dm" &&
    channel.archivedAt === null &&
    (channel.visibility === "open" || selfMember !== null);
  const previousChannelIdRef = React.useRef(channel.id);

  React.useEffect(() => {
    if (previousChannelIdRef.current === channel.id) {
      return;
    }

    previousChannelIdRef.current = channel.id;
    setIsAddBotOpen(false);
  }, [channel.id]);

  const dialogErrorMessage =
    providersQuery.error instanceof Error
      ? providersQuery.error.message
      : managedAgentsQuery.error instanceof Error
        ? managedAgentsQuery.error.message
        : relayAgentsQuery.error instanceof Error
          ? relayAgentsQuery.error.message
          : null;

  return (
    <React.Fragment>
      <div className="flex items-center gap-[6px]">
        <Button
          aria-label="Add agent"
          className="h-8 w-8 rounded-lg border border-border/40 text-muted-foreground hover:bg-muted/70 hover:text-foreground [&_svg]:size-5"
          data-testid="channel-add-bot-trigger"
          disabled={!canAddAgents}
          onClick={() => {
            setIsAddBotOpen(true);
          }}
          size="icon"
          type="button"
          variant="ghost"
        >
          <Plus className="size-5" />
        </Button>

        <Button
          aria-label={`View channel members (${memberCount})`}
          className="h-8 gap-1 rounded-lg border border-border/40 px-2.5 text-muted-foreground hover:bg-muted/70 hover:text-foreground [&_svg]:size-5"
          data-testid="channel-members-trigger"
          onClick={onToggleMembers}
          type="button"
          variant="ghost"
        >
          <Users className="size-5" />
          <span className="min-w-[1ch] text-[11px] font-medium tabular-nums">
            {memberCount}
          </span>
        </Button>

        <HuddleIndicator
          className="h-8 w-8"
          channelId={channel.id}
          onStart={async () => {
            try {
              await startHuddle(channel.id, []);
              // Refetch channels so the new ephemeral channel appears in the sidebar immediately
              // (default poll interval is 60s — too slow for huddle UX).
              void queryClient.invalidateQueries({ queryKey: ["channels"] });
            } catch (e) {
              console.error("Failed to start huddle:", e);
            }
          }}
          startDisabled={!canAddAgents || isStartingHuddle}
        />

        <Button
          aria-label="Manage channel"
          className="h-8 w-8 rounded-lg border border-border/40 text-muted-foreground hover:bg-muted/70 hover:text-foreground [&_svg]:size-5"
          data-testid="channel-management-trigger"
          onClick={onManageChannel}
          size="icon"
          type="button"
          variant="ghost"
        >
          <Settings2 className="size-5" />
        </Button>
      </div>

      <AddChannelBotDialog
        backendProviders={backendProvidersQuery.data ?? []}
        backendProvidersLoading={backendProvidersQuery.isLoading}
        channelId={channel.id}
        onOpenChange={setIsAddBotOpen}
        open={isAddBotOpen}
        providers={providers}
        providersErrorMessage={dialogErrorMessage}
        providersLoading={providersQuery.isLoading}
      />
    </React.Fragment>
  );
}
