import { MessagesSquare } from "lucide-react";

import { useThreadTitleForRoot } from "@/features/threads/hooks";
import { cn } from "@/shared/lib/cn";

/**
 * A thread's shared title on its own line, shown under the author of the
 * thread root (channel) or under the context label (Inbox). Renders nothing
 * for an untitled thread. With `onOpen` it is a button that opens the thread.
 */
export function ThreadTitleLine({
  className,
  onOpen,
  rootId,
}: {
  className?: string;
  onOpen?: () => void;
  rootId: string | null;
}) {
  const title = useThreadTitleForRoot(rootId);
  if (!title) {
    return null;
  }

  const content = (
    <>
      <MessagesSquare
        aria-hidden="true"
        className="h-3.5 w-3.5 shrink-0 text-muted-foreground"
      />
      <span className="min-w-0 truncate">{title}</span>
    </>
  );
  const lineClass = cn(
    "flex min-w-0 max-w-full items-center gap-1.5 text-sm font-semibold leading-5 text-foreground",
    className,
  );

  if (!onOpen) {
    return (
      <div className={lineClass} data-testid="thread-title-line">
        {content}
      </div>
    );
  }
  return (
    <button
      aria-label={`Open thread "${title}"`}
      className={cn(
        lineClass,
        "w-fit rounded text-left hover:underline focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring",
      )}
      data-testid="thread-title-line"
      onClick={onOpen}
      type="button"
    >
      {content}
    </button>
  );
}
