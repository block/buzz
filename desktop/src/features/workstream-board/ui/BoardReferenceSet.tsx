import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { parseBoardReferences } from "@/features/workstream-board/lib/boardReferences";
import { BoardReferenceItems } from "@/features/workstream-board/ui/BoardReferenceItems";

export const BOARD_REPLAY_STORAGE_KEY = "buzz:board-replay:v1";

export function BoardReferenceSet({
  channelId,
  tags,
}: {
  channelId?: string | null;
  tags?: readonly (readonly string[])[];
}) {
  const references = parseBoardReferences(tags, channelId ?? undefined);
  const { goWorkstreams } = useAppNavigation();
  if (references.length === 0) return null;
  return (
    <button
      className="mt-2 flex w-full flex-wrap gap-1.5 rounded-lg border bg-muted/40 p-2 text-left"
      data-testid="message-board-references"
      onClick={() => {
        sessionStorage.setItem(
          BOARD_REPLAY_STORAGE_KEY,
          JSON.stringify(references),
        );
        void goWorkstreams();
      }}
      type="button"
    >
      <BoardReferenceItems references={references} />
    </button>
  );
}
