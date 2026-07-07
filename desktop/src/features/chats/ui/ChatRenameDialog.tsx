import * as React from "react";

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

export function ChatRenameDialog({
  currentTitle,
  isSaving = false,
  onOpenChange,
  onRename,
  open,
}: {
  currentTitle: string;
  isSaving?: boolean;
  onOpenChange: (open: boolean) => void;
  onRename: (title: string) => void;
  open: boolean;
}) {
  const [title, setTitle] = React.useState(currentTitle);

  React.useEffect(() => {
    if (open) {
      setTitle(currentTitle);
    }
  }, [currentTitle, open]);

  const trimmed = title.trim();
  const canSave = trimmed.length > 0 && !isSaving;

  return (
    <Dialog onOpenChange={onOpenChange} open={open}>
      <DialogContent className="sm:max-w-sm">
        <DialogHeader>
          <DialogTitle>Rename chat</DialogTitle>
          <DialogDescription>
            The new name replaces the auto-generated title.
          </DialogDescription>
        </DialogHeader>
        <form
          onSubmit={(event) => {
            event.preventDefault();
            if (canSave) {
              onRename(trimmed);
            }
          }}
        >
          <Input
            aria-label="Chat name"
            autoFocus
            onChange={(event) => setTitle(event.target.value)}
            placeholder="Chat name"
            value={title}
          />
          <DialogFooter className="mt-4">
            <Button
              onClick={() => onOpenChange(false)}
              type="button"
              variant="ghost"
            >
              Cancel
            </Button>
            <Button disabled={!canSave} type="submit">
              Rename
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
