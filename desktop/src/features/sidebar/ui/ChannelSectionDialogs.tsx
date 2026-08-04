import * as React from "react";

import { X } from "lucide-react";

import { EmojiPicker } from "@/features/custom-emoji/ui/EmojiPicker";
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
import { StatusEmoji } from "@/features/user-status/ui/StatusEmoji";
import { Input } from "@/shared/ui/input";
import { Popover, PopoverContent, PopoverTrigger } from "@/shared/ui/popover";
import type { Channel } from "@/shared/api/types";
import {
  useDeleteChannelMutation,
  useLeaveChannelMutation,
} from "@/features/channels/hooks";
import { ChannelDeleteConfirmationDialog } from "@/features/channels/ui/ChannelManagementModerationActions";

export type SectionDialogValue = {
  name: string;
  icon?: string;
};

type SectionNameDialogProps = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  title: string;
  description: string;
  initialValue: string;
  initialIcon?: string;
  confirmLabel: string;
  isConfirmDisabled: (trimmed: string, icon: string) => boolean;
  getNameError?: (trimmed: string) => string | null;
  onConfirm: (value: SectionDialogValue) => void;
  /**
   * CSS selector for the control that opened this dialog (e.g. sidebar
   * section-actions trigger). When the movable Channels/category lane
   * remounts while the dialog is open, Radix's stored focus node can be
   * detached; re-querying by selector restores focus to the live trigger.
   */
  restoreFocusSelector?: string | null;
};

function focusRestoreTarget(selector: string | null | undefined) {
  if (!selector) return;
  const node = document.querySelector(selector);
  if (node instanceof HTMLElement) {
    node.focus();
  }
}

function SectionNameDialog({
  open,
  onOpenChange,
  title,
  description,
  initialValue,
  initialIcon = "",
  confirmLabel,
  isConfirmDisabled,
  getNameError,
  onConfirm,
  restoreFocusSelector,
}: SectionNameDialogProps) {
  const [name, setName] = React.useState(initialValue);
  const [icon, setIcon] = React.useState(initialIcon);
  const [pickerOpen, setPickerOpen] = React.useState(false);
  const inputRef = React.useRef<HTMLInputElement>(null);
  // Snap on open so parent can clear the prop during close without losing
  // the target for onCloseAutoFocus.
  const restoreFocusSelectorRef = React.useRef<string | null>(null);
  const inputId = React.useId();
  const errorId = `${inputId}-error`;
  const nameError = name.trim() ? getNameError?.(name.trim()) : null;

  React.useEffect(() => {
    if (!open) {
      setPickerOpen(false);
      return;
    }
    restoreFocusSelectorRef.current = restoreFocusSelector ?? null;
    setName(initialValue);
    setIcon(initialIcon);
    // Small delay to let dialog animation start before focusing
    const timerId = globalThis.setTimeout(() => {
      inputRef.current?.focus();
    }, 50);
    return () => globalThis.clearTimeout(timerId);
  }, [open, initialValue, initialIcon, restoreFocusSelector]);

  function handleIconSelect(selectedIcon: string) {
    setIcon(selectedIcon);
    setPickerOpen(false);
  }

  function handleSubmit(event: React.FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const trimmed = name.trim();
    const trimmedIcon = icon.trim();
    if (isConfirmDisabled(trimmed, trimmedIcon)) return;
    onConfirm({
      name: trimmed,
      ...(trimmedIcon ? { icon: trimmedIcon } : {}),
    });
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent
        className="max-w-sm"
        onCloseAutoFocus={(event) => {
          const selector = restoreFocusSelectorRef.current;
          if (!selector) return;
          // Prefer the live trigger over Radix's possibly-detached node
          // after sortable block remounts (Channels wrapper / new category).
          event.preventDefault();
          focusRestoreTarget(selector);
          restoreFocusSelectorRef.current = null;
        }}
      >
        <DialogHeader>
          <DialogTitle>{title}</DialogTitle>
          <DialogDescription>{description}</DialogDescription>
        </DialogHeader>
        <form onSubmit={handleSubmit}>
          <div className="flex items-center gap-2">
            <Popover onOpenChange={setPickerOpen} open={pickerOpen}>
              <div className="relative shrink-0">
                <PopoverTrigger asChild>
                  <button
                    aria-label="Choose category icon"
                    className="flex h-9 w-9 items-center justify-center rounded-md border border-input text-lg transition-colors hover:bg-accent"
                    type="button"
                  >
                    {icon ? (
                      <StatusEmoji className="h-5 w-5" value={icon} />
                    ) : (
                      <span className="text-sm font-medium">#</span>
                    )}
                  </button>
                </PopoverTrigger>
                {icon ? (
                  <button
                    aria-label="Clear category icon"
                    className="absolute -right-1 -top-1 flex h-4 w-4 items-center justify-center rounded-full border border-background bg-muted text-muted-foreground hover:bg-accent hover:text-foreground"
                    onClick={(event) => {
                      event.stopPropagation();
                      setIcon("");
                    }}
                    type="button"
                  >
                    <X className="h-3 w-3" />
                  </button>
                ) : null}
              </div>
              <PopoverContent
                align="start"
                className="w-auto overflow-hidden rounded-2xl p-0"
                sideOffset={4}
              >
                <EmojiPicker onSelect={handleIconSelect} />
              </PopoverContent>
            </Popover>
            <Input
              aria-describedby={nameError ? errorId : undefined}
              aria-invalid={nameError ? true : undefined}
              autoCapitalize="none"
              autoComplete="off"
              autoCorrect="off"
              className="flex-1"
              id={inputId}
              onChange={(event) => setName(event.target.value)}
              placeholder="Category name"
              ref={inputRef}
              spellCheck={false}
              value={name}
            />
          </div>
          <label className="sr-only" htmlFor={inputId}>
            Category name
          </label>
          {nameError ? (
            <p
              className="mt-2 text-sm text-destructive"
              id={errorId}
              role="alert"
            >
              {nameError}
            </p>
          ) : null}
          <div className="flex justify-end gap-2 mt-4">
            <DialogClose asChild>
              <Button variant="ghost" type="button">
                Cancel
              </Button>
            </DialogClose>
            <Button
              type="submit"
              disabled={isConfirmDisabled(name.trim(), icon.trim())}
            >
              {confirmLabel}
            </Button>
          </div>
        </form>
      </DialogContent>
    </Dialog>
  );
}

