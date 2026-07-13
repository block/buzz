import { getVersion } from "@tauri-apps/api/app";
import { useMutation } from "@tanstack/react-query";
import * as React from "react";

import { useChannelsQuery } from "@/features/channels/hooks";
import type { ImetaMedia } from "@/features/messages/lib/imetaMediaMarkdown";
import { buildOutgoingMessage } from "@/features/messages/lib/imetaMediaMarkdown";
import type { SendFeedbackInput } from "@/features/settings/ui/SendFeedbackDialog";
import { FEEDBACK_CATEGORY_LABELS } from "@/features/settings/ui/SendFeedbackDialog";
import { useWorkspaces } from "@/features/workspaces/useWorkspaces";
import { sendChannelMessage, uploadMediaBytes } from "@/shared/api/tauri";
import { pickAndUploadImage } from "@/shared/api/tauriMedia";
import type { Channel } from "@/shared/api/types";

/** Returns a usable channel ID, or null when feedback is not configured. */
export function normalizeFeedbackChannelId(
  configuredId: string | undefined,
): string | null {
  const channelId = configuredId?.trim() ?? "";
  return channelId.length > 0 ? channelId : null;
}

/**
 * Build-time feedback destination. This is intentionally optional: default OSS
 * builds do not claim to offer a monitored feedback path.
 */
export function getConfiguredFeedbackChannelId(): string | null {
  return normalizeFeedbackChannelId(
    import.meta.env?.VITE_BUZZ_FEEDBACK_CHANNEL_ID,
  );
}

/** True when the optional feedback capability is enabled for this build. */
export const FEEDBACK_ENABLED = getConfiguredFeedbackChannelId() !== null;

/**
 * Resolves the configured destination by exact channel ID. It must be an active
 * private stream that the current user belongs to. No name matching or channel
 * creation is allowed: distributors are responsible for provisioning a
 * monitored inbox and its membership before enabling the feature.
 */
export function findFeedbackChannel(
  channels: Channel[] | undefined,
  configuredId: string,
): Channel | null {
  if (!channels) {
    return null;
  }
  return (
    channels.find(
      (channel) =>
        channel.id === configuredId &&
        channel.channelType === "stream" &&
        channel.visibility === "private" &&
        channel.archivedAt === null &&
        channel.isMember,
    ) ?? null
  );
}

const FEEDBACK_CHANNEL_UNAVAILABLE_MESSAGE =
  "Feedback is unavailable in this workspace. The configured feedback channel is missing, inaccessible, or not an active private stream. Ask your workspace administrator to verify the channel and your membership.";

/**
 * Best-effort diagnostics text bundled when the user checks
 * "Attach diagnostics".
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
 * Owns feedback delivery to the explicitly configured private channel, manages
 * the optional image attachment, gathers a diagnostics bundle when requested,
 * and posts the feedback message with imeta tags.
 */
export function useSendFeedback() {
  const configuredChannelId = getConfiguredFeedbackChannelId();
  const channelsQuery = useChannelsQuery({
    enabled: configuredChannelId !== null,
  });
  const { activeWorkspace } = useWorkspaces();
  const [attachedImage, setAttachedImage] = React.useState<ImetaMedia | null>(
    null,
  );
  // Bumped on every reset/close. `attachImage` captures the current value
  // before its (slow) upload await and drops the result if the token changed
  // meanwhile — otherwise an upload that resolves after the dialog closed
  // would repopulate a stale image into the next session's draft.
  const sessionRef = React.useRef(0);

  const attachImage = React.useCallback(async () => {
    const session = sessionRef.current;
    // The Rust `pick_and_upload_image` command validates the file is an image
    // (via MIME sniffing) BEFORE upload, so discarded/non-image files never
    // leave the client. Returns null when the user cancels the dialog.
    const descriptor = await pickAndUploadImage();
    if (!descriptor) {
      return;
    }
    // Dialog was reset/closed while the upload was in flight — discard.
    if (sessionRef.current !== session) {
      return;
    }
    setAttachedImage(descriptor);
  }, []);

  const removeImage = React.useCallback(() => {
    setAttachedImage(null);
  }, []);

  const reset = React.useCallback(() => {
    // Invalidate any in-flight attachImage continuation for this session.
    sessionRef.current += 1;
    setAttachedImage(null);
  }, []);

  const submitMutation = useMutation({
    mutationFn: async (input: SendFeedbackInput) => {
      if (!configuredChannelId) {
        throw new Error("Feedback is not configured for this build.");
      }

      // Refetch authoritatively before every send. The configured destination
      // may have been archived or the user's membership may have changed since
      // the sidebar cache was populated.
      const refreshed = await channelsQuery.refetch();
      const channel = findFeedbackChannel(refreshed.data, configuredChannelId);
      if (!channel) {
        throw new Error(FEEDBACK_CHANNEL_UNAVAILABLE_MESSAGE);
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
    destinationChannelId: configuredChannelId,
    destinationChannelName: configuredChannelId
      ? (findFeedbackChannel(channelsQuery.data, configuredChannelId)?.name ??
        null)
      : null,
    destinationRelayUrl: activeWorkspace?.relayUrl ?? "the current relay",
    destinationWorkspaceName: activeWorkspace?.name ?? "this workspace",
    isPending: submitMutation.isPending,
    removeImage,
    reset,
    submit: submitMutation.mutateAsync,
  };
}
