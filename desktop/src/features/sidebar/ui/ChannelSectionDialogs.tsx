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
  onConfirm: (value: SectionDialogValue) => void;
};

function SectionNameDialog({
  open,
  onOpenChange,
  title,
  description,
  initialValue,
  initialIcon = "",
  confirmLabel,
  isConfirmDisabled,
  onConfirm,
}: SectionNameDialogProps) {
  const [name, setName] = React.useState(initialValue);
  const [icon, setIcon] = React.useState(initialIcon);
  const [pickerOpen, setPickerOpen] = React.useState(false);
  const inputRef = React.useRef<HTMLInputElement>(null);

  React.useEffect(() => {
    if (!open) {
      setPickerOpen(false);
      return;
    }
    setName(initialValue);
    setIcon(initialIcon);
    // Small delay to let dialog animation start before focusing
    const timerId = globalThis.setTimeout(() => {
      inputRef.current?.focus();
    }, 50);
    return () => globalThis.clearTimeout(timerId);
  }, [open, initialValue, initialIcon]);

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
      <DialogContent className="max-w-sm">
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
                    aria-label="Choose section icon"
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
                    aria-label="Clear section icon"
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
              autoCapitalize="none"
              autoComplete="off"
              autoCorrect="off"
              className="flex-1"
              onChange={(event) => setName(event.target.value)}
              placeholder="Section name"
              ref={inputRef}
              spellCheck={false}
              value={name}
            />
          </div>
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

export type RenameSectionDialogProps = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  sectionName: string;
  sectionIcon?: string;
  onConfirm: (value: SectionDialogValue) => void;
};

export function RenameSectionDialog({
  open,
  onOpenChange,
  sectionName,
  sectionIcon,
  onConfirm,
}: RenameSectionDialogProps) {
  return (
    <SectionNameDialog
      open={open}
      onOpenChange={onOpenChange}
      title="Rename section"
      description="Enter a new name for this section."
      initialValue={sectionName}
      initialIcon={sectionIcon}
      confirmLabel="Save"
      isConfirmDisabled={(trimmed, icon) =>
        trimmed.length === 0 ||
        (trimmed === sectionName && icon === (sectionIcon ?? ""))
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
