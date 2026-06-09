import * as React from "react";

import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/shared/ui/alert-dialog";
import { Button } from "@/shared/ui/button";
import {
  Dialog,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
import { Input } from "@/shared/ui/input";

// ---------------------------------------------------------------------------
// SectionNameDialog (internal)
// ---------------------------------------------------------------------------

type SectionNameDialogProps = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  title: string;
  description: string;
  initialValue: string;
  confirmLabel: string;
  isConfirmDisabled: (trimmed: string) => boolean;
  onConfirm: (name: string) => void;
};

function SectionNameDialog({
  open,
  onOpenChange,
  title,
  description,
  initialValue,
  confirmLabel,
  isConfirmDisabled,
  onConfirm,
}: SectionNameDialogProps) {
  const [name, setName] = React.useState(initialValue);
  const inputRef = React.useRef<HTMLInputElement>(null);

  React.useEffect(() => {
    if (!open) return;
    setName(initialValue);
    // Small delay to let dialog animation start before focusing
    const timerId = globalThis.setTimeout(() => {
      inputRef.current?.focus();
    }, 50);
    return () => globalThis.clearTimeout(timerId);
  }, [open, initialValue]);

  function handleSubmit(event: React.FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const trimmed = name.trim();
    if (isConfirmDisabled(trimmed)) return;
    onConfirm(trimmed);
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-sm">
        <DialogHeader>
          <DialogTitle>{title}</DialogTitle>
          <DialogDescription>{description}</DialogDescription>
        </DialogHeader>
        <form onSubmit={handleSubmit}>
          <Input
            autoCapitalize="none"
            autoComplete="off"
            autoCorrect="off"
            onChange={(event) => setName(event.target.value)}
            placeholder="Section name"
            ref={inputRef}
            spellCheck={false}
            value={name}
          />
          <div className="flex justify-end gap-2 mt-4">
            <DialogClose asChild>
              <Button variant="ghost" type="button">
                Cancel
              </Button>
            </DialogClose>
            <Button type="submit" disabled={isConfirmDisabled(name.trim())}>
              {confirmLabel}
            </Button>
          </div>
        </form>
      </DialogContent>
    </Dialog>
  );
}

// ---------------------------------------------------------------------------
// CreateSectionDialog
// ---------------------------------------------------------------------------

export type CreateSectionDialogProps = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onConfirm: (name: string) => void;
};

export function CreateSectionDialog({
  open,
  onOpenChange,
  onConfirm,
}: CreateSectionDialogProps) {
  return (
    <SectionNameDialog
      open={open}
      onOpenChange={onOpenChange}
      title="Create section"
      description="Sections let you group related channels in the sidebar."
      initialValue=""
      confirmLabel="Create"
      isConfirmDisabled={(trimmed) => trimmed.length === 0}
      onConfirm={onConfirm}
    />
  );
}

// ---------------------------------------------------------------------------
// RenameSectionDialog
// ---------------------------------------------------------------------------

export type RenameSectionDialogProps = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  sectionName: string;
  onConfirm: (newName: string) => void;
};

export function RenameSectionDialog({
  open,
  onOpenChange,
  sectionName,
  onConfirm,
}: RenameSectionDialogProps) {
  return (
    <SectionNameDialog
      open={open}
      onOpenChange={onOpenChange}
      title="Rename section"
      description="Enter a new name for this section."
      initialValue={sectionName}
      confirmLabel="Rename"
      isConfirmDisabled={(trimmed) =>
        trimmed.length === 0 || trimmed === sectionName
      }
      onConfirm={onConfirm}
    />
  );
}

// ---------------------------------------------------------------------------
// DeleteSectionAlertDialog
// ---------------------------------------------------------------------------

export type DeleteSectionAlertDialogProps = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  sectionName: string;
  channelCount: number;
  onConfirm: () => void;
};

export function DeleteSectionAlertDialog({
  open,
  onOpenChange,
  sectionName,
  channelCount,
  onConfirm,
}: DeleteSectionAlertDialogProps) {
  const channelLabel =
    channelCount === 1 ? "1 channel" : `${channelCount} channels`;
  const description =
    channelCount === 0
      ? `Delete section "${sectionName}"? It has no channels.`
      : `Delete section "${sectionName}"? Its ${channelLabel} will move back to the default Channels group.`;

  return (
    <AlertDialog open={open} onOpenChange={onOpenChange}>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>Delete section</AlertDialogTitle>
          <AlertDialogDescription>{description}</AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel>Cancel</AlertDialogCancel>
          <AlertDialogAction
            className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
            onClick={onConfirm}
          >
            Delete
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}
