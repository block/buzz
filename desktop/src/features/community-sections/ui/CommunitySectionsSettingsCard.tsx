import {
  DndContext,
  PointerSensor,
  closestCenter,
  useSensor,
  useSensors,
  type DragEndEvent,
} from "@dnd-kit/core";
import {
  SortableContext,
  arrayMove,
  useSortable,
  verticalListSortingStrategy,
} from "@dnd-kit/sortable";
import { CSS } from "@dnd-kit/utilities";
import {
  GripVertical,
  LayoutList,
  MoreHorizontal,
  Pencil,
  Plus,
  Trash2,
} from "lucide-react";
import * as React from "react";
import { toast } from "sonner";

import { useCommunitySections } from "@/features/community-sections/hooks";
import { channelIdsClaimedByOtherSections } from "@/features/community-sections/lib/sectionAdmin";
import type {
  CommunitySection,
  CommunitySectionDraft,
} from "@/features/community-sections/lib/types";
import { MAX_SECTION_NAME_LEN } from "@/features/community-sections/lib/types";
import { SettingsOptionGroup } from "@/features/settings/ui/SettingsOptionGroup";
import { SettingsSectionHeader } from "@/features/settings/ui/SettingsSectionHeader";
import type { Channel } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import { ChooserDialogContent } from "@/shared/ui/chooser-dialog-content";
import { Dialog } from "@/shared/ui/dialog";
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
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";
import { Input } from "@/shared/ui/input";
import { Switch } from "@/shared/ui/switch";

