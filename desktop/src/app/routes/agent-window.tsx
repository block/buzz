import { createFileRoute } from "@tanstack/react-router";

import { AgentWindowScreen } from "@/features/agents/ui/AgentWindowScreen";
import type { ChannelType } from "@/shared/api/types";

type AgentWindowSearch = {
  channelId?: string;
  pubkey?: string;
  name?: string;
  channelName?: string;
  channelType?: ChannelType;
};

const CHANNEL_TYPES: ChannelType[] = ["stream", "forum", "dm"];

function asString(value: unknown): string | undefined {
  return typeof value === "string" && value.length > 0 ? value : undefined;
}

function validateAgentWindowSearch(
  search: Record<string, unknown>,
): AgentWindowSearch {
  const channelType = asString(search.channelType);
  return {
    channelId: asString(search.channelId),
    pubkey: asString(search.pubkey),
    name: asString(search.name),
    channelName: asString(search.channelName),
    channelType: CHANNEL_TYPES.includes(channelType as ChannelType)
      ? (channelType as ChannelType)
      : undefined,
  };
}

export const Route = createFileRoute("/agent-window")({
  validateSearch: validateAgentWindowSearch,
  component: AgentWindowRouteComponent,
});

function AgentWindowRouteComponent() {
  const { channelId, pubkey, name, channelName, channelType } =
    Route.useSearch();

  if (!channelId || !pubkey) {
    return (
      <div
        className="flex min-h-0 flex-1 items-center justify-center px-6 text-center text-sm text-muted-foreground"
        data-testid="agent-window-missing"
      >
        This conversation window is missing its agent or channel reference.
      </div>
    );
  }

  return (
    <AgentWindowScreen
      agentPubkey={pubkey}
      channelId={channelId}
      channelType={channelType ?? null}
      initialAgentName={name ?? "Agent"}
      initialChannelName={channelName ?? ""}
    />
  );
}
