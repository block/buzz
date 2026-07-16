import * as React from "react";

import { attachManagedAgentToChannel } from "./channelAgents";
import type { AgentChannelAttachmentFailure } from "./channelAttachmentFailure";
import type { Channel, CreateManagedAgentResponse } from "@/shared/api/types";

type TargetChannel = Pick<Channel, "id" | "name">;

/**
 * Keeps agent creation successful even when the follow-up channel attachment
 * fails, and retries only that attachment rather than recreating the agent.
 */
export function useCreatedAgentChannelAttachment() {
  const [createdAgent, setCreatedAgent] =
    React.useState<CreateManagedAgentResponse | null>(null);
  const [attachmentFailure, setAttachmentFailure] =
    React.useState<AgentChannelAttachmentFailure | null>(null);
  const targetChannelRef = React.useRef<TargetChannel | null>(null);
  const [isRetryingAttachment, setIsRetryingAttachment] = React.useState(false);

  async function attach(
    created: CreateManagedAgentResponse,
    targetChannel: TargetChannel,
  ) {
    targetChannelRef.current = targetChannel;
    try {
      const attached = await attachManagedAgentToChannel(targetChannel.id, {
        agent: created.agent,
        role: "bot",
        ensureRunning: true,
      });
      created.agent = attached.agent;
      targetChannelRef.current = null;
      setAttachmentFailure(null);
    } catch (cause) {
      setAttachmentFailure({
        channelName: targetChannel.name,
        error: cause instanceof Error ? cause.message : "Failed to add agent.",
      });
    }
  }

  async function presentCreatedAgent(
    created: CreateManagedAgentResponse,
    targetChannel?: TargetChannel | null,
  ) {
    setAttachmentFailure(null);
    targetChannelRef.current = null;
    if (!created.spawnError && targetChannel) {
      await attach(created, targetChannel);
    }
    setCreatedAgent({ ...created });
  }

  async function retryAttachment() {
    const targetChannel = targetChannelRef.current;
    if (!createdAgent || !targetChannel || isRetryingAttachment) return;

    setIsRetryingAttachment(true);
    try {
      await attach(createdAgent, targetChannel);
      setCreatedAgent({ ...createdAgent });
    } finally {
      setIsRetryingAttachment(false);
    }
  }

  function dismissCreatedAgent() {
    setCreatedAgent(null);
    setAttachmentFailure(null);
    targetChannelRef.current = null;
  }

  return {
    attachmentFailure,
    createdAgent,
    dismissCreatedAgent,
    isRetryingAttachment,
    presentCreatedAgent,
    retryAttachment,
  };
}
