import type { TimelineMessage } from "@/features/messages/types";

export function ProjectedThreadContextLine({
  message,
  onOpenThread,
  rootAuthor,
}: {
  message: TimelineMessage;
  onOpenThread?: (message: TimelineMessage) => void;
  rootAuthor?: string | null;
}) {
  if (!rootAuthor) return null;

  return (
    <div
      className="mb-1 flex min-h-[var(--inline-chip-min-height)] min-w-0 flex-wrap items-center gap-x-1.5 gap-y-0.5 pt-0.5 text-sm font-normal leading-4 text-muted-foreground/70"
      data-testid="projected-thread-context"
    >
      <span className="min-w-0 truncate">Replying to {rootAuthor}</span>
      {onOpenThread ? (
        <>
          <span aria-hidden="true" className="text-muted-foreground/45">
            ·
          </span>
          <button
            className="rounded-sm font-medium text-muted-foreground transition-colors hover:text-foreground focus-visible:outline-hidden focus-visible:ring-1 focus-visible:ring-ring"
            data-testid="projected-thread-open"
            onClick={() => onOpenThread(message)}
            type="button"
          >
            View original thread
          </button>
        </>
      ) : null}
    </div>
  );
}
