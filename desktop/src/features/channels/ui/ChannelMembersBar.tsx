import { EllipsisVertical, Settings2, Users } from "lucide-react";
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
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";
import { AddChannelBotDialog } from "./AddChannelBotDialog";

type ChannelMembersBarProps = {
  channel: Channel;
  currentPubkey?: string;
  isAddBotOpen?: boolean;
  onAddBotOpenChange?: (open: boolean) => void;
  onManageChannel: () => void;
  onToggleMembers: () => void;
  variant?: "inline" | "compact";
};

export function ChannelMembersBar({
  channel,
  currentPubkey,
  isAddBotOpen: isAddBotOpenProp,
  onAddBotOpenChange,
  onManageChannel,
  onToggleMembers,
  variant = "inline",
}: ChannelMembersBarProps) {
  const [uncontrolledAddBotOpen, setUncontrolledAddBotOpen] =
    React.useState(false);
  const isAddBotOpen = isAddBotOpenProp ?? uncontrolledAddBotOpen;
  const setIsAddBotOpen = React.useCallback(
    (open: boolean) => {
      onAddBotOpenChange?.(open);
      if (isAddBotOpenProp === undefined) {
        setUncontrolledAddBotOpen(open);
      }
    },
    [isAddBotOpenProp, onAddBotOpenChange],
  );
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
  const canStartHuddle =
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
  }, [channel.id, setIsAddBotOpen]);

  const dialogErrorMessage =
    providersQuery.error instanceof Error
      ? providersQuery.error.message
      : managedAgentsQuery.error instanceof Error
        ? managedAgentsQuery.error.message
        : relayAgentsQuery.error instanceof Error
          ? relayAgentsQuery.error.message
          : null;

  const huddleIndicator = (
    <HuddleIndicator
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
      renderMode={variant === "compact" ? "menu-item" : "button"}
      startDisabled={!canStartHuddle || isStartingHuddle}
    />
  );

  const controls =
    variant === "compact" ? (
      <DropdownMenu modal={false}>
        <DropdownMenuTrigger asChild>
          <Button
            aria-label="Channel actions"
            data-testid="channel-actions-menu-trigger"
            size="icon"
            type="button"
            variant="outline"
          >
            <EllipsisVertical />
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end" className="w-48" forceMount>
          <DropdownMenuItem
            data-testid="channel-members-trigger"
            onSelect={onToggleMembers}
          >
            <Users />
            <span>Members</span>
            <span className="ml-auto text-xs text-muted-foreground">
              {memberCount}
            </span>
          </DropdownMenuItem>
          {huddleIndicator}
          <DropdownMenuItem
            data-testid="channel-management-trigger"
            onSelect={onManageChannel}
          >
            <Settings2 />
            <span>Manage channel</span>
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>
    ) : (
      <div className="flex items-center gap-[6px]">
        <Button
          aria-label={`View channel members (${memberCount})`}
          className="h-8 px-2.5"
          data-testid="channel-members-trigger"
          onClick={onToggleMembers}
          type="button"
          variant="outline"
        >
          <Users />
          <span className="min-w-[1ch] text-sm font-medium tabular-nums">
            {memberCount}
          </span>
        </Button>

        {huddleIndicator}

        <Button
          aria-label="Manage channel"
          data-testid="channel-management-trigger"
          onClick={onManageChannel}
          size="icon"
          type="button"
          variant="outline"
        >
          <Settings2 />
        </Button>
      </div>
    );

  return (
    <React.Fragment>
      {controls}

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