export function CommunitySectionsSettingsCard() {
  const {
    sections,
    streamChannels,
    canManage,
    isLoading,
    isSaving,
    createSection,
    updateSection,
    reorderSections,
    deleteSection,
    setSubscribed,
    isSubscribed,
    communitySectionsEnabled,
    setCommunitySectionsEnabled,
  } = useCommunitySections();
  const [createOpen, setCreateOpen] = React.useState(false);
  const [editing, setEditing] = React.useState<CommunitySection | null>(null);
  const [deleteTarget, setDeleteTarget] =
    React.useState<CommunitySection | null>(null);

  const sectionIds = React.useMemo(
    () => sections.map((section) => section.id),
    [sections],
  );
  const sensors = useSensors(
    useSensor(PointerSensor, { activationConstraint: { distance: 6 } }),
  );

  const handleDragEnd = React.useCallback(
    (event: DragEndEvent) => {
      if (!canManage || isSaving) return;
      const { active, over } = event;
      if (!over || active.id === over.id) return;
      const oldIdx = sectionIds.indexOf(String(active.id));
      const newIdx = sectionIds.indexOf(String(over.id));
      if (oldIdx === -1 || newIdx === -1) return;
      void reorderSections(arrayMove(sectionIds, oldIdx, newIdx)).catch(
        (error) => {
          toast.error(
            error instanceof Error
              ? error.message
              : "Failed to reorder sections",
          );
        },
      );
    },
    [canManage, isSaving, reorderSections, sectionIds],
  );

  const catalogRows = sections.map((section) => (
    <SectionRow
      canManage={canManage}
      channels={streamChannels}
      key={section.id}
      onDelete={() => setDeleteTarget(section)}
      onEdit={() => setEditing(section)}
      onSubscribeChange={(subscribed) => setSubscribed(section.id, subscribed)}
      section={section}
      subscribeDisabled={!communitySectionsEnabled}
      subscribed={isSubscribed(section.id)}
    />
  ));

  return (
    <section className="min-w-0" data-testid="settings-community-sections">
      <SettingsSectionHeader
        action={
          canManage ? (
            <Button
              data-testid="community-sections-add"
              disabled={isLoading || isSaving}
              onClick={() => setCreateOpen(true)}
              size="sm"
              type="button"
              variant="outline"
            >
              <Plus className="mr-1.5 h-4 w-4" />
              Add section
            </Button>
          ) : null
        }
        description="Admins define shared channel sections for the community. Subscribe to show a section on your left nav — unsubscribed sections stay out of your sidebar."
        title="Community sections"
      />

      <SettingsOptionGroup title="Preference">
        <div
          className="flex items-center gap-3 px-4 py-3"
          data-testid="community-sections-master-toggle-row"
        >
          <div className="min-w-0 flex-1">
            <div className="text-sm font-medium">Use Community Sections</div>
            <p
              className="text-sm font-normal text-muted-foreground/70"
              data-settings-subcopy
            >
              When on, subscribed community sections appear on your left nav and
              their channels stay locked in those sections. When off, those
              sections hide from the nav and channels can move into personal
              sections again — personal sections are never deleted.
            </p>
          </div>
          <Switch
            aria-label="Use Community Sections"
            checked={communitySectionsEnabled}
            data-testid="community-sections-master-toggle"
            onCheckedChange={setCommunitySectionsEnabled}
          />
        </div>
      </SettingsOptionGroup>

      {isLoading ? (
        <SettingsOptionGroup title="Catalog">
          <p className="px-4 py-6 text-center text-sm text-muted-foreground">
            Loading community sections...
          </p>
        </SettingsOptionGroup>
      ) : sections.length === 0 ? (
        <SettingsOptionGroup title="Catalog">
          <div className="px-4 py-8 text-center text-sm text-muted-foreground">
            {canManage
              ? "No community sections yet. Add one and pick the channels everyone will see when they subscribe."
              : "No community sections yet. Ask an admin to create one."}
          </div>
        </SettingsOptionGroup>
      ) : (
        <SettingsOptionGroup title="Catalog">
          {canManage ? (
            <DndContext
              collisionDetection={closestCenter}
              onDragEnd={handleDragEnd}
              sensors={sensors}
            >
              <SortableContext
                items={sectionIds}
                strategy={verticalListSortingStrategy}
              >
                {catalogRows}
              </SortableContext>
            </DndContext>
          ) : (
            catalogRows
          )}
        </SettingsOptionGroup>
      )}

      {createOpen ? (
        <SectionFormDialog
          channels={streamChannels}
          claimedChannelIds={channelIdsClaimedByOtherSections(sections, null)}
          onOpenChange={setCreateOpen}
          onSave={async (draft) => {
            await createSection(draft);
            toast.success("Community section added");
          }}
          section={null}
        />
      ) : null}

      {editing ? (
        <SectionFormDialog
          channels={streamChannels}
          claimedChannelIds={channelIdsClaimedByOtherSections(
            sections,
            editing.id,
          )}
          onOpenChange={(open) => {
            if (!open) setEditing(null);
          }}
          onSave={async (draft) => {
            await updateSection(editing.id, draft);
            toast.success("Community section updated");
            setEditing(null);
          }}
          section={editing}
        />
      ) : null}

      {deleteTarget ? (
        <AlertDialog
          onOpenChange={(open) => {
            if (!open) setDeleteTarget(null);
          }}
          open
        >
          <AlertDialogContent>
            <AlertDialogHeader>
              <AlertDialogTitle>Delete community section?</AlertDialogTitle>
              <AlertDialogDescription>
                Remove &quot;{deleteTarget.name}&quot; from the catalog for
                everyone? Member subscriptions for this section are cleared on
                this device.
              </AlertDialogDescription>
            </AlertDialogHeader>
            <AlertDialogFooter>
              <AlertDialogCancel type="button">Cancel</AlertDialogCancel>
              <AlertDialogAction
                className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
                data-testid="community-sections-confirm-delete"
                onClick={() => {
                  void deleteSection(deleteTarget.id)
                    .then(() => {
                      toast.success("Community section deleted");
                      setDeleteTarget(null);
                    })
                    .catch((error) => {
                      toast.error(
                        error instanceof Error
                          ? error.message
                          : "Failed to delete section",
                      );
                    });
                }}
                type="button"
              >
                Delete
              </AlertDialogAction>
            </AlertDialogFooter>
          </AlertDialogContent>
        </AlertDialog>
      ) : null}
    </section>
  );
}

