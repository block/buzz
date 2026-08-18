import { getVersion } from "@tauri-apps/api/app";
import { useMutation } from "@tanstack/react-query";
import * as React from "react";

import type { ImetaMedia } from "@/features/messages/lib/imetaMediaMarkdown";
import { buildOutgoingMessage } from "@/features/messages/lib/imetaMediaMarkdown";
import type { SendFeedbackInput } from "@/features/settings/ui/SendFeedbackDialog";
import { relayClient } from "@/shared/api/relayClient";
import { signRelayEvent, uploadMediaBytes } from "@/shared/api/tauri";
import { pickAndUploadImage } from "@/shared/api/tauriMedia";
import { KIND_PRODUCT_FEEDBACK } from "@/shared/constants/kinds";

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

export function buildProductFeedbackEvent(
  input: Pick<SendFeedbackInput, "category" | "message">,
  attachments: ImetaMedia[],
): { content: string; tags: string[][] } {
  const { content, mediaTags } = buildOutgoingMessage(
    input.message.trim(),
    attachments,
  );
  return {
    content,
    tags: [
      ...(input.category ? [["category", input.category]] : []),
      ...(mediaTags ?? []),
    ],
  };
}

/** Owns private product-feedback submission and optional attachment uploads. */
export function useSendFeedback() {
  const [attachedImage, setAttachedImage] = React.useState<ImetaMedia | null>(
    null,
  );
  const [isAttaching, setIsAttaching] = React.useState(false);
  const sessionRef = React.useRef(0);
  const attachmentAttemptRef = React.useRef(0);

  const attachImage = React.useCallback(async () => {
    const session = sessionRef.current;
    const attempt = attachmentAttemptRef.current + 1;
    attachmentAttemptRef.current = attempt;
    setIsAttaching(true);
    try {
      const descriptor = await pickAndUploadImage();
      if (
        !descriptor ||
        sessionRef.current !== session ||
        attachmentAttemptRef.current !== attempt
      ) {
        return;
      }
      setAttachedImage(descriptor);
    } catch (error) {
      if (
        sessionRef.current === session &&
        attachmentAttemptRef.current === attempt
      ) {
        throw error;
      }
    } finally {
      if (
        sessionRef.current === session &&
        attachmentAttemptRef.current === attempt
      ) {
        setIsAttaching(false);
      }
    }
  }, []);

  const removeImage = React.useCallback(() => {
    setAttachedImage(null);
  }, []);

  const reset = React.useCallback(() => {
    sessionRef.current += 1;
    attachmentAttemptRef.current += 1;
    setAttachedImage(null);
    setIsAttaching(false);
  }, []);

  const submitMutation = useMutation({
    mutationFn: async (input: SendFeedbackInput) => {
      const attachments: ImetaMedia[] = [];
      if (attachedImage) {
        attachments.push(attachedImage);
      }
      if (input.includeLogs) {
        const diagnostics = await collectDiagnostics();
        const bytes = Array.from(new TextEncoder().encode(diagnostics));
        attachments.push(
          await uploadMediaBytes(
            bytes,
            `feedback-diagnostics-${Date.now()}.txt`,
          ),
        );
      }

      const payload = buildProductFeedbackEvent(input, attachments);
      const event = await signRelayEvent({
        kind: KIND_PRODUCT_FEEDBACK,
        content: payload.content,
        tags: payload.tags,
      });
      await relayClient.publishEvent(
        event,
        "Timed out while sending feedback.",
        "Failed to send feedback.",
      );
    },
    onSuccess: reset,
  });

  return {
    attachImage,
    attachedImage,
    isAttaching,
    isPending: submitMutation.isPending,
    removeImage,
    reset,
    submit: submitMutation.mutateAsync,
  };
}
