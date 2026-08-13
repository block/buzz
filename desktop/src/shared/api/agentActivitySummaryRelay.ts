import { relayClient } from "@/shared/api/relayClient";
import type { RelayEvent } from "@/shared/api/types";
import {
  buildAgentActivitySummaryFilter,
  parseAgentActivityEvent,
  type SharedAgentActivityFrame,
} from "@/features/agents/sharedAgentActivity";

export type AgentActivitySummaryEvent = {
  eventId: string;
  frame: SharedAgentActivityFrame;
};

export async function subscribeToAgentActivitySummaries(input: {
  agentPubkey: string;
  channelId: string;
  onEvent: (event: AgentActivitySummaryEvent) => void;
  onReady?: () => void;
  onTerminalClosed?: () => void;
}) {
  const parse = (event: RelayEvent) =>
    parseAgentActivityEvent(event, {
      expectedAgentPubkey: input.agentPubkey,
      expectedChannelId: input.channelId,
    });

  return relayClient.subscribeLive(
    buildAgentActivitySummaryFilter(input.agentPubkey, input.channelId),
    (event) => {
      const frame = parse(event);
      if (frame) input.onEvent({ eventId: event.id, frame });
    },
    (readiness) => {
      if (readiness !== "closed") input.onReady?.();
    },
    {
      reconnectMode: "live-only",
      // Admission happens before replay cursors/watermarks are mutated.
      admitEvent: (event) => parse(event) !== null,
      onClosed: (_message, terminal) => {
        if (terminal) input.onTerminalClosed?.();
      },
    },
  );
}