function SectionRow(props: {
  canManage: boolean;
  channels: Channel[];
  onDelete: () => void;
  onEdit: () => void;
  onSubscribeChange: (subscribed: boolean) => void;
  section: CommunitySection;
  subscribeDisabled?: boolean;
  subscribed: boolean;
}) {
  if (props.canManage) {
    return <SortableSectionRow {...props} />;
  }
  return <SectionRowContent {...props} />;
}

function SortableSectionRow(props: {
  canManage: boolean;
  channels: Channel[];
  onDelete: () => void;
  onEdit: () => void;
  onSubscribeChange: (subscribed: boolean) => void;
  section: CommunitySection;
  subscribeDisabled?: boolean;
  subscribed: boolean;
}) {
  const {
    attributes,
    listeners,
    setNodeRef,
    transform,
    transition,
    isDragging,
  } = useSortable({ id: props.section.id });

  return (
    <SectionRowContent
      {...props}
      dragHandleProps={{ ...attributes, ...listeners }}
      isDragging={isDragging}
      setNodeRef={setNodeRef}
      style={{
        transform: CSS.Transform.toString(transform),
        transition,
      }}
    />
  );
}

function SectionRowContent({
  canManage,
  channels,
  dragHandleProps,
  isDragging = false,
  onDelete,
  onEdit,
  onSubscribeChange,
  section,
  setNodeRef,
  style,
  subscribeDisabled = false,
  subscribed,
}: {
  canManage: boolean;
  channels: Channel[];
  dragHandleProps?: React.HTMLAttributes<HTMLElement>;
  isDragging?: boolean;
  onDelete: () => void;
  onEdit: () => void;
  onSubscribeChange: (subscribed: boolean) => void;
  section: CommunitySection;
  setNodeRef?: (node: HTMLElement | null) => void;
  style?: React.CSSProperties;
  subscribeDisabled?: boolean;
  subscribed: boolean;
}) {
  const channelNames = section.channelIds
    .map((id) => {
      const channel = channels.find((entry) => entry.id === id);
      return channel ? channel.name : null;
    })
    .filter((name): name is string => Boolean(name));

  return (
    <div
      className={cn(
        "group flex items-center gap-3 px-4 py-3",
        isDragging && "opacity-40",
      )}
      data-testid={`community-section-row-${section.id}`}
      ref={setNodeRef}
      style={style}
    >
      {canManage && dragHandleProps ? (
        <button
          aria-label={`Drag to reorder ${section.name}`}
          className="touch-none text-muted-foreground hover:text-foreground"
          data-testid={`community-section-drag-${section.id}`}
          type="button"
          {...dragHandleProps}
        >
          <GripVertical className="h-4 w-4" />
        </button>
      ) : (
        <LayoutList className="h-4 w-4 shrink-0 text-muted-foreground" />
      )}
      <div className="min-w-0 flex-1">
        <div className="truncate text-sm font-medium">{section.name}</div>
        <p
          className="truncate text-sm font-normal text-muted-foreground/70"
          data-settings-subcopy
        >
          {channelNames.length > 0
            ? `${channelNames.slice(0, 4).join(", ")}${channelNames.length > 4 ? ` +${channelNames.length - 4}` : ""}`
            : "No channels listed yet"}
        </p>
      </div>
      <div className="flex shrink-0 items-center gap-2">
        <Switch
          aria-label={`Subscribe to ${section.name}`}
          checked={subscribed}
          data-testid={`community-section-subscribe-${section.id}`}
          disabled={subscribeDisabled}
          onCheckedChange={onSubscribeChange}
        />
        {canManage ? (
          <DropdownMenu modal={false}>
            <DropdownMenuTrigger asChild>
              <Button
                aria-label={`Actions for ${section.name}`}
                className="h-7 w-7 shrink-0"
                data-testid={`community-section-actions-${section.id}`}
                size="icon"
                type="button"
                variant="ghost"
              >
                <MoreHorizontal className="h-4 w-4" />
              </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end">
              <DropdownMenuItem onClick={onEdit}>
                <Pencil className="mr-2 h-4 w-4" />
                Edit
              </DropdownMenuItem>
              <DropdownMenuItem onClick={onDelete}>
                <Trash2 className="mr-2 h-4 w-4" />
                Delete
              </DropdownMenuItem>
            </DropdownMenuContent>
          </DropdownMenu>
        ) : null}
      </div>
    </div>
  );
}

