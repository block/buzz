import type { BoardReference } from "@/features/workstream-board/lib/boardReferences";
import { boardReferenceKey } from "@/features/workstream-board/lib/boardReferences";
import { cn } from "@/shared/lib/cn";

export function BoardReferenceItems({
  references,
  onRemove,
}: {
  references: readonly BoardReference[];
  onRemove?: (reference: BoardReference) => void;
}) {
  return (
    <ol className="flex flex-wrap gap-1.5" data-testid="board-reference-items">
      {references.map((reference, index) => (
        <li
          className="inline-flex items-center gap-1 rounded-full border bg-background px-2 py-1 text-3xs"
          data-reference-key={boardReferenceKey(reference)}
          key={boardReferenceKey(reference)}
        >
          <span>
            {index + 1}. {reference.kind}: {reference.snapshot.label}
          </span>
          {onRemove ? (
            <button
              aria-label={`Remove ${reference.snapshot.label}`}
              className={cn("font-semibold underline")}
              onClick={() => onRemove(reference)}
              type="button"
            >
              Remove
            </button>
          ) : null}
        </li>
      ))}
    </ol>
  );
}
