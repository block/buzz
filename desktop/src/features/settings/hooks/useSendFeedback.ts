import { getVersion } from "@tauri-apps/api/app";
import { useMutation } from "@tanstack/react-query";
import * as React from "react";

import { useChannelsQuery } from "@/features/channels/hooks";
import type { ImetaMedia } from "@/features/messages/lib/imetaMediaMarkdown";
import { buildOutgoingMessage } from "@/features/messages/lib/imetaMediaMarkdown";
import type { SendFeedbackInput } from "@/features/settings/ui/SendFeedbackDialog";
import {
  createChannel,
  pickAndUploadMedia,
  sendChannelMessage,
  uploadMediaBytes,
} from "@/shared/api/tauri";
import type { Channel } from "@/shared/api/types";

/** Name of the private channel feedback is delivered to. */
export const FEEDBACK_CHANNEL_NAME = "Buzz feedback";

const CATEGORY_LABELS: Record<string, string> = {
  bug: "Bug",
  praise: "Praise",
  "needs-work": "Needs work",
};

function findFeedbackChannel(channels: Channel[] | undefined): Channel | null {
  if (!channels) {
    return null;
  }
  return (
    channels.find(
      (channel) =>
        channel.channelType !== "dm" &&
        channel.name.trim().toLowerCase() ===
          FEEDBACK_CHANNEL_NAME.toLowerCase(),
    ) ?? null
  );
}

/**
 * Best-effort diagnostics text bundled when the user checks "Attach logs".
 *
 * The desktop app has no on-disk log file yet (it logs to stderr), so this
 * captures the environment context that is available client-side. When a real
 * log sink lands, extend this to include recent log lines.
 */
async function collectDiagnostics(): Promise<string> {
  let appVersion = "unknown";
  try {
    appVersion = await getVersion();
  } catch {
    // Non-fatal — fall through with "unknown".
  }
  const nav = typeof navigator !== "undefined" ? navigator : undefined;
  return [
    "Buzz feedback diagnostics",
    `captured: ${new Date().toISOString()}`,
    `app version: ${appVersion}`,
    `platform: ${nav?.platform ?? "unknown"}`,
    `user agent: ${nav?.userAgent ?? "unknown"}`,
    `language: ${nav?.language ?? "unknown"}`,
  ].join("\n");
}

/**
 * Owns feedback delivery: resolves (or creates) the private "Buzz feedback"
 * channel, manages the optional image attachment, gathers a diagnostics bundle
 * when logs are requested, and posts the feedback message with imeta tags.
 */
export function useSendFeedback() {
  const channelsQuery = useChannelsQuery();
  const [attachedImage, setAttachedImage] = React.useState<ImetaMedia | null>(
    null,
  );

  const attachImage = React.useCallback(async () => {
    const descriptors = await pickAndUploadMedia();
    const first = descriptors[0];
    if (first) {
      setAttachedImage(first);
    }
  }, []);

  const removeImage = React.useCallback(() => {
    setAttachedImage(null);
  }, []);

  const reset = React.useCallback(() => {
    setAttachedImage(null);
  }, []);

  const submitMutation = useMutation({
    mutationFn: async (input: SendFeedbackInput) => {
      // Resolve or create the private feedback channel.
      let channel = findFeedbackChannel(channelsQuery.data);
      if (!channel) {
        channel = await createChannel({
          name: FEEDBACK_CHANNEL_NAME,
          channelType: "stream",
          visibility: "private",
          description: "In-app feedback and bug reports.",
        });
      }

      const headerParts: string[] = [];
      if (input.category) {
        headerParts.push(
          `**${CATEGORY_LABELS[input.category] ?? input.category}**`,
        );
      }
      const bodyLines = [headerParts.join(" "), input.message]
        .filter((part) => part.length > 0)
        .join("\n\n");

      const attachments: ImetaMedia[] = [];
      if (attachedImage) {
        attachments.push(attachedImage);
      }
      if (input.includeLogs) {
        const diagnostics = await collectDiagnostics();
        const bytes = Array.from(new TextEncoder().encode(diagnostics));
        const logDescriptor = await uploadMediaBytes(
          bytes,
          `feedback-diagnostics-${Date.now()}.txt`,
        );
        attachments.push(logDescriptor);
      }

      const { content, mediaTags } = buildOutgoingMessage(
        bodyLines,
        attachments,
      );

      await sendChannelMessage(channel.id, content, null, mediaTags);
    },
    onSuccess: () => {
      reset();
    },
  });

  return {
    attachImage,
    attachedImage,
    isPending: submitMutation.isPending,
    removeImage,
    reset,
    submit: submitMutation.mutateAsync,
  };
}
