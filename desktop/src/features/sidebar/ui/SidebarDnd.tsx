// biome-ignore format: keep compact to stay within file size limit
import {
  DndContext,
  DragOverlay,
  PointerSensor,
  pointerWithin,
  useDraggable,
  useDroppable,
  useSensor,
  useSensors,
} from "@dnd-kit/core";
import type { DragEndEvent, DragStartEvent } from "@dnd-kit/core";
import {
  SortableContext,
  arrayMove,
  verticalListSortingStrategy,
  useSortable,
} from "@dnd-kit/sortable";
import { CSS } from "@dnd-kit/utilities";
import { Hash } from "lucide-react";
import * as React from "react";

import { cn } from "@/shared/lib/cn";

// ---------------------------------------------------------------------------
// Data attribute shapes used on draggable / droppable items
// ---------------------------------------------------------------------------

export type DndChannelData = { type: "channel"; channelId: string };
export type DndSectionData = { type: "section"; sectionId: string };
export type DndSectionDropData = { type: "section-drop"; sectionId: string };
export type DndUngroupedData = { type: "ungrouped" };

// ---------------------------------------------------------------------------
// DraggableChannelRow — wraps a channel list item with useDraggable
// ---------------------------------------------------------------------------

export function DraggableChannelRow({
  channelId,
  children,
}: {
  channelId: string;
  children: React.ReactNode;
}) {
  const { attributes, listeners, setNodeRef, isDragging } = useDraggable({
    id: channelId,
    data: { type: "channel", channelId } satisfies DndChannelData,
  });

  return (
    <div
      ref={setNodeRef}
      {...attributes}
      {...listeners}
      className={cn("touch-none", isDragging && "opacity-30")}
    >
      {children}
    </div>
  );
}

// ---------------------------------------------------------------------------
// DroppableSectionBody — makes a section body accept channel drops
// ---------------------------------------------------------------------------

export function DroppableSectionBody({
  sectionId,
  children,
  className,
}: {
  sectionId: string;
  children: React.ReactNode;
  className?: string;
}) {
  const droppableId = `section-drop:${sectionId}`;
  const { setNodeRef, isOver } = useDroppable({
    id: droppableId,
    data: { type: "section-drop", sectionId } satisfies DndSectionDropData,
  });

  return (
    <div
      ref={setNodeRef}
      className={cn(
        "rounded-md transition-all",
        isOver && "ring-2 ring-primary/30",
        className,
      )}
    >
      {children}
    </div>
  );
}

// ---------------------------------------------------------------------------
// DroppableUngroupedBody — makes the ungrouped Channels section a drop target
// ---------------------------------------------------------------------------

export function DroppableUngroupedBody({
  children,
  className,
}: {
  children: React.ReactNode;
  className?: string;
}) {
  const { setNodeRef, isOver } = useDroppable({
    id: "ungrouped",
    data: { type: "ungrouped" } satisfies DndUngroupedData,
  });

  return (
    <div
      ref={setNodeRef}
      className={cn(
        "rounded-md transition-all",
        isOver && "ring-2 ring-primary/30",
        className,
      )}
    >
      {children}
    </div>
  );
}

// ---------------------------------------------------------------------------
// SortableSectionShell — wraps a CustomChannelSection header area
// ---------------------------------------------------------------------------

export function SortableSectionShell({
  sectionId,
  children,
}: {
  sectionId: string;
  children: (props: {
    dragHandleProps: React.HTMLAttributes<HTMLElement>;
    isDragging: boolean;
    style: React.CSSProperties;
  }) => React.ReactNode;
}) {
  const {
    attributes,
    listeners,
    setNodeRef,
    transform,
    transition,
    isDragging,
  } = useSortable({
    id: sectionId,
    data: { type: "section", sectionId } satisfies DndSectionData,
  });

  const style: React.CSSProperties = {
    transform: CSS.Transform.toString(transform),
    transition,
  };

  return (
    <div ref={setNodeRef} style={style}>
      {children({
        dragHandleProps: { ...attributes, ...listeners },
        isDragging,
        style,
      })}
    </div>
  );
}

// ---------------------------------------------------------------------------
// DragOverlayChannel — ghost preview for a channel being dragged
// ---------------------------------------------------------------------------

