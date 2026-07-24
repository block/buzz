import type { InboxItem } from "@/features/home/lib/inbox";

export function resolveActivityFilterSelection({
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
