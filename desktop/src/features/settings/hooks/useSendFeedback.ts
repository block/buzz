import { getVersion } from "@tauri-apps/api/app";
import { useMutation } from "@tanstack/react-query";
import * as React from "react";

import {
  useChannelsQuery,
  useCreateChannelMutation,
} from "@/features/channels/hooks";
import type { ImetaMedia } from "@/features/messages/lib/imetaMediaMarkdown";
import { buildOutgoingMessage } from "@/features/messages/lib/imetaMediaMarkdown";
import type { SendFeedbackInput } from "@/features/settings/ui/SendFeedbackDialog";
import { FEEDBACK_CATEGORY_LABELS } from "@/features/settings/ui/SendFeedbackDialog";
import { sendChannelMessage, uploadMediaBytes } from "@/shared/api/tauri";
import { pickAndUploadImage } from "@/shared/api/tauriMedia";
import type { Channel } from "@/shared/api/types";

/** Name of the private channel feedback is delivered to. */
export const FEEDBACK_CHANNEL_NAME = "Buzz feedback";

/**
 * Resolves the private feedback channel from the channel list by name
 * (case-insensitive). Requires an **active private stream** channel the user
 * is a member of: it must be a `stream` (not a DM or forum — a forum would
 * file feedback as forum posts), non-archived (archived channels reject
 * writes), private (never an open/public channel that merely shares the
 * name), and one the user belongs to. Anything else falls through so the
 * caller creates a fresh channel. Exported for unit testing.
 */
export function findFeedbackChannel(
  channels: Channel[] | undefined,
): Channel | null {
  if (!channels) {
    return null;
  }
  return (
    channels.find(
      (channel) =>
        channel.channelType === "stream" &&
        channel.visibility === "private" &&
        channel.archivedAt === null &&
        channel.isMember &&
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
  const createChannelMutation = useCreateChannelMutation();
  const [attachedImage, setAttachedImage] = React.useState<ImetaMedia | null>(
    null,
  );

  const attachImage = React.useCallback(async () => {
    // The Rust `pick_and_upload_image` command validates the file is an image
    // (via MIME sniffing) BEFORE upload, so discarded/non-image files never
    // leave the client. Returns null when the user cancels the dialog.
    const descriptor = await pickAndUploadImage();
    if (!descriptor) {
      return;
    }
    setAttachedImage(descriptor);
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
        // The cached list may be stale or still seeded from a cold-start
        // snapshot — e.g. the initial fetch hasn't landed, or we were added to
        // the feedback channel from another client. Refetch authoritatively
        // before creating, so we don't spawn a duplicate of a channel that
        // already exists server-side.
        const refreshed = await channelsQuery.refetch();
        channel = findFeedbackChannel(refreshed.data);
      }
      if (!channel) {
        channel = await createChannelMutation.mutateAsync({
          name: FEEDBACK_CHANNEL_NAME,
          channelType: "stream",
          visibility: "private",
          description: "In-app feedback and bug reports.",
        });
      }

      const headerParts: string[] = [];
      if (input.category) {
        headerParts.push(
          `**${FEEDBACK_CATEGORY_LABELS[input.category] ?? input.category}**`,
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
