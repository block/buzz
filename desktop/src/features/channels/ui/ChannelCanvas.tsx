import { History, Pencil, Save, X } from "lucide-react";
import * as React from "react";

import {
  useCanvasQuery,
  useSetCanvasMutation,
} from "@/features/channels/hooks";
import { useChannelNavigation } from "@/shared/context/ChannelNavigationContext";
import { Button } from "@/shared/ui/button";
import { Markdown } from "@/shared/ui/markdown";
import { Textarea } from "@/shared/ui/textarea";
import {
  isRelayUnreachableError,
  RELAY_UNREACHABLE_SHORT,
} from "@/shared/lib/relayError";
import {
  CANVAS_EXPECTED_REVISION_NONE,
  canvasConflictMessage,
} from "@/features/channels/canvasConflict";
import { CanvasHistoryPanel } from "./CanvasHistoryPanel";

type ChannelCanvasProps = {
  channelId: string | null;
  canEdit: boolean;
  isArchived: boolean;
};

export function ChannelCanvas({
  channelId,
  canEdit,
  isArchived,
}: ChannelCanvasProps) {
  const canvasQuery = useCanvasQuery(channelId, channelId !== null);
  const setCanvasMutation = useSetCanvasMutation(channelId);
  const { channels } = useChannelNavigation();
  const channelNames = React.useMemo(
    () => channels.filter((c) => c.channelType !== "dm").map((c) => c.name),
    [channels],
  );
  const [isEditing, setIsEditing] = React.useState(false);
  const [showHistory, setShowHistory] = React.useState(false);
  const [draft, setDraft] = React.useState("");
  // Non-destructive notice shown after a save the relay accepted but could not
  // verify (the post-write supersession read failed). The write is durable; the
  // note tells the user to check History if a concurrent edit later appears.
  // Cleared whenever a new edit session starts.
  const [unverifiedSaveNotice, setUnverifiedSaveNotice] = React.useState(false);
  // Head event id captured at edit-start. A background canvas refetch can move
  // the live head mid-edit; the save must assert against what the editor
  // actually loaded, not the latest head. `null` means "no canvas existed when
  // I started" and maps to the `none` create-race sentinel below.
  const [editBaseRevision, setEditBaseRevision] = React.useState<string | null>(
    null,
  );
  // After a save settles the focused Save button unmounts with the editor. To
  // keep keyboard focus from falling back to the document body, we move it to
  // the most informative surviving destination: the unverified notice when it
  // renders, otherwise the Edit button next to the canvas. `pendingSaveFocus`
  // arms the move; the effect below runs it once the non-editing tree paints.
  const noticeRef = React.useRef<HTMLParagraphElement | null>(null);
  const editButtonRef = React.useRef<HTMLButtonElement | null>(null);
  const [pendingSaveFocus, setPendingSaveFocus] = React.useState(false);
  React.useEffect(() => {
    if (pendingSaveFocus && !isEditing) {
      (noticeRef.current ?? editButtonRef.current)?.focus();
      setPendingSaveFocus(false);
    }
  }, [pendingSaveFocus, isEditing]);

  const canvasContent = canvasQuery.data?.content ?? null;
  const canvasRevision = canvasQuery.data?.eventId ?? null;
  // A canvas exists whenever a persisted revision is present — an empty-string
  // revision is a valid kind:40100 write (restore can republish one), so
  // existence, the Create/Edit label, and History must key off the revision id,
  // not content truthiness.
  const canvasExists = canvasRevision !== null;
  // Defer the single large Markdown parse so opening the canvas commits the
  // surrounding chrome immediately and the heavy render reconciles after.
  const deferredCanvasContent = React.useDeferredValue(canvasContent);

  function handleStartEditing() {
    // Clear any prior rejected-save error so it can't render in the fresh
    // editor session — the mutation state persists across edit sessions and
    // the editor renders `setCanvasMutation.error` whenever it opens.
    setCanvasMutation.reset();
    setDraft(canvasContent ?? "");
    setEditBaseRevision(canvasRevision);
    setUnverifiedSaveNotice(false);
    setIsEditing(true);
  }

  function handleCancelEditing() {
    setIsEditing(false);
    setDraft("");
  }

  async function handleSave() {
    // Assert against the head snapshotted at edit-start, not the live head —
    // a refetch may have moved `canvasRevision` while the editor was open.
    // A null snapshot means no canvas existed then, so send the `none`
    // sentinel to close the concurrent-first-creation race.
    const result = await setCanvasMutation.mutateAsync({
      content: draft,
      expectedRevision: editBaseRevision ?? CANVAS_EXPECTED_REVISION_NONE,
    });
    // The write was accepted. `verified: false` means the post-write
    // supersession read failed, not that the save failed — close the editor and
    // surface a non-destructive note rather than a conflict. A detected
    // supersession is a rejected promise handled by the catch in the click
    // wiring below, so it never reaches here.
    setUnverifiedSaveNotice(!result.verified);
    setIsEditing(false);
    setPendingSaveFocus(true);
  }

  if (canvasQuery.isLoading) {
    return (
      <p className="text-sm text-muted-foreground" role="status">
        Loading canvas...
      </p>
    );
  }

  // An initial load failure with no cached data: surface the full error state.
  // A failed background refetch (data is defined, error is also set) must not
  // replace the cached canvas and accepted-write notice — show a non-destructive
  // refresh warning inline instead.
  if (canvasQuery.error instanceof Error && canvasQuery.data === undefined) {
    return (
      <p
        className="rounded-xl border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive"
        role="alert"
      >
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
          <p className="text-sm text-destructive" role="alert">
            {canvasConflictMessage(setCanvasMutation.error) ??
              setCanvasMutation.error.message}
          </p>
        ) : null}
      </div>
    );
  }

  return (
    <div className="space-y-3">
      {canvasQuery.error instanceof Error ? (
        <p
          aria-live="polite"
          className="rounded-xl border border-border/70 bg-muted/20 px-3 py-2 text-sm text-muted-foreground"
          data-testid="channel-canvas-refresh-error"
          role="status"
        >
          {isRelayUnreachableError(canvasQuery.error)
            ? RELAY_UNREACHABLE_SHORT
            : "Couldn't refresh canvas — showing last known content."}
        </p>
      ) : null}
      {unverifiedSaveNotice ? (
        <p
          aria-live="polite"
          className="rounded-xl border border-border/70 bg-muted/20 px-3 py-2 text-sm text-muted-foreground"
          data-testid="channel-canvas-unverified-notice"
          role="status"
          ref={noticeRef}
          tabIndex={-1}
        >
          Saved. We couldn't verify against the latest revision just now — check
          History if a concurrent edit appears.
        </p>
      ) : null}
      {canvasExists ? (
        <div
          className="rounded-2xl border border-border/70 bg-muted/20 px-4 py-3"
          data-testid="channel-canvas-content"
        >
          <Markdown
            channelNames={channelNames}
            content={deferredCanvasContent ?? ""}
          />
        </div>
      ) : (
        <p className="text-sm text-muted-foreground">
          No canvas set for this channel.
        </p>
      )}
      {canEdit && !isArchived ? (
        <Button
          data-testid="channel-canvas-edit"
          onClick={handleStartEditing}
          ref={editButtonRef}
          size="sm"
          type="button"
          variant="outline"
        >
          <Pencil className="h-4 w-4" />
          {canvasExists ? "Edit canvas" : "Create canvas"}
        </Button>
      ) : null}
      {canvasExists ? (
        <>
          <Button
            aria-expanded={showHistory}
            data-testid="channel-canvas-history-toggle"
            onClick={() => setShowHistory((open) => !open)}
            size="sm"
            type="button"
            variant="ghost"
          >
            <History className="h-4 w-4" />
            {showHistory ? "Hide history" : "History"}
          </Button>
          {showHistory && channelId ? (
            <CanvasHistoryPanel
              canRestore={canEdit && !isArchived}
              channelId={channelId}
              currentContent={canvasContent ?? ""}
              currentRevision={canvasRevision}
            />
          ) : null}
        </>
      ) : null}
    </div>
  );
}
