import * as React from "react";
import { toast } from "sonner";

import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";
import { Textarea } from "@/shared/ui/textarea";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
import {
  createBoard,
  KANBAN_TEMPLATES,
  type KanbanTemplateName,
} from "@/features/kanban/lib/boardWrite";

type NewBoardDialogProps = {
  /** Current identity pubkey — the board is created under this owner. */
  owner: string;
  onCreated: (boardId: string) => void | Promise<void>;
  onOpenChange: (open: boolean) => void;
  open: boolean;
};

/**
 * Inline board creation (P3 write path). The board is signed under the
 * Desktop's own identity and published to the relay, so it renders in
 * `/boards` immediately — no CLI, no key handling.
 */
export function NewBoardDialog({
  owner,
  onCreated,
  onOpenChange,
  open,
}: NewBoardDialogProps) {
  const [name, setName] = React.useState("");
  const [description, setDescription] = React.useState("");
  const [template, setTemplate] = React.useState<KanbanTemplateName>("kanban");
  const [creating, setCreating] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);

  async function handleSubmit(event: React.FormEvent) {
    event.preventDefault();
    const trimmed = name.trim();
    if (!trimmed || creating) return;
    setCreating(true);
    setError(null);
    try {
      const { boardId } = await createBoard({
        owner,
        name: trimmed,
        description,
        template,
      });
      toast.success("Board created.");
      setName("");
      setDescription("");
      setTemplate("kanban");
      onOpenChange(false);
      await onCreated(boardId);
    } catch (caught) {
      const message =
        caught instanceof Error
          ? caught.message
          : "Failed to create the board.";
      setError(message);
      toast.error(message);
    } finally {
      setCreating(false);
    }
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-md">
        <DialogHeader>
          <DialogTitle className="text-base">New board</DialogTitle>
          <DialogDescription asChild>
            <span className="sr-only">Create a kanban board</span>
          </DialogDescription>
        </DialogHeader>

        <form className="space-y-4 text-sm" onSubmit={handleSubmit}>
          <label
            className="block space-y-1.5 font-medium"
            htmlFor="new-board-name"
          >
            <span>Name</span>
            <Input
              autoFocus
              data-testid="new-board-name"
              disabled={creating}
              id="new-board-name"
              onChange={(event) => setName(event.target.value)}
              placeholder="My board"
              value={name}
            />
          </label>

          <label
            className="block space-y-1.5 font-medium"
            htmlFor="new-board-description"
          >
            <span>Description (optional)</span>
            <Textarea
              data-testid="new-board-description"
              disabled={creating}
              id="new-board-description"
              onChange={(event) => setDescription(event.target.value)}
              placeholder="What is this board for?"
              value={description}
            />
          </label>

          <label
            className="block space-y-1.5 font-medium"
            htmlFor="new-board-template"
          >
            <span>Template</span>
            <select
              className="h-10 w-full rounded-lg border border-input/40 bg-background px-3 text-sm font-normal outline-hidden focus:ring-1 focus:ring-ring"
              data-testid="new-board-template"
              disabled={creating}
              id="new-board-template"
              onChange={(event) =>
                setTemplate(event.target.value as KanbanTemplateName)
              }
              value={template}
            >
              {Object.keys(KANBAN_TEMPLATES).map((key) => (
                <option key={key} value={key}>
                  {key[0].toUpperCase() + key.slice(1)}
                </option>
              ))}
            </select>
          </label>

          {error ? <p className="text-xs text-destructive">{error}</p> : null}

          <DialogFooter>
            <Button
              disabled={creating}
              onClick={() => onOpenChange(false)}
              type="button"
              variant="ghost"
            >
              Cancel
            </Button>
            <Button
              data-testid="new-board-submit"
              disabled={creating || name.trim().length === 0}
              type="submit"
            >
              {creating ? "Creating…" : "Create board"}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
