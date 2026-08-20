import * as React from "react";

import type { ParsedMessageLink } from "@/features/messages/lib/messageLink";
import { summarizeMessageLinkContent } from "@/features/messages/lib/messageLinkMetadata";
import { getEventById } from "@/shared/api/tauri";
import { getUserProfile } from "@/shared/api/tauriProfiles";
import { truncatePubkey } from "@/shared/lib/pubkey";
import { isDefinitiveEventNotFound } from "@/shared/lib/eventLookupError";

const MESSAGE_METADATA_RETRY_DELAY_MS = 750;
const PREVIEWABLE_MESSAGE_KINDS = new Set([9, 40002, 45001, 45003]);

function waitForMessageMetadataRetry(): Promise<void> {
  return new Promise((resolve) => {
    window.setTimeout(resolve, MESSAGE_METADATA_RETRY_DELAY_MS);
  });
}

async function getMessageLinkEvent(messageId: string) {
  try {
    return await getEventById(messageId);
  } catch (error) {
    if (isDefinitiveEventNotFound(error)) throw error;
    await waitForMessageMetadataRetry();
    return getEventById(messageId);
  }
}

type MessageLinkMetadata = {
  author: string;
  createdAt: number;
  snippet: string;
};
type MessageLinkMetadataState =
  | { kind: "idle" }
  | { kind: "loading" }
  | ({ kind: "ready" } & MessageLinkMetadata)
  | { kind: "deleted" }
  | { kind: "unavailable" };

type CachedMessageLinkMetadata =
  | ({ kind: "ready" } & MessageLinkMetadata)
  | { kind: "deleted" }
  | { kind: "unavailable" };

const metadataCache = new Map<string, Promise<CachedMessageLinkMetadata>>();

export function resetMessageLinkMetadataCache() {
  metadataCache.clear();
}

function fetchMetadata(
  link: Pick<ParsedMessageLink, "channelId" | "messageId">,
): Promise<CachedMessageLinkMetadata> {
  const key = `${link.channelId}:${link.messageId}`;
  let request = metadataCache.get(key);
  if (!request) {
    request = getMessageLinkEvent(link.messageId)
      .then(async (event) => {
        const eventChannelId = event.tags.find((tag) => tag[0] === "h")?.[1];
        if (
          eventChannelId !== link.channelId ||
          !PREVIEWABLE_MESSAGE_KINDS.has(event.kind)
        ) {
          return { kind: "unavailable" } as const;
        }
        const profile = await getUserProfile(event.pubkey).catch(() => null);
        return {
          kind: "ready" as const,
          author:
            profile?.displayName?.trim() ||
            profile?.nip05Handle?.trim() ||
            truncatePubkey(event.pubkey),
          createdAt: event.created_at,
          snippet: summarizeMessageLinkContent(event.content),
        };
      })
      .catch((error) =>
        isDefinitiveEventNotFound(error)
          ? ({ kind: "deleted" } as const)
          : ({ kind: "unavailable" } as const),
      );
    metadataCache.set(key, request);
    void request.then((result) => {
      if (result.kind === "unavailable" && metadataCache.get(key) === request) {
        metadataCache.delete(key);
      }
    });
  }
  return request;
}

export function useMessageLinkMetadata(
  link: ParsedMessageLink,
  channelReadable: boolean,
): { state: MessageLinkMetadataState } {
  const [resolved, setResolved] = React.useState<{
    key: string;
    state: CachedMessageLinkMetadata;
  } | null>(null);
  const requestId = React.useRef(0);
  const key = `${link.channelId}:${link.messageId}`;
  React.useEffect(() => {
    requestId.current += 1;
    if (!channelReadable) return;

    const currentRequest = requestId.current;
    const currentLink = {
      channelId: link.channelId,
      messageId: link.messageId,
    };
    void fetchMetadata(currentLink).then((metadata) => {
      if (requestId.current === currentRequest) {
        setResolved({ key, state: metadata });
      }
    });
    return () => {
      requestId.current += 1;
    };
  }, [channelReadable, key, link.channelId, link.messageId]);
  if (!channelReadable) return { state: { kind: "idle" } };
  return {
    state: resolved?.key === key ? resolved.state : { kind: "loading" },
  };
}
