import * as React from "react";

import {
  type CanvasBoardCard,
  type CanvasBoardCardDraft,
  type CanvasBoardCardStatus,
  type CanvasBoardCardType,
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

const CARD_TYPE_OPTIONS: Array<{
  label: string;
  value: CanvasBoardCardType;
}> = [
  { label: "Note", value: "note" },
  { label: "Task", value: "task" },
  { label: "Decision", value: "decision" },
  { label: "Conversation", value: "conversation" },
  { label: "Project", value: "project" },
  { label: "Artifact", value: "artifact" },
  { label: "Person", value: "person" },
  { label: "Agent", value: "agent" },
];

const CARD_STATUS_OPTIONS: Array<{
  label: string;
  value: CanvasBoardCardStatus;
}> = [
  { label: "Backlog", value: "backlog" },
  { label: "Doing", value: "doing" },
  { label: "Done", value: "done" },
];

const SELECT_CLASS_NAME =
  "flex h-10 w-full rounded-md border border-input bg-background px-3 py-2 text-sm ring-offset-background focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 disabled:cursor-not-allowed disabled:opacity-50";

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
  const [type, setType] = React.useState<CanvasBoardCardType>("note");
  const [status, setStatus] = React.useState<CanvasBoardCardStatus>("backlog");
  const draft = React.useMemo(
    () => ({ body, status, title, type }),
    [body, status, title, type],
  );
  const validationError = validateCanvasBoardCardDraft(draft);

  React.useEffect(() => {
    if (!open) {
      return;
    }
    setTitle(card?.title ?? "");
    setBody(card?.body ?? "");
    setType(card?.type ?? "note");
    setStatus(card?.status ?? "backlog");
  }, [card?.body, card?.status, card?.title, card?.type, open]);

  async function handleSave(event: React.FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (validationError || isSaving) {
      return;
    }

    try {
      await onSave({
        body: body.trim(),
        status,
        title: title.trim(),
        type,
      });
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

          <div className="grid gap-4 sm:grid-cols-2">
            <div className="space-y-2">
              <label
                className="text-sm font-medium"
                htmlFor={`${titleId}-type`}
              >
                Type
              </label>
              <select
                className={SELECT_CLASS_NAME}
                data-testid="magic-board-card-type"
                disabled={isSaving}
                id={`${titleId}-type`}
                onChange={(event) =>
                  setType(event.target.value as CanvasBoardCardType)
                }
                value={type}
              >
                {CARD_TYPE_OPTIONS.map((option) => (
                  <option key={option.value} value={option.value}>
                    {option.label}
                  </option>
                ))}
              </select>
            </div>

            <div className="space-y-2">
              <label
                className="text-sm font-medium"
                htmlFor={`${titleId}-status`}
              >
                Status
              </label>
              <select
                className={SELECT_CLASS_NAME}
                data-testid="magic-board-card-status"
                disabled={isSaving}
                id={`${titleId}-status`}
                onChange={(event) =>
                  setStatus(event.target.value as CanvasBoardCardStatus)
                }
                value={status}
              >
                {CARD_STATUS_OPTIONS.map((option) => (
                  <option key={option.value} value={option.value}>
                    {option.label}
                  </option>
                ))}
              </select>
            </div>
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
