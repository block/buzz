import { refreshDirectoryAfterMembershipChange } from "@/features/channels/membershipDirectorySync";
import { useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";

import { attachManagedAgentToChannel } from "./channelAgents";
import type { Channel, CreateManagedAgentResponse } from "@/shared/api/types";

type TargetChannel = Pick<Channel, "id" | "name">;

async function attach(
  created: CreateManagedAgentResponse,
  targetChannel: TargetChannel,
  onMembershipAdded: () => void,
) {
  const attached = await attachManagedAgentToChannel(targetChannel.id, {
    agent: created.agent,
    role: "bot",
    ensureRunning: true,
    onMembershipAdded,
  });
  created.agent = attached.agent;
}

function showAttachmentFailure(
  created: CreateManagedAgentResponse,
  targetChannel: TargetChannel,
  cause: unknown,
  onMembershipAdded: () => void,
  toastId?: string | number,
) {
  const error = cause instanceof Error ? cause.message : "Failed to add agent.";
  const id = toast.warning("Agent created", {
    description: `${created.agent.name} couldn’t be added to #${targetChannel.name}. ${error}`,
    id: toastId,
    action: {
      label: "Try again",
      onClick: (event) => {
        event.preventDefault();
        toast.loading("Agent created", {
          description: `Adding ${created.agent.name} to #${targetChannel.name}…`,
          id,
        });
        void attach(created, targetChannel, onMembershipAdded).then(
          () => {
            toast.success("Agent created", {
              description: `Added ${created.agent.name} to #${targetChannel.name}`,
              id,
            });
          },
          (retryCause: unknown) => {
            showAttachmentFailure(
              created,
              targetChannel,
              retryCause,
              onMembershipAdded,
              id,
            );
          },
        );
      },
    },
  });
}

/** Keeps creation successful when its optional channel attachment fails. */
export function useCreatedAgentChannelAttachment() {
  const queryClient = useQueryClient();
  const onMembershipAdded = () =>
    refreshDirectoryAfterMembershipChange(queryClient);
  async function presentCreatedAgent(
    created: CreateManagedAgentResponse,
    targetChannel?: TargetChannel | null,
  ) {
    if (created.spawnError || !targetChannel) {
      toast.success("Agent created");
      return;
    }

    try {
      await attach(created, targetChannel, onMembershipAdded);
      toast.success("Agent created");
    } catch (cause) {
      showAttachmentFailure(created, targetChannel, cause, onMembershipAdded);
    }
  }

  return { presentCreatedAgent };
}
