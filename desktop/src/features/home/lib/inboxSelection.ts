import type { InboxItem } from "@/features/home/lib/inbox";

export function resolveInboxCompletionSelection({
  completedConversationId,
  isNarrow,
  items,
}: {
  completedConversationId: string;
  isNarrow: boolean;
  items: readonly Pick<InboxItem, "conversationId" | "id">[];
}): string | null {
  if (isNarrow) return null;

  const completedIndex = items.findIndex(
    (item) => item.conversationId === completedConversationId,
  );
  if (completedIndex === -1) return items[0]?.id ?? null;

  return (
    items
      .slice(completedIndex + 1)
      .find((item) => item.conversationId !== completedConversationId)?.id ??
    items
      .slice(0, completedIndex)
      .reverse()
      .find((item) => item.conversationId !== completedConversationId)?.id ??
    null
  );
}

export function resolveInboxFilterSelection({
  isNarrow,
  items,
  selectedConversationId,
}: {
  isNarrow: boolean;
  items: readonly Pick<InboxItem, "conversationId" | "id">[];
  selectedConversationId: string | null;
}) {
  const preserveSelection =
    selectedConversationId !== null &&
    items.some((item) => item.conversationId === selectedConversationId);

  return {
    autoSelectedEventId:
      preserveSelection || isNarrow ? null : (items[0]?.id ?? null),
    preserveSelection,
  };
}
