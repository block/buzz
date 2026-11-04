// Buzz source localisation: authored UI strings only; user values stay unchanged.
import { uiT } from "@/shared/i18n/uiTranslation";
import { toast } from "sonner";

import { attachManagedAgentToChannel } from "./channelAgents";
import type { Channel, CreateManagedAgentResponse } from "@/shared/api/types";

type TargetChannel = Pick<Channel, "id" | "name">;

async function attach(
  created: CreateManagedAgentResponse,
  targetChannel: TargetChannel,
) {
  const attached = await attachManagedAgentToChannel(targetChannel.id, {
    agent: created.agent,
    role: "bot",
    ensureRunning: true,
  });
  created.agent = attached.agent;
}

function showAttachmentFailure(
  created: CreateManagedAgentResponse,
  targetChannel: TargetChannel,
  cause: unknown,
  toastId?: string | number,
) {
  const error = cause instanceof Error ? cause.message : "Failed to add agent.";
  const id = toast.warning(uiT("Agent created"), {
    description: uiT("{0} couldn’t be added to #{1}. {2}", {
      p0: created.agent.name,
      p1: targetChannel.name,
      p2: error,
    }),
    id: toastId,
    action: {
      get label() {
        return uiT("Try again");
      },
      onClick: (event) => {
        event.preventDefault();
        toast.loading(uiT("Agent created"), {
          description: uiT("Adding {0} to #{1}…", {
            p0: created.agent.name,
            p1: targetChannel.name,
          }),
          id,
        });
        void attach(created, targetChannel).then(
          () => {
            toast.success(uiT("Agent created"), {
              description: uiT("Added {0} to #{1}", {
                p0: created.agent.name,
                p1: targetChannel.name,
              }),
              id,
            });
          },
          (retryCause: unknown) => {
            showAttachmentFailure(created, targetChannel, retryCause, id);
          },
        );
      },
    },
  });
}

/** Keeps creation successful when its optional channel attachment fails. */
export function useCreatedAgentChannelAttachment() {
  async function presentCreatedAgent(
    created: CreateManagedAgentResponse,
    targetChannel?: TargetChannel | null,
  ) {
    if (created.spawnError || !targetChannel) {
      toast.success(uiT("Agent created"));
      return;
    }

    try {
      await attach(created, targetChannel);
      toast.success(uiT("Agent created"));
    } catch (cause) {
      showAttachmentFailure(created, targetChannel, cause);
    }
  }

  return { presentCreatedAgent };
}