export function DragOverlayChannel({ name }: { name: string }) {
  return (
    <div className="flex cursor-grabbing items-center gap-2 rounded-md bg-sidebar px-2 py-1.5 text-sm text-sidebar-foreground opacity-90 shadow-lg ring-1 ring-sidebar-border">
      <Hash className="h-4 w-4 shrink-0 text-sidebar-foreground/60" />
      <span className="truncate">{name}</span>
    </div>
  );
}

// ---------------------------------------------------------------------------
// DragOverlaySection — ghost preview for a section header being dragged
// ---------------------------------------------------------------------------

export function DragOverlaySection({ name }: { name: string }) {
  return (
    <div className="flex cursor-grabbing items-center gap-1 rounded-md bg-sidebar px-2 py-1 text-xs font-medium uppercase tracking-wider text-sidebar-foreground/60 opacity-90 shadow-lg ring-1 ring-sidebar-border">
      <span>{name}</span>
    </div>
  );
}

// ---------------------------------------------------------------------------
// SidebarDndContext — DndContext + SortableContext + DragOverlay wrapper
// ---------------------------------------------------------------------------

type SidebarDragItem =
  | { type: "channel"; channelId: string; channelName: string }
  | { type: "section"; sectionId: string; sectionName: string };

export function SidebarDndContext({
  sectionIds,
  channels,
  sections,
  children,
  onAssignChannel,
  onUnassignChannel,
  onReorderSections,
}: {
  sectionIds: string[];
  channels: { id: string; name: string }[];
  sections: { id: string; name: string }[];
  children: React.ReactNode;
  onAssignChannel: (channelId: string, sectionId: string) => void;
  onUnassignChannel: (channelId: string) => void;
  onReorderSections: (orderedIds: string[]) => void;
}) {
  const [activeDragItem, setActiveDragItem] =
    React.useState<SidebarDragItem | null>(null);
  const sensors = useSensors(
    useSensor(PointerSensor, { activationConstraint: { distance: 6 } }),
  );

  const handleDragStart = React.useCallback(
    (event: DragStartEvent) => {
      const data = event.active.data.current;
      if (!data) return;
      if (data.type === "channel") {
        const ch = channels.find((c) => c.id === data.channelId);
        if (ch)
          setActiveDragItem({
            type: "channel",
            channelId: ch.id,
            channelName: ch.name,
          });
      } else if (data.type === "section") {
        const sec = sections.find((s) => s.id === data.sectionId);
        if (sec)
          setActiveDragItem({
            type: "section",
            sectionId: sec.id,
            sectionName: sec.name,
          });
      }
    },
    [channels, sections],
  );

  const handleDragEnd = React.useCallback(
    (event: DragEndEvent) => {
      setActiveDragItem(null);
      const { active, over } = event;
      if (!over) return;
      const activeData = active.data.current;
      const overData = over.data.current;
      if (!activeData) return;
      if (activeData.type === "channel") {
        const channelId = activeData.channelId as string;
        if (overData?.type === "section-drop") {
          onAssignChannel(channelId, overData.sectionId as string);
        } else if (overData?.type === "ungrouped") {
          onUnassignChannel(channelId);
        }
      } else if (activeData.type === "section") {
        const overSectionId =
          (overData?.sectionId as string | undefined) ?? (over.id as string);
        const oldIdx = sectionIds.indexOf(active.id as string);
        const newIdx = sectionIds.indexOf(overSectionId);
        if (oldIdx !== -1 && newIdx !== -1 && oldIdx !== newIdx) {
          onReorderSections(arrayMove(sectionIds, oldIdx, newIdx));
        }
      }
    },
    [sectionIds, onAssignChannel, onUnassignChannel, onReorderSections],
  );

  return (
    <DndContext
      collisionDetection={pointerWithin}
      onDragEnd={handleDragEnd}
      onDragStart={handleDragStart}
      sensors={sensors}
    >
      <SortableContext
        items={sectionIds}
        strategy={verticalListSortingStrategy}
      >
        {children}
      </SortableContext>
      <DragOverlay>
        {activeDragItem?.type === "channel" ? (
          <DragOverlayChannel name={activeDragItem.channelName} />
        ) : activeDragItem?.type === "section" ? (
          <DragOverlaySection name={activeDragItem.sectionName} />
        ) : null}
      </DragOverlay>
    </DndContext>
  );
}
