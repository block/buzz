import { Pencil, Save, X } from "lucide-react";
import * as React from "react";

import {
  useCanvasQuery,
  useSetCanvasMutation,
} from "@/features/channels/hooks";
import { useTranslation } from "@/i18n";
import { useChannelNavigation } from "@/shared/context/ChannelNavigationContext";
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

export function ChannelCanvas({
  channelId,
  canEdit,
  isArchived,
}: ChannelCanvasProps) {
  const { t } = useTranslation();
  const canvasQuery = useCanvasQuery(channelId, channelId !== null);
  const setCanvasMutation = useSetCanvasMutation(channelId);
  const { channels } = useChannelNavigation();
  const channelNames = React.useMemo(
    () => channels.filter((c) => c.channelType !== "dm").map((c) => c.name),
    [channels],
  );
  const [isEditing, setIsEditing] = React.useState(false);
  const [draft, setDraft] = React.useState("");

  const canvasContent = canvasQuery.data?.content ?? null;
  // Defer the single large Markdown parse so opening the canvas commits the
  // surrounding chrome immediately and the heavy render reconciles after.
  const deferredCanvasContent = React.useDeferredValue(canvasContent);

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
    return (
      <p className="text-sm text-muted-foreground">
        {t("channels.canvas.loading")}
      </p>
    );
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
          aria-label={t("channels.canvas.content-aria")}
          className="min-h-48 font-mono text-sm"
          data-testid="channel-canvas-editor"
          disabled={setCanvasMutation.isPending}
          onChange={(event) => setDraft(event.target.value)}
          placeholder={t("channels.canvas.placeholder")}
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
            {setCanvasMutation.isPending
              ? t("channels.canvas.saving")
              : t("channels.canvas.save")}
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
            {t("channels.canvas.cancel")}
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
      {canvasContent ? (
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
          {t("channels.canvas.empty")}
        </p>
      )}
      {canEdit && !isArchived ? (
        <Button
          data-testid="channel-canvas-edit"
          onClick={handleStartEditing}
          size="sm"
          type="button"
          variant="outline"
        >
          <Pencil className="h-4 w-4" />
          {canvasContent
            ? t("channels.canvas.edit")
            : t("channels.canvas.create")}
        </Button>
      ) : null}
    </div>
  );
}
