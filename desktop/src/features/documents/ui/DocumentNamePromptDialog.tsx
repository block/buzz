import * as React from "react";

import { nameError } from "@/features/documents/useVaultMutations";
import { Button } from "@/shared/ui/button";
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
import { Input } from "@/shared/ui/input";

export type NamePrompt = {
  /** Directory the new entry goes in, or the entry being renamed. */
  contextPath: string;
  initialValue: string;
  kind: "note" | "folder" | "rename";
};

const TITLES: Record<NamePrompt["kind"], string> = {
  folder: "New folder",
  note: "New note",
  rename: "Rename",
};

/**
 * Single dialog for naming a new note, a new folder, or a rename.
 *
 * Validation runs as the user types so the confirm button reflects whether the
 * name is usable, rather than failing after a round trip to the filesystem.
 */
export function DocumentNamePromptDialog({
  onCancel,
  onSubmit,
  prompt,
}: {
  onCancel: () => void;
  onSubmit: (value: string) => void;
  prompt: NamePrompt | null;
}) {
  const [value, setValue] = React.useState("");

  // Reset the field whenever a different prompt opens.
  const [lastPrompt, setLastPrompt] = React.useState<NamePrompt | null>(null);
  if (lastPrompt !== prompt) {
    setLastPrompt(prompt);
    setValue(prompt?.initialValue ?? "");
  }

  const error = prompt ? nameError(value) : null;
  const canSubmit = Boolean(prompt) && error === null;

  const submit = () => {
    if (!canSubmit) return;
    onSubmit(value.trim());
  };

  return (
    <Dialog onOpenChange={(open) => !open && onCancel()} open={Boolean(prompt)}>
      <DialogContent data-testid="documents-name-dialog">
        <DialogHeader>
          <DialogTitle>{prompt ? TITLES[prompt.kind] : ""}</DialogTitle>
        </DialogHeader>

        <Input
          autoFocus
          data-testid="documents-name-input"
          onChange={(event) => setValue(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Enter") {
              event.preventDefault();
              submit();
            }
          }}
          placeholder={prompt?.kind === "folder" ? "Folder name" : "Note name"}
          value={value}
        />
        {/* Only nag once the user has typed something. */}
        {error && value.trim() !== "" ? (
          <p className="text-sm text-destructive">{error}</p>
        ) : null}

        <DialogFooter>
          <Button onClick={onCancel} type="button" variant="ghost">
            Cancel
          </Button>
          <Button
            data-testid="documents-name-submit"
            disabled={!canSubmit}
            onClick={submit}
            type="button"
          >
            {prompt?.kind === "rename" ? "Rename" : "Create"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
