import * as React from "react";

import { useChannelWorkingAgentPubkeys } from "@/features/agents/agentWorkingSignal";
import { useAgentTranscript } from "@/features/agents/ui/useObserverEvents";
import { selectLatestActivityHeadline } from "@/features/dev-mode/lib/agentActivityStatus";
import { useAuthorColorResolver } from "@/features/dev-mode/lib/authorColors";
import { useMemberNameResolver } from "@/features/dev-mode/lib/useMemberNameResolver";
import type { Channel } from "@/shared/api/types";

const MAX_STATUS_ROWS = 3;

/** One agent's line: colored name + its newest channel-scoped activity. */
function AgentStatusRow({
  pubkey,
  channelId,
  name,
  color,
}: {
  pubkey: string;
  channelId: string;
  name: string;
  color: string;
}) {
  const transcript = useAgentTranscript(true, pubkey);
  const headline = React.useMemo(
    () => selectLatestActivityHeadline(transcript, channelId),
    [channelId, transcript],
  );

  return (
    <div
      className="flex min-w-0 items-baseline gap-2"
      data-testid="dev-mode-agent-status-row"
    >
      <span className="shrink-0" style={{ color }}>
        {name}
      </span>
      <span className="min-w-0 truncate text-muted-foreground/60">
        {headline ?? "working…"}
      </span>
    </div>
  );
}

/**
 * Quiet per-channel activity readout, pinned between the transcript and the
 * composer. While agents are working here it shows one line per agent with
 * the newest headline from their observer transcript (the command being run,
 * file being edited, …). It fades in once and updates text in place — no
 * spinner, no rotation — matching dev mode's terminal restraint.
 */
export function DevAgentStatusLine({ channel }: { channel: Channel }) {
  const workingPubkeys = useChannelWorkingAgentPubkeys(channel.id);
  const resolveName = useMemberNameResolver(channel.id, workingPubkeys);
  const resolveColor = useAuthorColorResolver();

  if (workingPubkeys.length === 0) {
    return null;
  }

  const visible = workingPubkeys.slice(0, MAX_STATUS_ROWS);
  const hidden = workingPubkeys.length - visible.length;

  return (
    <div
      className="shrink-0 select-none px-4 pb-1.5 font-mono text-xs leading-5 animate-in fade-in duration-300 motion-reduce:animate-none"
      data-testid="dev-mode-agent-status"
    >
      {visible.map((pubkey) => (
        <AgentStatusRow
          channelId={channel.id}
          color={resolveColor(pubkey)}
          key={pubkey}
          name={resolveName(pubkey)}
          pubkey={pubkey}
        />
      ))}
      {hidden > 0 ? (
        <div className="text-muted-foreground/60">
          +{hidden} more {hidden === 1 ? "agent" : "agents"} working
        </div>
      ) : null}
    </div>
  );
}
