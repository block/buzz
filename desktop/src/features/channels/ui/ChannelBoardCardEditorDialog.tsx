import * as React from "react";

import {
  type CanvasBoardCard,
  type CanvasBoardCardDraft,
  validateCanvasBoardCardDraft,
} from "@/features/channels/lib/canvasBoard";
import { Button } from "@/shared/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
import { Input } from "@/shared/ui/input";
import { Textarea } from "@/shared/ui/textarea";

type ChannelBoardCardEditorDialogProps = {
  card: CanvasBoardCard | null;
  errorMessage: string | null;
  isSaving: boolean;
  onOpenChange: (open: boolean) => void;
  onSave: (draft: CanvasBoardCardDraft) => Promise<void>;
  open: boolean;
};

export function ChannelBoardCardEditorDialog({
  card,
  errorMessage,
  isSaving,
  onOpenChange,
  onSave,
  open,
}: ChannelBoardCardEditorDialogProps) {
  const titleId = React.useId();
  const bodyId = React.useId();
  const [title, setTitle] = React.useState("");
  const [body, setBody] = React.useState("");
  const draft = React.useMemo(() => ({ body, title }), [body, title]);
  const validationError = validateCanvasBoardCardDraft(draft);

  React.useEffect(() => {
    if (!open) {
      return;
    }
    setTitle(card?.title ?? "");
    setBody(card?.body ?? "");
  }, [card?.body, card?.title, open]);

  async function handleSave(event: React.FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (validationError || isSaving) {
      return;
    }

    try {
      await onSave({ body: body.trim(), title: title.trim() });
    } catch {
      // The mutation error stays visible in the dialog for a safe retry.
    }
  }

  const isEditing = card !== null;

  return (
    <Dialog
      onOpenChange={(nextOpen) => {
        if (!isSaving) {
          onOpenChange(nextOpen);
        }
      }}
      open={open}
    >
      <DialogContent
        className="max-h-[calc(100vh-2rem)] max-w-xl overflow-y-auto"
        data-testid="magic-board-card-editor"
      >
        <DialogHeader>
          <DialogTitle>{isEditing ? "Edit card" : "Create card"}</DialogTitle>
          <DialogDescription>
            Cards are shared Markdown sections. Use level-three headings inside
            the body; level-two headings start separate cards.
          </DialogDescription>
        </DialogHeader>

        <form className="space-y-5" onSubmit={handleSave}>
          <div className="space-y-2">
            <label className="text-sm font-medium" htmlFor={titleId}>
              Title
            </label>
            <Input
              autoFocus
              data-testid="magic-board-card-title"
              disabled={isSaving}
              id={titleId}
              maxLength={120}
              onChange={(event) => setTitle(event.target.value)}
              placeholder="What should people notice?"
              value={title}
            />
          </div>

          <div className="space-y-2">
            <label className="text-sm font-medium" htmlFor={bodyId}>
              Body
            </label>
            <Textarea
              className="min-h-56 font-mono text-sm"
              data-testid="magic-board-card-body"
              disabled={isSaving}
              id={bodyId}
              onChange={(event) => setBody(event.target.value)}
              placeholder="Write the card in Markdown..."
              value={body}
            />
          </div>

          {validationError || errorMessage ? (
            <p
              className="rounded-lg border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive"
              data-testid="magic-board-card-error"
            >
              {validationError ?? errorMessage}
            </p>
          ) : null}

          <DialogFooter>
            <Button
              disabled={isSaving}
              onClick={() => onOpenChange(false)}
              type="button"
              variant="outline"
            >
              Cancel
            </Button>
            <Button
              data-testid="magic-board-card-save"
              disabled={Boolean(validationError) || isSaving}
              type="submit"
            >
              {isSaving ? "Saving..." : isEditing ? "Save card" : "Create card"}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
