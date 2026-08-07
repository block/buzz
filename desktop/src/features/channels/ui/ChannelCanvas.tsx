import { Pencil, Save, X } from "lucide-react";
import * as React from "react";

import {
  useCanvasHistoryQuery,
  useSetCanvasMutation,
} from "@/features/channels/hooks";
import { useUsersBatchQuery } from "@/features/profile/hooks";
import { useChannelNavigation } from "@/shared/context/ChannelNavigationContext";
import type { CanvasRevision } from "@/shared/api/canvasTypes";
import { resolveUserLabel } from "@/features/profile/lib/identity";
import { Button } from "@/shared/ui/button";
import { Markdown } from "@/shared/ui/markdown";
import { Textarea } from "@/shared/ui/textarea";
import {
  isRelayUnreachableError,
  RELAY_UNREACHABLE_SHORT,
} from "@/shared/lib/relayError";

type ChannelCanvasProps = {
  channelId: string | null;
  canEdit: boolean;
  isArchived: boolean;
};

const revisionDateFormatter = new Intl.DateTimeFormat(undefined, {
  dateStyle: "medium",
  timeStyle: "short",
});

function formatRevisionTimestamp(timestamp: number) {
  return revisionDateFormatter.format(new Date(timestamp * 1_000));
}

function isCurrentRevision(
  revision: CanvasRevision,
  currentRevision: CanvasRevision | null,
) {
  return revision.eventId === currentRevision?.eventId;
}

