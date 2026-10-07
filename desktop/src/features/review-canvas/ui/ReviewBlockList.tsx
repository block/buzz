import { cn } from "@/shared/lib/cn";
import type { ReviewBlock } from "../lib/reviewContract";

type ReviewBlockListProps = {
  blocks: ReviewBlock[];
  selectedBlockId: string | null;
  /** Feedback already attached to each block of this revision. */
  feedbackCounts: Record<string, number>;
  /**
   * Whether commenting on a block is unavailable (stale revision, no agent). A
   * block holding a signed draft stays available so it can be retried.
   */
  disabled: (blockId: string) => boolean;
  onSelect: (id: string, trigger: HTMLElement) => void;
};

/**
 * The keyboard- and pointer-equivalent way to pick a review block from trusted
 * chrome. It exposes exactly the blocks the document declared, so it works
 * even when the in-frame annotator cannot start.
 */
export function ReviewBlockList({
  blocks,
  selectedBlockId,
  feedbackCounts,
  disabled,
  onSelect,
}: ReviewBlockListProps) {
  if (blocks.length === 0) {
    return (
      <p className="text-xs text-muted-foreground">
        This revision declares no reviewable blocks.
      </p>
    );
  }
  return (
    <ul aria-label="Review blocks" className="flex flex-col gap-1">
      {blocks.map((block) => {
        const selected = block.id === selectedBlockId;
        const count = feedbackCounts[block.id] ?? 0;
        return (
          <li key={block.id}>
            <button
              aria-disabled={disabled(block.id)}
              aria-pressed={selected}
              className={cn(
                "flex w-full items-center justify-between gap-2 rounded-lg px-2.5 py-1.5 text-left text-sm transition-colors hover:bg-muted/70 focus-visible:outline-hidden focus-visible:ring-1 focus-visible:ring-ring aria-disabled:cursor-not-allowed aria-disabled:opacity-60",
                selected && "bg-muted",
              )}
              data-testid={`review-block-${block.id}`}
              id={`review-block-${block.id}`}
              onClick={(event) => onSelect(block.id, event.currentTarget)}
              type="button"
            >
              <span className="min-w-0">
                <span className="block truncate font-medium">
                  {block.title}
                </span>
                {block.sourceRef ? (
                  <span className="block truncate font-mono text-xs text-muted-foreground">
                    {block.sourceRef}
                  </span>
                ) : null}
              </span>
              {count > 0 ? (
                <span className="shrink-0 rounded-full bg-primary/10 px-2 py-0.5 text-xs text-primary">
                  {count} {count === 1 ? "comment" : "comments"}
                </span>
              ) : null}
            </button>
          </li>
        );
      })}
    </ul>
  );
}