function SectionFormDialog({
  channels,
  claimedChannelIds,
  onOpenChange,
  onSave,
  section,
}: {
  channels: Channel[];
  claimedChannelIds: ReadonlySet<string>;
  onOpenChange: (open: boolean) => void;
  onSave: (draft: CommunitySectionDraft) => Promise<void>;
  section: CommunitySection | null;
}) {
  const [name, setName] = React.useState(section?.name ?? "");
  const [selectedIds, setSelectedIds] = React.useState<string[]>(
    section?.channelIds ?? [],
  );
  const [saving, setSaving] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  const formId = "community-section-form";

  const toggleChannel = (channelId: string) => {
    if (claimedChannelIds.has(channelId)) return;
    setSelectedIds((current) =>
      current.includes(channelId)
        ? current.filter((id) => id !== channelId)
        : [...current, channelId],
    );
  };

  async function handleSubmit(event: React.FormEvent) {
    event.preventDefault();
    setSaving(true);
    setError(null);
    try {
      const channelIds = selectedIds.filter((id) => !claimedChannelIds.has(id));
      await onSave({ name, channelIds });
      onOpenChange(false);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : "Failed to save");
    } finally {
      setSaving(false);
    }
  }

  return (
    <Dialog onOpenChange={onOpenChange} open>
      <ChooserDialogContent
        data-testid="community-section-form-dialog"
        footer={
          <div className="flex justify-end gap-2">
            <Button
              onClick={() => onOpenChange(false)}
              type="button"
              variant="ghost"
            >
              Cancel
            </Button>
            <Button
              data-testid="community-section-save"
              disabled={saving}
              form={formId}
              type="submit"
            >
              {section ? "Save" : "Add"}
            </Button>
          </div>
        }
        headerSubtitle="Pick the channels that belong in this shared section. Everyone who subscribes sees the same list. A channel can belong to only one community section."
        title={section ? "Edit community section" : "Add community section"}
      >
        <form
          className="space-y-4"
          id={formId}
          onSubmit={(event) => void handleSubmit(event)}
        >
          <div className="space-y-1.5">
            <label
              className="text-sm font-medium"
              htmlFor="community-section-name"
            >
              Name
            </label>
            <Input
              data-testid="community-section-name"
              id="community-section-name"
              maxLength={MAX_SECTION_NAME_LEN}
              onChange={(event) => setName(event.target.value)}
              placeholder="Engineering"
              value={name}
            />
          </div>

          <div className="space-y-2">
            <div className="text-sm font-medium">Channels</div>
            <div className="max-h-64 space-y-1 overflow-y-auto rounded-md border p-2">
              {channels.length === 0 ? (
                <p className="px-2 py-4 text-center text-sm text-muted-foreground">
                  No stream channels available.
                </p>
              ) : (
                channels.map((channel) => {
                  const checked = selectedIds.includes(channel.id);
                  const claimed = claimedChannelIds.has(channel.id);
                  const disabled = claimed && !checked;
                  return (
                    <button
                      aria-disabled={disabled}
                      className={cn(
                        "flex w-full items-center justify-between rounded-md px-2 py-1.5 text-left text-sm hover:bg-muted/60",
                        checked && "bg-muted",
                        disabled &&
                          "cursor-not-allowed text-muted-foreground/50 hover:bg-transparent",
                      )}
                      data-testid={`community-section-channel-${channel.id}`}
                      disabled={disabled}
                      key={channel.id}
                      onClick={() => toggleChannel(channel.id)}
                      type="button"
                    >
                      <span className="truncate">{channel.name}</span>
                      <span className="text-xs text-muted-foreground">
                        {checked
                          ? "Selected"
                          : claimed
                            ? "In another section"
                            : "Add"}
                      </span>
                    </button>
                  );
                })
              )}
            </div>
          </div>

          {error ? (
            <p className="text-sm text-destructive" role="alert">
              {error}
            </p>
          ) : null}
        </form>
      </ChooserDialogContent>
    </Dialog>
  );
}
