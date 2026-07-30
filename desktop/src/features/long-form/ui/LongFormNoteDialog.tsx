import { Loader2, RotateCw } from "lucide-react";

import { useUserProfileQuery } from "@/features/profile/hooks";
import { UserProfilePopover } from "@/features/profile/ui/UserProfilePopover";
import { truncatePubkey } from "@/shared/lib/pubkey";
import { Button } from "@/shared/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
import { Markdown } from "@/shared/ui/markdown";

import { useLongFormNoteQuery } from "../hooks";
import type { LongFormAddress } from "../lib/nostrAddress";

function tagValue(tags: string[][], name: string): string | null {
  return tags.find((tag) => tag[0] === name)?.[1]?.trim() || null;
}

function formatPublishedAt(unixSeconds: number): string {
  return new Date(unixSeconds * 1_000).toLocaleDateString(undefined, {
    year: "numeric",
    month: "short",
    day: "numeric",
  });
}

export function LongFormNoteDialog({
  address,
  onOpenChange,
  open,
}: {
  address: LongFormAddress;
  onOpenChange: (open: boolean) => void;
  open: boolean;
}) {
  const noteQuery = useLongFormNoteQuery(address, open);
  const profileQuery = useUserProfileQuery(open ? address.pubkey : undefined);
  const note = noteQuery.data;
  const displayName =
    profileQuery.data?.displayName || truncatePubkey(address.pubkey);
  const title = note
    ? tagValue(note.tags, "title") || address.identifier
    : "Long-form note";
  const publishedAt = note
    ? Number(tagValue(note.tags, "published_at")) || note.createdAt
    : null;

  return (
    <Dialog onOpenChange={onOpenChange} open={open}>
      <DialogContent
        className="max-h-[calc(100vh-2rem)] grid-rows-[auto_minmax(0,1fr)]"
        data-testid="long-form-note-dialog"
      >
        <DialogHeader className="pr-8">
          <DialogTitle data-testid="long-form-note-title">{title}</DialogTitle>
          <DialogDescription asChild>
            <div className="flex flex-wrap items-center gap-x-2 gap-y-1">
              <span>Long-form note by</span>
              <UserProfilePopover pubkey={address.pubkey}>
                <button
                  className="rounded font-medium text-foreground hover:underline focus-visible:outline-hidden focus-visible:ring-1 focus-visible:ring-ring"
                  type="button"
                >
                  {displayName}
                </button>
              </UserProfilePopover>
              {publishedAt ? (
                <span>· {formatPublishedAt(publishedAt)}</span>
              ) : null}
            </div>
          </DialogDescription>
        </DialogHeader>

        <div className="min-h-40 overflow-y-auto pr-2">
          {noteQuery.isPending ? (
            <div
              className="flex min-h-40 items-center justify-center gap-2 text-sm text-muted-foreground"
              data-testid="long-form-note-loading"
            >
              <Loader2 className="h-4 w-4 animate-spin" />
              Loading long-form note…
            </div>
          ) : noteQuery.isError ? (
            <div
              className="flex min-h-40 flex-col items-center justify-center gap-3 text-center"
              data-testid="long-form-note-error"
            >
              <div>
                <p className="text-sm font-medium text-foreground">
                  Couldn&apos;t load this note
                </p>
                <p className="mt-1 text-sm text-muted-foreground">
                  Check the community connection and try again.
                </p>
              </div>
              <Button
                disabled={noteQuery.isFetching}
                onClick={() => {
                  void noteQuery.refetch();
                }}
                size="sm"
                type="button"
                variant="outline"
              >
                {noteQuery.isFetching ? (
                  <Loader2 className="h-4 w-4 animate-spin" />
                ) : (
                  <RotateCw className="h-4 w-4" />
                )}
                Retry
              </Button>
            </div>
          ) : note === null ? (
            <div
              className="flex min-h-40 flex-col items-center justify-center text-center"
              data-testid="long-form-note-not-found"
            >
              <p className="text-sm font-medium text-foreground">
                Not found in this community
              </p>
              <p className="mt-1 max-w-sm text-sm text-muted-foreground">
                The address may point to a note that has not been published to
                this community.
              </p>
            </div>
          ) : note ? (
            <article
              className="pb-2 text-base text-foreground"
              data-testid="long-form-note-content"
            >
              {note.content ? (
                <Markdown content={note.content} />
              ) : (
                <p className="text-sm text-muted-foreground">
                  This note has no content.
                </p>
              )}
            </article>
          ) : null}
        </div>
      </DialogContent>
    </Dialog>
  );
}
