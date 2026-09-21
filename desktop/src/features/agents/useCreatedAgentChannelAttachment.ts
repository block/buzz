import { toast } from "sonner";

import { i18n, useTranslation } from "@/i18n";
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
  const error =
    cause instanceof Error
      ? cause.message
      : i18n.t("agents.channel-attach.add-failed");
  const id = toast.warning(i18n.t("agents.agent-created.title"), {
    description: i18n.t("agents.channel-attach.failed-description", {
      agentName: created.agent.name,
      channelName: targetChannel.name,
      error,
    }),
    id: toastId,
    action: {
      label: i18n.t("agents.channel-attach.try-again"),
      onClick: (event) => {
        event.preventDefault();
        toast.loading(i18n.t("agents.agent-created.title"), {
          description: i18n.t("agents.channel-attach.retrying-description", {
            agentName: created.agent.name,
            channelName: targetChannel.name,
          }),
          id,
        });
        void attach(created, targetChannel).then(
          () => {
            toast.success(i18n.t("agents.agent-created.title"), {
              description: i18n.t("agents.channel-attach.added-description", {
                agentName: created.agent.name,
                channelName: targetChannel.name,
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
  const { t } = useTranslation();

  async function presentCreatedAgent(
    created: CreateManagedAgentResponse,
    targetChannel?: TargetChannel | null,
  ) {
    if (created.spawnError || !targetChannel) {
      toast.success(t("agents.agent-created.title"));
      return;
    }

    try {
      await attach(created, targetChannel);
      toast.success(t("agents.agent-created.title"));
    } catch (cause) {
      showAttachmentFailure(created, targetChannel, cause);
    }
  }

  return { presentCreatedAgent };
}
