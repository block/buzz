// biome-ignore format: keep compact to stay within file size limit
import {
  DndContext,
  DragOverlay,
  KeyboardSensor,
  PointerSensor,
  closestCenter,
  pointerWithin,
  useDroppable,
  useSensor,
  useSensors,
} from "@dnd-kit/core";
import type { DragEndEvent, DragStartEvent } from "@dnd-kit/core";
import type { CollisionDetection } from "@dnd-kit/core";
import {
  SortableContext,
  arrayMove,
  sortableKeyboardCoordinates,
  verticalListSortingStrategy,
  useSortable,
} from "@dnd-kit/sortable";
import { CSS } from "@dnd-kit/utilities";
import { GripVertical, Hash } from "lucide-react";
import * as React from "react";

import { cn } from "@/shared/lib/cn";
import type { ChannelSortGroupKey } from "@/features/sidebar/lib/channelSortPreference";
/** Sentinel id for the Channels block in SortableContext / block order. */
export { CHANNELS_BLOCK_ID } from "@/features/sidebar/lib/channelSectionsHelpers";

export type DndChannelData = {
  type: "channel";
  channelId: string;
  groupKey: ChannelSortGroupKey;
};
export type DndSectionData = { type: "section"; sectionId: string };
/** Built-in uncategorized Channels block in the unified movable lane. */
export type DndChannelsBlockData = { type: "channels-block" };
export type DndSectionDropData = {
  type: "section-drop";
  sectionId: string;
  groupKey: ChannelSortGroupKey;
};
export type DndUngroupedData = {
  type: "ungrouped";
  groupKey: ChannelSortGroupKey;
};

export function DraggableChannelRow({
  channelId,
  channelName,
  groupKey,
  children,
}: {
  channelId: string;
  channelName: string;
  groupKey: ChannelSortGroupKey;
  children: React.ReactNode;
}) {
  const {
    attributes,
    listeners,
    setNodeRef,
    isDragging,
    transform,
    transition,
  } = useSortable({
    id: channelId,
    data: { type: "channel", channelId, groupKey } satisfies DndChannelData,
  });

  return (
    <div
      ref={setNodeRef}
      className={cn(
        "group/channel-drag-row relative [&_[data-sidebar=menu-button]]:pr-8",
        isDragging && "opacity-30",
      )}
      data-dnd-channel={channelId}
      style={{
        transform: CSS.Transform.toString(transform),
        transition,
      }}
    >
      {children}
      <button
        {...attributes}
        {...listeners}
        aria-label={`Move ${channelName}`}
        aria-roledescription="sortable channel"
        className={cn(
          "absolute right-1 top-1/2 z-10 flex h-7 w-7 -translate-y-1/2 touch-none cursor-grab items-center justify-center rounded-md text-sidebar-foreground/40 opacity-0 transition-opacity hover:bg-sidebar-accent hover:text-sidebar-foreground focus-visible:opacity-100 focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-sidebar-ring group-focus-within/channel-drag-row:opacity-100 group-hover/channel-drag-row:opacity-100 active:cursor-grabbing [@media(hover:none)]:opacity-100",
          isDragging && "opacity-100",
        )}
        data-dnd-handle={channelId}
        type="button"
      >
        <GripVertical aria-hidden="true" className="h-4 w-4" />
      </button>
    </div>
  );
}

export function SortableChannelContext({
  channelIds,
  children,
}: {
  channelIds: string[];
  children: React.ReactNode;
}) {
  return (
    <SortableContext items={channelIds} strategy={verticalListSortingStrategy}>
      {children}
    </SortableContext>
  );
}

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
  const groupKey = `section:${sectionId}` as const;
  const { setNodeRef, isOver } = useDroppable({
    id: droppableId,
    data: {
      type: "section-drop",
      sectionId,
      groupKey,
    } satisfies DndSectionDropData,
  });

  return (
    <div
      ref={setNodeRef}
      className={cn(
        "rounded-md transition-all",
        isOver && "ring-2 ring-primary/30",
        className,
      )}
      data-testid={`section-drop-${sectionId}`}
    >
      {children}
    </div>
  );
}

