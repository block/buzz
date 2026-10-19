import { Pencil } from "lucide-react";
import * as React from "react";
import { toast } from "sonner";

import {
  useSetThreadTitleMutation,
  useThreadTitle,
} from "@/features/threads/hooks";
import {
  interpretTitleDraft,
  MAX_THREAD_TITLE_BYTES,
} from "@/features/threads/lib/threadTitles";

export type ThreadTitleTarget = { channelId: string; rootId: string };

/** The title and revision an edit started from. */
type EditSession = {
  draft: string;
  startTitle: string | null;
  startRevision: string | null;
};

/**
 * Thread panel title: shows the shared thread title (falling back to the
 * panel's default label) with an inline editor. Blank input clears the title.
 * When the fallback is navigational context (e.g. a project panel's channel
 * link), it stays visible after the title.
 */
export function ThreadTitleEditor({
  fallback,
  keepFallback = false,
  target,
}: {
  fallback: React.ReactNode;
  keepFallback?: boolean;
  target: ThreadTitleTarget;
}) {
  const shared = useThreadTitle(target.channelId, target.rootId);
  const mutation = useSetThreadTitleMutation();
  const [session, setSession] = React.useState<EditSession | null>(null);
  const inputRef = React.useRef<HTMLInputElement>(null);
  // Enter commits and unmounts the input, which can also fire blur; commit once.
  const committedRef = React.useRef(false);
  const currentTitle = shared?.title ?? null;
  const isEditing = session !== null;

  React.useEffect(() => {
    if (isEditing) {
      committedRef.current = false;
      inputRef.current?.focus();
    }
  }, [isEditing]);

  const startEditing = () => {
    setSession({
      draft: currentTitle ?? "",
      startTitle: currentTitle,
      // An optimistic, not-yet-confirmed write has no revision to expect.
      startRevision: shared?.revision || null,
    });
  };

  const commit = () => {
    if (session === null || committedRef.current) {
      return;
    }
    committedRef.current = true;
    setSession(null);
    // Compare with the title the edit started from, so an untouched editor
    // never re-sends a stale title over someone else's rename.
    const result = interpretTitleDraft(session.draft, session.startTitle);
    if (result.kind === "unchanged") {
      return;
    }
    if (result.kind === "too-long") {
      toast.error(
        `Thread titles are limited to ${MAX_THREAD_TITLE_BYTES} bytes.`,
      );
      return;
    }
    mutation.mutate({
      channelId: target.channelId,
      expectedRevision: session.startRevision,
      rootId: target.rootId,
      title: result.kind === "set" ? result.title : null,
    });
  };

  if (session !== null) {
    return (
      <input
        aria-label="Thread title"
        className="h-7 w-full min-w-0 rounded-md border border-input bg-background px-2 text-sm font-semibold leading-6 outline-none focus-visible:ring-1 focus-visible:ring-ring"
        data-testid="thread-title-input"
        onBlur={commit}
        onChange={(event) =>
          setSession({ ...session, draft: event.target.value })
        }
        onFocus={(event) => event.currentTarget.select()}
        onKeyDown={(event) => {
          if (event.nativeEvent.isComposing) {
            return;
          }
          if (event.key === "Enter") {
            event.preventDefault();
            commit();
          } else if (event.key === "Escape") {
            event.preventDefault();
            event.stopPropagation();
            committedRef.current = true;
            setSession(null);
          }
        }}
        placeholder="Name this thread"
        ref={inputRef}
        value={session.draft}
      />
    );
  }

  return (
    <span className="group/thread-title flex min-w-0 items-center gap-1">
      <span
        className="min-w-0 truncate"
        data-testid="thread-title"
        title={currentTitle ?? undefined}
      >
        {currentTitle ?? fallback}
      </span>
      <button
        aria-label={currentTitle ? "Rename thread" : "Name thread"}
        className="inline-flex h-6 w-6 shrink-0 items-center justify-center rounded-md text-muted-foreground opacity-0 transition-opacity hover:bg-accent hover:text-foreground focus-visible:opacity-100 group-hover/thread-title:opacity-100"
        data-testid="thread-title-edit"
        onClick={startEditing}
        title={currentTitle ? "Rename thread" : "Name thread"}
        type="button"
      >
        <Pencil className="h-3.5 w-3.5" />
      </button>
      {currentTitle && keepFallback ? (
        <span className="min-w-0 shrink truncate text-sm font-normal text-muted-foreground">
          {fallback}
        </span>
      ) : null}
    </span>
  );
}
