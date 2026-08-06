import { getThreadReference } from "@/features/messages/lib/threading";
import { ArtilleryAgentTimeoutError } from "@/features/games/artillery/referee";
import {
  appendArtilleryDurableEvent,
  createArtilleryTurnRequestedEvent,
  type ArtilleryTurnRequestedEvent,
} from "@/features/games/artillery/durableProtocol";
import { relayClient } from "@/shared/api/relayClient";
import { sendChannelMessage } from "@/shared/api/tauri";
import type { RelayEvent } from "@/shared/api/types";
import {
  KIND_STREAM_MESSAGE,
  KIND_STREAM_MESSAGE_V2,
} from "@/shared/constants/kinds";
import type {
  ArtilleryAgent,
  ArtilleryMatchState,
} from "@/features/games/artillery/referee";

type LiveAgentAdapterDependencies = {
  sendPrompt: (
    channelId: string,
    content: string,
    mentionPubkeys: string[],
    parentEventId?: string | null,
  ) => Promise<{ eventId: string }>;
  subscribe: (
    channelId: string,
    onEvent: (event: RelayEvent) => void,
  ) => Promise<() => Promise<void>>;
};

const defaultDependencies: LiveAgentAdapterDependencies = {
  sendPrompt: (channelId, content, mentionPubkeys, parentEventId) =>
    sendChannelMessage(
      channelId,
      content,
      parentEventId,
      undefined,
      mentionPubkeys,
    ),
  subscribe: (channelId, onEvent) =>
    relayClient.subscribeToChannelLive(channelId, onEvent),
};

export function buildLiveAgentMovePrompt(
  state: ArtilleryMatchState,
  requestId: string,
  durableEvent?: ArtilleryTurnRequestedEvent,
) {
  const content = [
    `🎯 Buzz Artillery turn ${state.turn} · request ${requestId}`,
    `State: ${JSON.stringify(state)}`,
    "Reply with only this JSON shape—no markdown or explanation:",
    `{"requestId":"${requestId}","angle":45,"power":70,"weapon":"pulse-shell","taunt":"optional short line"}`,
    "Rules: angle 20-80, power 30-100, weapon must be pulse-shell.",
  ].join("\n");
  return durableEvent
    ? appendArtilleryDurableEvent(content, durableEvent)
    : content;
}

export function parseLiveAgentMove(
  content: string,
  requestId: string,
): unknown {
  try {
    const parsed: unknown = JSON.parse(content.trim());
    if (!parsed || typeof parsed !== "object") return null;
    if ((parsed as { requestId?: unknown }).requestId !== requestId)
      return null;
    return parsed;
  } catch {
    return null;
  }
}

function referencesPrompt(event: RelayEvent, promptEventId: string | null) {
  if (!promptEventId) return false;
  const reference = getThreadReference(event.tags);
  return (
    reference.parentId === promptEventId || reference.rootId === promptEventId
  );
}

export function createManagedArtilleryAgent({
  agent,
  channelId,
  responseTimeoutMs,
  side,
  threadRootEventId = null,
  dependencies = defaultDependencies,
}: {
  agent: { pubkey: string; name: string };
  channelId: string;
  responseTimeoutMs: number;
  side: "red" | "blue";
  threadRootEventId?: string | null;
  dependencies?: LiveAgentAdapterDependencies;
}): ArtilleryAgent {
  return {
    id: agent.pubkey,
    name: agent.name,
    side,
    decide: async (state) => {
      const requestId = `${state.id}:${state.turn}:${crypto.randomUUID()}`;
      const prompt = buildLiveAgentMovePrompt(
        state,
        requestId,
        createArtilleryTurnRequestedEvent({
          agent: { id: agent.pubkey, name: agent.name },
          deadlineAt: Date.now() + responseTimeoutMs,
          requestId,
          state,
        }),
      );
      let promptEventId: string | null = null;
      let unsubscribe: (() => Promise<void>) | undefined;
      let timer: ReturnType<typeof setTimeout> | undefined;

      try {
        return await new Promise<unknown>((resolve, reject) => {
          const settleFromEvent = (event: RelayEvent) => {
            if (
              (event.kind !== KIND_STREAM_MESSAGE &&
                event.kind !== KIND_STREAM_MESSAGE_V2) ||
              event.pubkey.toLowerCase() !== agent.pubkey.toLowerCase()
            ) {
              return;
            }
            const containsRequestId = event.content.includes(requestId);
            if (!containsRequestId && !referencesPrompt(event, promptEventId)) {
              return;
            }
            resolve(parseLiveAgentMove(event.content, requestId));
          };

          void dependencies
            .subscribe(channelId, settleFromEvent)
            .then((dispose) => {
              unsubscribe = dispose;
              return dependencies.sendPrompt(
                channelId,
                prompt,
                [agent.pubkey],
                threadRootEventId,
              );
            })
            .then((result) => {
              promptEventId = result.eventId;
            })
            .catch(reject);

          timer = setTimeout(
            () => reject(new ArtilleryAgentTimeoutError()),
            responseTimeoutMs,
          );
        });
      } finally {
        if (timer) clearTimeout(timer);
        if (unsubscribe) await unsubscribe().catch(() => {});
      }
    },
  };
}