export function DroppableUngroupedBody({
  children,
  className,
}: {
  children: React.ReactNode;
  className?: string;
}) {
  const { setNodeRef, isOver } = useDroppable({
    id: "ungrouped",
    data: {
      type: "ungrouped",
      groupKey: "channels",
    } satisfies DndUngroupedData,
  });

  return (
    <div
      ref={setNodeRef}
      className={cn(
        "rounded-md transition-all",
        isOver && "ring-2 ring-primary/30",
        className,
      )}
      data-testid="section-drop-channels"
    >
      {children}
    </div>
  );
}

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
    <div ref={setNodeRef} style={style} data-dnd-block={sectionId}>
      {children({
        dragHandleProps: { ...attributes, ...listeners },
        isDragging,
        style,
      })}
    </div>
  );
}

export function SortableChannelsBlockShell({
  blockId,
  children,
}: {
  blockId: string;
  children: (props: {
    dragHandleProps: React.HTMLAttributes<HTMLElement>;
    isDragging: boolean;
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
    id: blockId,
    data: { type: "channels-block" } satisfies DndChannelsBlockData,
  });

  return (
    <div
      ref={setNodeRef}
      data-dnd-block={blockId}
      style={{
        transform: CSS.Transform.toString(transform),
        transition,
      }}
    >
      {children({
        dragHandleProps: { ...attributes, ...listeners },
        isDragging,
      })}
    </div>
  );
}

/** Quiet category/Channels block grip — mounted always; visible on hover/focus. */
export function BlockDragHandle({
  label,
  dragHandleProps,
  isDragging,
  testId,
}: {
  label: string;
  dragHandleProps: React.HTMLAttributes<HTMLElement>;
  isDragging: boolean;
  testId?: string;
}) {
  return (
    <button
      {...dragHandleProps}
      aria-label={`Move ${label}`}
      aria-roledescription="sortable category"
      className={cn(
        "flex h-7 w-7 touch-none cursor-grab items-center justify-center rounded-md text-sidebar-foreground/40 opacity-0 transition-opacity hover:bg-sidebar-accent hover:text-sidebar-foreground focus-visible:opacity-100 focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-sidebar-ring group-hover/sidebar-section:opacity-100 group-focus-within/sidebar-section:opacity-100 active:cursor-grabbing [@media(hover:none)]:opacity-100",
        isDragging && "opacity-100",
      )}
      data-testid={testId}
      type="button"
    >
      <GripVertical aria-hidden="true" className="h-4 w-4" />
    </button>
  );
}

export function DragOverlayChannel({ name }: { name: string }) {
  return (
    <div
      data-buzz-flat
      className="flex cursor-grabbing items-center gap-2 rounded-md bg-sidebar px-2 py-1.5 text-sm text-sidebar-foreground opacity-90 shadow-lg ring-1 ring-sidebar-border"
    >
      <Hash className="h-4 w-4 shrink-0 text-sidebar-foreground/60" />
      <span className="truncate">{name}</span>
    </div>
  );
}

export function DragOverlaySection({ name }: { name: string }) {
  return (
    <div
      data-buzz-flat
      className="flex cursor-grabbing items-center gap-1 rounded-md bg-sidebar px-2 py-1 text-xs font-medium uppercase tracking-wider text-sidebar-foreground/60 opacity-90 shadow-lg ring-1 ring-sidebar-border"
    >
      <span>{name}</span>
    </div>
  );
}

type SidebarDragItem =
  | { type: "channel"; channelId: string; channelName: string }
  | { type: "section"; sectionId: string; sectionName: string }
  | { type: "channels-block" };

const BLOCK_TYPES = new Set(["section", "channels-block"]);

const sidebarCollisionDetection: CollisionDetection = (args) => {
  if (args.active.data.current?.type !== "channel") {
    // Category / Channels block reordering: collide with other movable blocks
    // (not channel drop targets) so the pointer can cross the Channels lane.
    const blockContainers = args.droppableContainers.filter((container) => {
      const type = container.data.current?.type;
      return (
        container.id !== args.active.id &&
        type !== undefined &&
        BLOCK_TYPES.has(type as string)
      );
    });
    if (args.pointerCoordinates) {
      return pointerWithin({
        ...args,
        droppableContainers: blockContainers,
      });
    }
    return closestCenter({
      ...args,
      droppableContainers: blockContainers,
    });
  }
  const collisions = args.pointerCoordinates
    ? pointerWithin(args)
    : closestCenter(args);

  const channelCollision = collisions.find((collision) => {
    if (collision.id === args.active.id) return false;
    return (
      args.droppableContainers.find(
        (container) => container.id === collision.id,
      )?.data.current?.type === "channel"
    );
  });
  if (channelCollision) return [channelCollision];

  const groupCollision = collisions.find((collision) => {
    const type = args.droppableContainers.find(
      (container) => container.id === collision.id,
    )?.data.current?.type;
    return type === "section-drop" || type === "ungrouped";
  });
  return groupCollision ? [groupCollision] : collisions;
};

export function SidebarDndContext({
  blockIds,
  channels,
  channelGroups,
  sections,
  children,
  manualGroupKeys,
  onMoveChannel,
  onReorderBlocks,
}: {
  /** Unified movable lane: section ids + Channels sentinel. */
  blockIds: string[];
  channels: { id: string; name: string }[];
  channelGroups: {
    key: ChannelSortGroupKey;
    name: string;
    channelIds: string[];
  }[];
  sections: { id: string; name: string }[];
  children: React.ReactNode;
  manualGroupKeys: ReadonlySet<ChannelSortGroupKey>;
  onMoveChannel: (input: {
    channelId: string;
    sourceGroup: ChannelSortGroupKey;
    targetGroup: ChannelSortGroupKey;
    overChannelId?: string;
  }) => void;
  onReorderBlocks: (orderedBlockIds: string[]) => void;
}) {
  const [activeDragItem, setActiveDragItem] =
    React.useState<SidebarDragItem | null>(null);
  const sensors = useSensors(
    useSensor(PointerSensor, { activationConstraint: { distance: 6 } }),
    useSensor(KeyboardSensor, {
      coordinateGetter: sortableKeyboardCoordinates,
    }),
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
      } else if (data.type === "channels-block") {
        setActiveDragItem({ type: "channels-block" });
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
        const sourceGroup = activeData.groupKey as ChannelSortGroupKey;
        const targetGroup = overData?.groupKey as
          | ChannelSortGroupKey
          | undefined;
        if (!targetGroup) return;
        if (sourceGroup === targetGroup && !manualGroupKeys.has(sourceGroup))
          return;
        onMoveChannel({
          channelId,
          sourceGroup,
          targetGroup,
          ...(overData?.type === "channel"
            ? { overChannelId: overData.channelId as string }
            : {}),
        });
      } else if (
        activeData.type === "section" ||
        activeData.type === "channels-block"
      ) {
        const overBlockId = (() => {
          if (overData?.type === "section") return overData.sectionId as string;
          if (overData?.type === "channels-block") return over.id as string;
          // Fall back to sortable id (works for both block types).
          if (blockIds.includes(over.id as string)) return over.id as string;
          return undefined;
        })();
        if (!overBlockId) return;
        const oldIdx = blockIds.indexOf(active.id as string);
        const newIdx = blockIds.indexOf(overBlockId);
        if (oldIdx !== -1 && newIdx !== -1 && oldIdx !== newIdx) {
          onReorderBlocks(arrayMove(blockIds, oldIdx, newIdx));
        }
      }
    },
    [blockIds, manualGroupKeys, onMoveChannel, onReorderBlocks],
  );

  const channelName = React.useCallback(
    (id: string) =>
      channels.find((channel) => channel.id === id)?.name ?? "channel",
    [channels],
  );
  const sectionName = React.useCallback(
    (id: string) =>
      sections.find((section) => section.id === id)?.name ?? "category",
    [sections],
  );
  const describeChannelDestination = React.useCallback(
    (
      activeChannelId: string,
      over: {
        id: string | number;
        data: { current?: Record<string, unknown> };
      },
    ) => {
      const data = over.data.current;
      const targetGroup = data?.groupKey as ChannelSortGroupKey | undefined;
      const group = channelGroups.find((entry) => entry.key === targetGroup);
      if (!group) return "the sidebar";
      const overChannelId =
        data?.type === "channel" ? String(data.channelId) : undefined;
      let index = group.channelIds.filter(
        (id) => id !== activeChannelId,
      ).length;
      if (overChannelId) {
        const overIndex = group.channelIds.indexOf(overChannelId);
        if (overIndex !== -1) index = overIndex;
      }
      return `position ${index + 1} in category ${group.name}`;
    },
    [channelGroups],
  );
  const describeTarget = React.useCallback(
    (over: {
      id: string | number;
      data: { current?: Record<string, unknown> };
    }) => {
      const data = over.data.current;
      if (data?.type === "channel") {
        return `channel ${channelName(String(data.channelId))}`;
      }
      if (data?.type === "section-drop") {
        return `category ${sectionName(String(data.sectionId))}`;
      }
      if (data?.type === "ungrouped") return "Channels";
      if (data?.type === "channels-block") return "Channels";
      if (data?.type === "section") {
        return `category ${sectionName(String(data.sectionId))}`;
      }
      return "the sidebar";
    },
    [channelName, sectionName],
  );
  const blockLabel = React.useCallback(
    (data: Record<string, unknown> | undefined) => {
      if (data?.type === "channels-block") return "Channels";
      return `category ${sectionName(String(data?.sectionId))}`;
    },
    [sectionName],
  );

  return (
    <DndContext
      accessibility={{
        announcements: {
          onDragStart({ active }) {
            const data = active.data.current;
            if (data?.type === "channel") {
              return `Picked up channel ${channelName(String(data.channelId))}.`;
            }
            return `Picked up ${blockLabel(data)}.`;
          },
          onDragOver({ active, over }) {
            const activeData = active.data.current;
            if (!over) return "The item is no longer over a drop target.";
            if (activeData?.type === "channel") {
              return `Channel ${channelName(String(activeData.channelId))} is over ${describeChannelDestination(String(activeData.channelId), over)}.`;
            }
            return `${blockLabel(activeData)} is over ${describeTarget(over)}.`;
          },
          onDragEnd({ active, over }) {
            const data = active.data.current;
            if (!over) return "Drag cancelled.";
            if (data?.type === "channel") {
              const group = data.groupKey as ChannelSortGroupKey;
              const targetGroup = over.data.current?.groupKey as
                | ChannelSortGroupKey
                | undefined;
              const name = channelName(String(data.channelId));
              if (group === targetGroup && !manualGroupKeys.has(group)) {
                return `${name} was not moved. Choose Manual sort to reorder within this category.`;
              }
              return `Moved channel ${name} to ${describeChannelDestination(String(data.channelId), over)}.`;
            }
            return `Moved ${blockLabel(data)} to ${describeTarget(over)}.`;
          },
          onDragCancel({ active }) {
            const data = active.data.current;
            if (data?.type === "channel") {
              return `Cancelled moving channel ${channelName(String(data.channelId))}.`;
            }
            return `Cancelled moving ${blockLabel(data)}.`;
          },
        },
      }}
      onDragCancel={() => setActiveDragItem(null)}
      collisionDetection={sidebarCollisionDetection}
      onDragEnd={handleDragEnd}
      onDragStart={handleDragStart}
      sensors={sensors}
    >
      <SortableContext items={blockIds} strategy={verticalListSortingStrategy}>
        {children}
      </SortableContext>
      <DragOverlay>
        {activeDragItem?.type === "channel" ? (
          <DragOverlayChannel name={activeDragItem.channelName} />
        ) : activeDragItem?.type === "section" ? (
          <DragOverlaySection name={activeDragItem.sectionName} />
        ) : activeDragItem?.type === "channels-block" ? (
          <DragOverlaySection name="Channels" />
        ) : null}
      </DragOverlay>
    </DndContext>
  );
}