export type CreateSectionDialogProps = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onConfirm: (value: SectionDialogValue) => void;
  existingNames?: string[];
  restoreFocusSelector?: string | null;
};

export function CreateSectionDialog({
  open,
  onOpenChange,
  onConfirm,
  existingNames = [],
  restoreFocusSelector,
}: CreateSectionDialogProps) {
  const names = new Set(existingNames.map((name) => name.trim().toLowerCase()));
  return (
    <SectionNameDialog
      open={open}
      onOpenChange={onOpenChange}
      title="Create category"
      description="Categories group related channels in your sidebar."
      initialValue=""
      confirmLabel="Create"
      isConfirmDisabled={(trimmed) =>
        trimmed.length === 0 || names.has(trimmed.toLowerCase())
      }
      getNameError={(trimmed) =>
        names.has(trimmed.toLowerCase())
          ? "A category with this name already exists."
          : null
      }
      onConfirm={onConfirm}
      restoreFocusSelector={restoreFocusSelector}
    />
  );
}

export type RenameSectionDialogProps = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  sectionName: string;
  sectionIcon?: string;
  onConfirm: (value: SectionDialogValue) => void;
  existingNames?: string[];
};

export function RenameSectionDialog({
  open,
  onOpenChange,
  sectionName,
  sectionIcon,
  onConfirm,
  existingNames = [],
}: RenameSectionDialogProps) {
  const names = new Set(
    existingNames
      .filter(
        (name) =>
          name.trim().toLowerCase() !== sectionName.trim().toLowerCase(),
      )
      .map((name) => name.trim().toLowerCase()),
  );
  return (
    <SectionNameDialog
      open={open}
      onOpenChange={onOpenChange}
      title="Rename category"
      description="Enter a new name for this category."
      initialValue={sectionName}
      initialIcon={sectionIcon}
      confirmLabel="Save"
      isConfirmDisabled={(trimmed, icon) =>
        trimmed.length === 0 ||
        names.has(trimmed.toLowerCase()) ||
        (trimmed === sectionName && icon === (sectionIcon ?? ""))
      }
      getNameError={(trimmed) =>
        names.has(trimmed.toLowerCase())
          ? "A category with this name already exists."
          : null
      }
      onConfirm={onConfirm}
    />
  );
}

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
      ? `Delete category "${sectionName}"? It has no channels.`
      : `Delete category "${sectionName}"? Its ${channelLabel} will move back to the default Channels group.`;

  return (
    <AlertDialog open={open} onOpenChange={onOpenChange}>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>Delete category</AlertDialogTitle>
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

// ---------------------------------------------------------------------------
// LeaveChannelAlertDialog
// ---------------------------------------------------------------------------

export type LeaveChannelAlertDialogProps = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  channelName: string;
  onConfirm: () => void;
};

export function LeaveChannelAlertDialog({
  open,
  onOpenChange,
  channelName,
  onConfirm,
}: LeaveChannelAlertDialogProps) {
  return (
    <AlertDialog open={open} onOpenChange={onOpenChange}>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>Leave channel</AlertDialogTitle>
          <AlertDialogDescription>
            {`Leave "${channelName}"? You'll stop receiving its messages and can rejoin later.`}
          </AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel>Cancel</AlertDialogCancel>
          <AlertDialogAction
            className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
            onClick={onConfirm}
          >
            Leave
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}

// ---------------------------------------------------------------------------
// useLeaveChannelDialog — owns leave-channel state, mutation, and dialog
// ---------------------------------------------------------------------------

export function useLeaveChannelDialog() {
  const [target, setTarget] = React.useState<Channel | null>(null);
  const leaveChannel = useLeaveChannelMutation(target?.id ?? null);

  const dialog = (
    <LeaveChannelAlertDialog
      open={target !== null}
      onOpenChange={(open) => {
        if (!open) setTarget(null);
      }}
      channelName={target?.name ?? ""}
      onConfirm={() => {
        if (target) {
          leaveChannel.mutate();
        }
        setTarget(null);
      }}
    />
  );

  return { requestLeaveChannel: setTarget, dialog };
}

export function useDeleteChannelDialog(onDeleted: (channel: Channel) => void) {
  const [target, setTarget] = React.useState<Channel | null>(null);
  const deleteChannel = useDeleteChannelMutation(target?.id ?? null);

  const dialog = (
    <ChannelDeleteConfirmationDialog
      channelName={target?.name ?? ""}
      error={deleteChannel.error}
      isPending={deleteChannel.isPending}
      onConfirm={() => {
        const deletedChannel = target;
        deleteChannel.mutate(undefined, {
          onSuccess: () => {
            setTarget(null);
            if (deletedChannel) onDeleted(deletedChannel);
          },
        });
      }}
      onOpenChange={(open) => {
        if (!open) {
          deleteChannel.reset();
          setTarget(null);
        }
      }}
      open={target !== null}
    />
  );

  return { requestDeleteChannel: setTarget, dialog };
}
