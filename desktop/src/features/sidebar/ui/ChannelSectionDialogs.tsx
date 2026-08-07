import * as React from "react";

import { ChevronDown, Plus, X } from "lucide-react";

import { EmojiPicker } from "@/features/custom-emoji/ui/EmojiPicker";
import { useChannelTemplatesQuery } from "@/features/channel-templates/hooks";
import { TemplateFormDialog } from "@/features/settings/ui/ChannelTemplatesSettingsCard";
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
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";
import { Popover, PopoverContent, PopoverTrigger } from "@/shared/ui/popover";
import type { Channel, ChannelTemplate } from "@/shared/api/types";
import {
  useDeleteChannelMutation,
  useLeaveChannelMutation,
} from "@/features/channels/hooks";
import { ChannelDeleteConfirmationDialog } from "@/features/channels/ui/ChannelManagementModerationActions";

export type SectionDialogValue = {
  name: string;
  icon?: string;
  templateId?: string;
};

const NO_TEMPLATE_VALUE = "__no-template__";

type SectionNameDialogProps = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  title: string;
  description: string;
  initialValue: string;
  initialIcon?: string;
  initialTemplateId?: string;
  templates?: ChannelTemplate[];
  allowTemplateCreation?: boolean;
  confirmLabel: string;
  isConfirmDisabled: (
    trimmed: string,
    icon: string,
    templateId: string,
  ) => boolean;
  onConfirm: (value: SectionDialogValue) => void;
};

function SectionNameDialog({
  open,
  onOpenChange,
  title,
  description,
  initialValue,
  initialIcon = "",
  initialTemplateId = "",
  templates,
  allowTemplateCreation = false,
  confirmLabel,
  isConfirmDisabled,
  onConfirm,
}: SectionNameDialogProps) {
  const [name, setName] = React.useState(initialValue);
  const [icon, setIcon] = React.useState(initialIcon);
  const [pickerOpen, setPickerOpen] = React.useState(false);
  const [isCreateTemplateOpen, setIsCreateTemplateOpen] = React.useState(false);
  const [selectedTemplateId, setSelectedTemplateId] = React.useState("");
  const inputRef = React.useRef<HTMLInputElement>(null);

  React.useEffect(() => {
    if (!open) {
      setPickerOpen(false);
      setIsCreateTemplateOpen(false);
      return;
    }
    setName(initialValue);
    setIcon(initialIcon);
    setSelectedTemplateId(initialTemplateId);
    // Small delay to let dialog animation start before focusing
    const timerId = globalThis.setTimeout(() => {
      inputRef.current?.focus();
    }, 50);
    return () => globalThis.clearTimeout(timerId);
  }, [open, initialValue, initialIcon, initialTemplateId]);

  function handleIconSelect(selectedIcon: string) {
    setIcon(selectedIcon);
    setPickerOpen(false);
  }

  function handleSubmit(event: React.FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const trimmed = name.trim();
    const trimmedIcon = icon.trim();
    if (isConfirmDisabled(trimmed, trimmedIcon, selectedTemplateId)) return;
    onConfirm({
      name: trimmed,
      ...(trimmedIcon ? { icon: trimmedIcon } : {}),
      ...(selectedTemplateId ? { templateId: selectedTemplateId } : {}),
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
          {templates ? (
            <div className="mt-3 flex min-h-11 items-center justify-between gap-4 rounded-xl border border-input bg-background px-3">
              <span className="text-sm font-medium text-foreground">
                Template
                <span className="ml-1 text-xs font-normal text-muted-foreground/50">
                  Optional
                </span>
              </span>
              <DropdownMenu modal={false}>
                <DropdownMenuTrigger asChild>
                  <Button
                    aria-label={`Section template: ${
                      templates.find(
                        (template) => template.id === selectedTemplateId,
                      )?.name ?? "None"
                    }`}
                    className="-mr-2.5 ml-auto h-9 min-w-0 max-w-[60%] justify-end px-2.5 text-right text-sm font-medium"
                    data-testid="create-section-template"
                    type="button"
                    variant="ghost"
                  >
                    <span className="truncate">
                      {templates.find(
                        (template) => template.id === selectedTemplateId,
                      )?.name ?? "None"}
                    </span>
                    <ChevronDown className="size-4 shrink-0 text-muted-foreground/70" />
                  </Button>
                </DropdownMenuTrigger>
                <DropdownMenuContent align="end">
                  <DropdownMenuRadioGroup
                    onValueChange={(value) =>
                      setSelectedTemplateId(
                        value === NO_TEMPLATE_VALUE ? "" : value,
                      )
                    }
                    value={selectedTemplateId || NO_TEMPLATE_VALUE}
                  >
                    <DropdownMenuRadioItem value={NO_TEMPLATE_VALUE}>
                      None
                    </DropdownMenuRadioItem>
                    {templates.map((template) => (
                      <DropdownMenuRadioItem
                        key={template.id}
                        value={template.id}
                      >
                        {template.name}
                      </DropdownMenuRadioItem>
                    ))}
                  </DropdownMenuRadioGroup>
                  {allowTemplateCreation ? (
                    <>
                      <DropdownMenuSeparator />
                      <DropdownMenuItem
                        onSelect={() => setIsCreateTemplateOpen(true)}
                      >
                        <Plus className="size-4" />
                        Create new section template
                      </DropdownMenuItem>
                    </>
                  ) : null}
                </DropdownMenuContent>
              </DropdownMenu>
            </div>
          ) : null}
          <div className="flex justify-end gap-2 mt-4">
            <DialogClose asChild>
              <Button variant="ghost" type="button">
                Cancel
              </Button>
            </DialogClose>
            <Button
              type="submit"
              disabled={isConfirmDisabled(
                name.trim(),
                icon.trim(),
                selectedTemplateId,
              )}
            >
              {confirmLabel}
            </Button>
          </div>
        </form>
      </DialogContent>
      {allowTemplateCreation ? (
        <TemplateFormDialog
          fixedTemplateType="section"
          onCreated={(template) => setSelectedTemplateId(template.id)}
          onOpenChange={setIsCreateTemplateOpen}
          open={isCreateTemplateOpen}
          template={null}
        />
      ) : null}
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
  const templatesQuery = useChannelTemplatesQuery();
  const sectionTemplates = (templatesQuery.data ?? []).filter(
    (template) => template.templateType === "section",
  );

  return (
    <SectionNameDialog
      open={open}
      onOpenChange={onOpenChange}
      title="Create section"
      description="Sections let you group related channels in the sidebar."
      initialValue=""
      templates={sectionTemplates}
      allowTemplateCreation
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
  sectionTemplateId?: string;
  onConfirm: (value: SectionDialogValue) => void;
};

export function RenameSectionDialog({
  open,
  onOpenChange,
  sectionName,
  sectionIcon,
  sectionTemplateId,
  onConfirm,
}: RenameSectionDialogProps) {
  const templatesQuery = useChannelTemplatesQuery();
  const sectionTemplates = (templatesQuery.data ?? []).filter(
    (template) => template.templateType === "section",
  );

  return (
    <SectionNameDialog
      open={open}
      onOpenChange={onOpenChange}
      title="Edit section"
      description="Update the section name and defaults for future channels."
      initialValue={sectionName}
      initialIcon={sectionIcon}
      initialTemplateId={sectionTemplateId}
      templates={sectionTemplates}
      confirmLabel="Save"
      isConfirmDisabled={(trimmed, icon, templateId) =>
        trimmed.length === 0 ||
        (trimmed === sectionName &&
          icon === (sectionIcon ?? "") &&
          templateId === (sectionTemplateId ?? ""))
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
