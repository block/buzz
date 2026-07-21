import { useQuery } from "@tanstack/react-query";

import { getEventById } from "@/shared/api/tauri";
import type { SentMessageLink } from "./messageLinks";

export function useSentMessageBody(
  messageLink: SentMessageLink | null,
  inlineContent: string | null,
): string | null {
  const shouldFetch = messageLink !== null && inlineContent === null;
  const { data } = useQuery({
    queryKey: ["sent-message-body", messageLink?.messageId],
    queryFn: () => getEventById(messageLink?.messageId ?? ""),
    enabled: shouldFetch,
    staleTime: Number.POSITIVE_INFINITY,
  });

  if (inlineContent) return inlineContent;
  return data?.content ?? null;
}
