import { useSearch } from "@tanstack/react-router";
import { ListTodo } from "lucide-react";
import * as React from "react";

import { useCommunities } from "@/features/communities/useCommunities";
import type { Channel } from "@/shared/api/types";
import { Button } from "@/shared/ui/button";
import { channelNextUpProjects } from "../lib/channelNextUp";
import type { Project } from "../projectModels";
import { ChannelNextUpPanel, ChannelNextUpRefresh } from "./ChannelNextUpPanel";

/** Compose Next up through the channel's existing header and auxiliary-panel slots. */
export function useChannelNextUpPanel({
  channel,
  projects,
  pubkey,
}: {
  channel: Channel | null;
  projects: Project[];
  pubkey?: string;
}) {
  const { activeCommunity } = useCommunities();
  const search = useSearch({ strict: false }) as {
    thread?: string;
    profile?: string;
    agentSession?: string;
  };
  const scopedProjects = React.useMemo(
    () => channelNextUpProjects(projects, channel?.id ?? ""),
    [channel?.id, projects],
  );
  const scope = JSON.stringify([
    activeCommunity?.relayUrl,
    pubkey,
    channel?.id,
    search.thread,
    search.profile,
    search.agentSession,
  ]);
  const [openScope, setOpenScope] = React.useState<string | null>(null);
  const close = React.useCallback(() => setOpenScope(null), []);
  React.useEffect(() => {
    setOpenScope((current) => (current === scope ? current : null));
  }, [scope]);
  const available = Boolean(
    channel?.isMember &&
      channel.channelType === "stream" &&
      pubkey &&
      scopedProjects.length,
  );
  const isOpen = available && openScope === scope;
  return {
    close,
    isOpen,
    headerAction: available ? (
      <Button
        aria-label={isOpen ? "Hide Next up" : "Show Next up"}
        aria-pressed={isOpen}
        className="gap-1.5"
        data-testid="channel-next-up-toggle"
        onClick={() => setOpenScope(isOpen ? null : scope)}
        size="sm"
        type="button"
        variant={isOpen ? "secondary" : "ghost"}
      >
        <ListTodo className="h-4 w-4" /> Next up
      </Button>
    ) : null,
    headerControls:
      isOpen && channel && pubkey
        ? {
            actions: (
              <ChannelNextUpRefresh
                channelId={channel.id}
                key={scope}
                projects={scopedProjects}
                pubkey={pubkey}
              />
            ),
          }
        : undefined,
    panel:
      isOpen && channel && pubkey ? (
        <ChannelNextUpPanel
          channelId={channel.id}
          key={scope}
          projects={scopedProjects}
          pubkey={pubkey}
        />
      ) : null,
  };
}