export function ChannelCanvas({
  channelId,
  canEdit,
  isArchived,
}: ChannelCanvasProps) {
  const canvasQuery = useCanvasHistoryQuery(channelId, channelId !== null);
  const setCanvasMutation = useSetCanvasMutation(channelId);
  const { channels } = useChannelNavigation();
  const channelNames = React.useMemo(
    () => channels.filter((c) => c.channelType !== "dm").map((c) => c.name),
    [channels],
  );
  const [isEditing, setIsEditing] = React.useState(false);
  const [draft, setDraft] = React.useState("");
  const [selectedRevisionId, setSelectedRevisionId] = React.useState<
    string | null
  >(null);

  const revisions = canvasQuery.data ?? [];
  const currentRevision = revisions[0] ?? null;
  const selectedRevision =
    revisions.find((revision) => revision.eventId === selectedRevisionId) ??
    currentRevision;
  const viewingHistoricalRevision =
    selectedRevision !== null &&
    !isCurrentRevision(selectedRevision, currentRevision);
  const revisionAuthors = React.useMemo(
    () => [...new Set(revisions.map((revision) => revision.author))],
    [revisions],
  );
  const profilesQuery = useUsersBatchQuery(revisionAuthors, {
    enabled: revisionAuthors.length > 0,
  });
  const profiles = profilesQuery.data?.profiles;
  const canvasContent = currentRevision?.content ?? null;
  const displayedContent = selectedRevision?.content ?? null;
  // Defer the single large Markdown parse so opening the canvas commits the
  // surrounding chrome immediately and the heavy render reconciles after.
  const deferredCanvasContent = React.useDeferredValue(displayedContent);

  React.useEffect(() => {
    if (!currentRevision) {
      setSelectedRevisionId(null);
      return;
    }

    if (
      selectedRevisionId === null ||
      !revisions.some((revision) => revision.eventId === selectedRevisionId)
    ) {
      setSelectedRevisionId(currentRevision.eventId);
    }
  }, [currentRevision, revisions, selectedRevisionId]);

  function handleStartEditing() {
    setDraft(canvasContent ?? "");
    setIsEditing(true);
  }

  function handleCancelEditing() {
    setIsEditing(false);
    setDraft("");
  }

  async function handleSave() {
    await setCanvasMutation.mutateAsync(draft);
    setIsEditing(false);
  }

  if (canvasQuery.isLoading) {
    return <p className="text-sm text-muted-foreground">Loading canvas...</p>;
  }

  if (canvasQuery.error instanceof Error) {
    return (
      <p className="rounded-xl border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive">
        {isRelayUnreachableError(canvasQuery.error)
          ? RELAY_UNREACHABLE_SHORT
          : canvasQuery.error.message}
      </p>
    );
  }

  if (isEditing) {
    return (
      <div className="space-y-3">
        <Textarea
          aria-label="Canvas content"
          className="min-h-48 font-mono text-sm"
          data-testid="channel-canvas-editor"
          disabled={setCanvasMutation.isPending}
          onChange={(event) => setDraft(event.target.value)}
          placeholder="Write your canvas content in Markdown..."
          value={draft}
        />
        <div className="flex gap-2">
          <Button
            data-testid="channel-canvas-save"
            disabled={setCanvasMutation.isPending}
            onClick={() => {
              void handleSave().catch(() => {
                // Error is already surfaced via setCanvasMutation.error
              });
            }}
            size="sm"
            type="button"
          >
            <Save className="h-4 w-4" />
            {setCanvasMutation.isPending ? "Saving..." : "Save canvas"}
          </Button>
          <Button
            data-testid="channel-canvas-cancel"
            disabled={setCanvasMutation.isPending}
            onClick={handleCancelEditing}
            size="sm"
            type="button"
            variant="outline"
          >
            <X className="h-4 w-4" />
            Cancel
          </Button>
        </div>
        {setCanvasMutation.error instanceof Error ? (
          <p className="text-sm text-destructive">
            {setCanvasMutation.error.message}
          </p>
        ) : null}
      </div>
    );
  }

  return (
    <div className="space-y-3">
      {selectedRevision ? (
        <div
          className="rounded-2xl border border-border/70 bg-muted/20 px-4 py-3"
          data-testid="channel-canvas-content"
        >
          {viewingHistoricalRevision ? (
            <div className="mb-3 flex items-center justify-between gap-3 rounded-xl border border-border/60 bg-background/50 px-3 py-2 text-sm">
              <span className="text-muted-foreground">
                Viewing an earlier revision (read-only).
              </span>
              <Button
                data-testid="channel-canvas-current-revision"
                onClick={() =>
                  setSelectedRevisionId(currentRevision?.eventId ?? null)
                }
                size="sm"
                type="button"
                variant="outline"
              >
                Return to current
              </Button>
            </div>
          ) : null}
          <Markdown
            channelNames={channelNames}
            content={deferredCanvasContent ?? ""}
          />
        </div>
      ) : (
        <p
          className="text-sm text-muted-foreground"
          data-testid="channel-canvas-history-empty"
        >
          No revision history yet — save the canvas to start tracking changes.
        </p>
      )}
      {selectedRevision && currentRevision ? (
        <div
          className="flex flex-wrap items-center gap-x-2 gap-y-1 text-sm text-muted-foreground"
          data-testid="channel-canvas-current-metadata"
        >
          <span>Updated by</span>
          <span className="font-medium text-foreground">
            {resolveUserLabel({ pubkey: currentRevision.author, profiles })}
          </span>
          <span aria-hidden="true">·</span>
          <time
            dateTime={new Date(currentRevision.updatedAt * 1_000).toISOString()}
          >
            {formatRevisionTimestamp(currentRevision.updatedAt)}
          </time>
        </div>
      ) : null}
      {revisions.length > 1 ? (
        <section
          aria-label="Canvas revision history"
          className="space-y-2 rounded-2xl border border-border/70 px-3 py-3"
          data-testid="channel-canvas-history"
        >
          <h3 className="text-sm font-semibold">Revision history</h3>
          <div className="space-y-1">
            {revisions.map((revision) => {
              const current = isCurrentRevision(revision, currentRevision);
              const selected = revision.eventId === selectedRevision?.eventId;
              return (
                <button
                  aria-current={selected ? "page" : undefined}
                  className={`flex w-full items-start justify-between gap-3 rounded-xl px-3 py-2 text-left text-sm transition-colors ${selected ? "bg-muted" : "hover:bg-muted/60"}`}
                  data-testid={
                    current
                      ? "channel-canvas-revision-current"
                      : `channel-canvas-revision-${revision.eventId}`
                  }
                  key={revision.eventId}
                  onClick={() => setSelectedRevisionId(revision.eventId)}
                  type="button"
                >
                  <span className="min-w-0">
                    <span className="block font-medium">
                      {current ? "Current revision" : "Earlier revision"}
                    </span>
                    <span className="block truncate text-muted-foreground">
                      {resolveUserLabel({ pubkey: revision.author, profiles })}
                    </span>
                  </span>
                  <time
                    className="shrink-0 text-xs text-muted-foreground"
                    dateTime={new Date(
                      revision.updatedAt * 1_000,
                    ).toISOString()}
                  >
                    {formatRevisionTimestamp(revision.updatedAt)}
                  </time>
                </button>
              );
            })}
          </div>
        </section>
      ) : null}
      {canEdit && !isArchived && !viewingHistoricalRevision ? (
        <Button
          data-testid="channel-canvas-edit"
          onClick={handleStartEditing}
          size="sm"
          type="button"
          variant="outline"
        >
          <Pencil className="h-4 w-4" />
          {canvasContent ? "Edit canvas" : "Create canvas"}
        </Button>
      ) : null}
    </div>
  );
}
