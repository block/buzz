import {
  DndContext,
  PointerSensor,
  closestCenter,
  useDroppable,
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
import { Check, GripVertical, Plus, Star, Trash2 } from "lucide-react";
import * as React from "react";

import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";
import { groupBestieTodos } from "./bestieTodoGrouping";
import { todayLocalDayKey } from "./bestieListStorage";
import {
  addBestieListItemForScope,
  removeBestieListItemForScope,
  reorderBestieTodosForScope,
  setBestieListItemStatusForScope,
  toggleBestieListItemStarredForScope,
  useBestieList,
} from "./bestieListStore";
import type { BestieListItem, BestieListScope } from "./bestieListTypes";

type DropContainer =
  | { kind: "starred" }
  | { dayKey: string; kind: "day" };

function parseContainerId(id: string): DropContainer | null {
  if (id === "todo-starred") return { kind: "starred" };
  if (id.startsWith("todo-day:")) {
    return { dayKey: id.slice("todo-day:".length), kind: "day" };
  }
  return null;
}

function containerIdFor(container: DropContainer): string {
  return container.kind === "starred"
    ? "todo-starred"
    : `todo-day:${container.dayKey}`;
}

function TodoRow({
  item,
  onComplete,
  onRemove,
  onToggleStar,
}: {
  item: BestieListItem;
  onComplete: () => void;
  onRemove: () => void;
  onToggleStar: () => void;
}) {
  const {
    attributes,
    listeners,
    setNodeRef,
    transform,
    transition,
    isDragging,
  } = useSortable({ id: item.id, data: { item } });

  return (
    <div
      className={cn(
        "group flex items-start gap-1 rounded-md border border-border/60 bg-muted/25 px-1.5 py-1.5",
        isDragging && "opacity-40",
      )}
      data-testid={`bestie-list-item-${item.id}`}
      ref={setNodeRef}
      style={{
        transform: CSS.Transform.toString(transform),
        transition,
      }}
    >
      <button
        aria-label="Drag to reorder"
        className="mt-0.5 touch-none text-muted-foreground hover:text-foreground"
        data-testid={`bestie-todo-drag-${item.id}`}
        type="button"
        {...attributes}
        {...listeners}
      >
        <GripVertical className="size-3.5" />
      </button>
      <Button
        aria-label="Mark done"
        className="mt-0.5 size-6 shrink-0"
        data-testid={`bestie-list-toggle-${item.id}`}
        onClick={onComplete}
        size="icon-xs"
        type="button"
        variant="ghost"
      >
        <Check className="size-3.5" />
      </Button>
      <div className="min-w-0 flex-1">
        <p className="text-sm leading-snug">{item.text}</p>
      </div>
      <Button
        aria-label={item.starred ? "Unstar to-do" : "Star to-do"}
        aria-pressed={item.starred}
        className="mt-0.5 size-6 shrink-0"
        data-testid={`bestie-todo-star-${item.id}`}
        onClick={onToggleStar}
        size="icon-xs"
        type="button"
        variant="ghost"
      >
        <Star
          className={cn(
            "size-3.5",
            item.starred
              ? "fill-amber-400 text-amber-400"
              : "text-muted-foreground",
          )}
        />
      </Button>
      <Button
        aria-label="Remove item"
        className="mt-0.5 size-6 shrink-0 opacity-0 transition-opacity group-hover:opacity-100 focus-visible:opacity-100"
        data-testid={`bestie-list-remove-${item.id}`}
        onClick={onRemove}
        size="icon-xs"
        type="button"
        variant="ghost"
      >
        <Trash2 className="size-3.5" />
      </Button>
    </div>
  );
}

function TodoGroup({
  container,
  items,
  label,
  scope,
}: {
  container: DropContainer;
  items: BestieListItem[];
  label: string;
  scope: BestieListScope;
}) {
  const id = containerIdFor(container);
  const { setNodeRef, isOver } = useDroppable({ id, data: { container } });
  return (
    <div className="space-y-1.5" data-testid={`bestie-todo-group-${id}`}>
      <p className="px-0.5 text-2xs font-medium uppercase tracking-wide text-muted-foreground">
        {label}
      </p>
      <SortableContext
        id={id}
        items={items.map((item) => item.id)}
        strategy={verticalListSortingStrategy}
      >
        <div
          className={cn(
            "min-h-8 space-y-1.5 rounded-md",
            isOver && "ring-2 ring-primary/30",
          )}
          ref={setNodeRef}
        >
          {items.length === 0 ? (
            <p className="px-0.5 text-2xs text-muted-foreground/80">
              Drop to-dos here
            </p>
          ) : (
            items.map((item) => (
              <TodoRow
                key={item.id}
                item={item}
                onComplete={() =>
                  setBestieListItemStatusForScope(scope, item.id, "done")
                }
                onRemove={() => removeBestieListItemForScope(scope, item.id)}
                onToggleStar={() =>
                  toggleBestieListItemStarredForScope(scope, item.id)
                }
              />
            ))
          )}
        </div>
      </SortableContext>
    </div>
  );
}

/**
 * To-dos sheet: starred pin block, day groups (Today first), drag across groups.
 */
export function BestieDmTodosSheet({
  adding,
  onRequestAdd,
  scope,
}: {
  adding: boolean;
  onRequestAdd?: () => void;
  scope: BestieListScope;
}) {
  const state = useBestieList(scope);
  const { dayGroups, starred } = React.useMemo(
    () => groupBestieTodos(state.items),
    [state.items],
  );
  const [text, setText] = React.useState("");
  const inputRef = React.useRef<HTMLInputElement>(null);
  React.useEffect(() => {
    if (adding) inputRef.current?.focus();
  }, [adding]);

  const sensors = useSensors(
    useSensor(PointerSensor, { activationConstraint: { distance: 6 } }),
  );

  const findContainerForId = React.useCallback(
    (id: string): DropContainer | null => {
      const asContainer = parseContainerId(id);
      if (asContainer) return asContainer;
      if (starred.some((item) => item.id === id)) return { kind: "starred" };
      for (const group of dayGroups) {
        if (group.items.some((item) => item.id === id)) {
          return { dayKey: group.dayKey, kind: "day" };
        }
      }
      return null;
    },
    [dayGroups, starred],
  );

  const itemsFor = React.useCallback(
    (container: DropContainer): BestieListItem[] => {
      if (container.kind === "starred") return starred;
      return (
        dayGroups.find((group) => group.dayKey === container.dayKey)?.items ??
        []
      );
    },
    [dayGroups, starred],
  );

  const onDragEnd = (event: DragEndEvent) => {
    const activeId = String(event.active.id);
    const overId = event.over ? String(event.over.id) : null;
    if (!overId) return;
    const from = findContainerForId(activeId);
    const to = findContainerForId(overId) ?? parseContainerId(overId);
    if (!from || !to) return;

    const fromItems = itemsFor(from).map((item) => item.id);
    let toItems = itemsFor(to).map((item) => item.id);

    if (from.kind === to.kind &&
      (from.kind === "starred" ||
        (from.kind === "day" && to.kind === "day" && from.dayKey === to.dayKey))) {
      const oldIndex = fromItems.indexOf(activeId);
      const newIndex = toItems.indexOf(overId);
      if (oldIndex < 0) return;
      const nextIds =
        newIndex < 0
          ? fromItems
          : arrayMove(fromItems, oldIndex, newIndex);
      reorderBestieTodosForScope(scope, {
        dayKey: from.kind === "day" ? from.dayKey : undefined,
        orderedIds: nextIds,
        starred: from.kind === "starred",
      });
      return;
    }

    // Cross-group move
    const without = fromItems.filter((id) => id !== activeId);
    // Persist source group order without the moved item
    reorderBestieTodosForScope(scope, {
      dayKey: from.kind === "day" ? from.dayKey : undefined,
      orderedIds: without,
      starred: from.kind === "starred",
    });

    const overIndex = toItems.indexOf(overId);
    if (overIndex >= 0) {
      toItems = [
        ...toItems.slice(0, overIndex),
        activeId,
        ...toItems.slice(overIndex),
      ];
    } else {
      toItems = [...toItems, activeId];
    }
    // Deduplicate if active was already listed (empty-group drop uses container id)
    toItems = toItems.filter(
      (id, index) => id === activeId || toItems.indexOf(id) === index,
    );
    // Ensure active appears once
    const deduped: string[] = [];
    for (const id of toItems) {
      if (id === activeId && deduped.includes(activeId)) continue;
      deduped.push(id);
    }
    if (!deduped.includes(activeId)) deduped.push(activeId);

    reorderBestieTodosForScope(scope, {
      dayKey: to.kind === "day" ? to.dayKey : todayLocalDayKey(),
      orderedIds: deduped,
      starred: to.kind === "starred",
    });
  };

  const submit = () => {
    const trimmed = text.trim();
    if (!trimmed) return;
    addBestieListItemForScope(scope, {
      dayKey: todayLocalDayKey(),
      kind: "todo",
      text: trimmed,
    });
    setText("");
  };

  const doneTodos = state.items.filter(
    (item) => item.kind === "todo" && item.status === "done",
  );

  return (
    <div
      className="flex flex-col gap-3 py-1"
      data-testid="bestie-dm-category-sheet-todo"
    >
      {adding ? (
        <div className="flex items-center gap-1.5" data-testid="bestie-add-todo">
          <Input
            aria-label="Add a to-do"
            className="h-8 text-sm"
            onChange={(event) => setText(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === "Enter") {
                event.preventDefault();
                submit();
              }
            }}
            placeholder="Add a to-do"
            ref={inputRef}
            value={text}
          />
          <Button
            aria-label="Add todo"
            className="size-8 shrink-0"
            disabled={!text.trim()}
            onClick={submit}
            size="icon"
            type="button"
            variant="secondary"
          >
            <Plus className="size-4" />
          </Button>
        </div>
      ) : (
        <p className="px-0.5 text-xs text-muted-foreground">
          Star to pin, drag across days.{" "}
          {onRequestAdd ? (
            <button
              className="underline-offset-2 hover:underline"
              onClick={onRequestAdd}
              type="button"
            >
              Add one
            </button>
          ) : null}
        </p>
      )}

      <DndContext
        collisionDetection={closestCenter}
        onDragEnd={onDragEnd}
        sensors={sensors}
      >
        <div className="space-y-3">
          <TodoGroup
            container={{ kind: "starred" }}
            items={starred}
            label="Starred"
            scope={scope}
          />
          {dayGroups.map((group) => (
            <TodoGroup
              key={group.dayKey}
              container={{ dayKey: group.dayKey, kind: "day" }}
              items={group.items}
              label={group.label}
              scope={scope}
            />
          ))}
        </div>
      </DndContext>

      {doneTodos.length > 0 ? (
        <div className="space-y-1.5 border-t border-border/40 pt-2">
          <p className="px-0.5 text-2xs font-medium uppercase tracking-wide text-muted-foreground">
            Done
          </p>
          {doneTodos.map((item) => (
            <div
              key={item.id}
              className="flex items-center gap-2 rounded-md px-2 py-1 opacity-60"
              data-testid={`bestie-list-item-${item.id}`}
            >
              <Button
                aria-label="Reopen item"
                className="size-6 shrink-0"
                onClick={() =>
                  setBestieListItemStatusForScope(scope, item.id, "open")
                }
                size="icon-xs"
                type="button"
                variant="ghost"
              >
                <Check className="size-3.5 text-primary" />
              </Button>
              <p className="min-w-0 flex-1 truncate text-sm line-through">
                {item.text}
              </p>
              <Button
                aria-label="Remove item"
                className="size-6 shrink-0"
                onClick={() => removeBestieListItemForScope(scope, item.id)}
                size="icon-xs"
                type="button"
                variant="ghost"
              >
                <Trash2 className="size-3.5" />
              </Button>
            </div>
          ))}
        </div>
      ) : null}
    </div>
  );
}
