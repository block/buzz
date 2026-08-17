import {
  closestCenter,
  DndContext,
  DragOverlay,
  KeyboardSensor,
  PointerSensor,
  useSensor,
  useSensors,
} from "@dnd-kit/core";
import type { DragEndEvent, DragStartEvent } from "@dnd-kit/core";
import {
  rectSortingStrategy,
  sortableKeyboardCoordinates,
  SortableContext,
  useSortable,
} from "@dnd-kit/sortable";
import {
  Boxes,
  Compass,
  GripVertical,
  HandHeart,
  MessageSquareText,
  PackageCheck,
  Pencil,
  Plus,
  Settings2,
  Sparkles,
  StickyNote,
  Users,
} from "lucide-react";
import * as React from "react";

import {
  type CanvasBoardCard,
  type CanvasBoardCardKind,
  parseCanvasBoard,
} from "@/features/channels/lib/canvasBoard";
import { useChannelNavigation } from "@/shared/context/ChannelNavigationContext";
import { cn } from "@/shared/lib/cn";
import { channelChrome } from "@/shared/layout/chromeLayout";
import { Button } from "@/shared/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/shared/ui/card";
import { Markdown } from "@/shared/ui/markdown";
import { PubKey } from "@/shared/ui/PubKey";

const CARD_STYLES: Record<CanvasBoardCardKind, string> = {
  artifact: "border-emerald-500/25 bg-emerald-500/8",
  invitation: "border-rose-500/25 bg-rose-500/8",
  note: "border-violet-500/20 bg-violet-500/7",
  now: "border-amber-500/30 bg-amber-500/10",
  people: "border-sky-500/25 bg-sky-500/8",
  welcome: "border-cyan-500/25 bg-cyan-500/8",
};

const CARD_LABELS: Record<CanvasBoardCardKind, string> = {
  artifact: "Made here",
  invitation: "Open invitation",
  note: "Shared note",
  now: "Happening now",
  people: "People",
  welcome: "Start here",
};

const CARD_ICONS = {
  artifact: PackageCheck,
  invitation: HandHeart,
  note: StickyNote,
  now: Sparkles,
  people: Users,
  welcome: Compass,
} satisfies Record<CanvasBoardCardKind, typeof StickyNote>;

type ChannelBoardProps = {
  actionErrorMessage?: string;
  agentCount: number;
  author: string | null;
  canEdit: boolean;
  channelName: string;
  content: string | null;
  errorMessage?: string;
  isLoading: boolean;
  isSaving: boolean;
  memberCount: number;
  onCreateCard: () => void;
  onEditCard: (card: CanvasBoardCard) => void;
  onManageBoard: () => void;
  onMoveCard: (activeCardId: string, overCardId: string) => void;
  onOpenMembers: () => void;
  onOpenStream: () => void;
  updatedAt: number | null;
};

function formatCanvasUpdatedAt(updatedAt: number | null): string | null {
  if (updatedAt === null) {
    return null;
  }
  return new Intl.DateTimeFormat(undefined, {
    dateStyle: "medium",
    timeStyle: "short",
  }).format(new Date(updatedAt * 1_000));
}

function BoardCard({
  card,
  canEdit,
  channelNames,
  isSaving,
  onEdit,
}: {
  card: CanvasBoardCard;
  canEdit: boolean;
  channelNames: string[];
  isSaving: boolean;
  onEdit: () => void;
}) {
  const Icon = CARD_ICONS[card.kind];
  const dragDisabled = !canEdit || isSaving;
  const {
    attributes,
    isDragging,
    isOver,
    listeners,
    setActivatorNodeRef,
    setNodeRef,
  } = useSortable({
    id: card.id,
    disabled: dragDisabled,
  });

  return (
    <div
      className={cn(
        "mb-4 break-inside-avoid rounded-xl transition-shadow",
        isDragging && "opacity-35",
        isOver && !isDragging && "ring-2 ring-primary/50 ring-offset-2",
      )}
      data-board-kind={card.kind}
      data-testid={`magic-board-card-${card.id}`}
      ref={setNodeRef}
    >
      <Card
        className={cn(
          "overflow-hidden shadow-sm transition-shadow duration-200 hover:shadow-md",
          CARD_STYLES[card.kind],
        )}
      >
        <CardHeader className="gap-3 p-5 pb-3">
          <div className="flex items-start justify-between gap-3">
            <div className="flex items-center gap-2 pt-1 text-xs font-semibold uppercase tracking-[0.14em] text-muted-foreground">
              <Icon className="h-3.5 w-3.5" />
              {CARD_LABELS[card.kind]}
            </div>
            {canEdit ? (
              <div className="flex shrink-0 items-center gap-1">
                <Button
                  aria-label={`Move ${card.title}`}
                  className="touch-none cursor-grab text-muted-foreground active:cursor-grabbing"
                  data-testid={`magic-board-drag-${card.id}`}
                  disabled={isSaving}
                  ref={setActivatorNodeRef}
                  size="icon-xs"
                  type="button"
                  variant="ghost"
                  {...attributes}
                  {...listeners}
                >
                  <GripVertical />
                </Button>
                <Button
                  aria-label={`Edit ${card.title}`}
                  data-testid={`magic-board-edit-${card.id}`}
                  disabled={isSaving}
                  onClick={onEdit}
                  size="icon-xs"
                  type="button"
                  variant="ghost"
                >
                  <Pencil />
                </Button>
              </div>
            ) : null}
          </div>
          <CardTitle className="text-lg leading-6">{card.title}</CardTitle>
        </CardHeader>
        {card.body ? (
          <CardContent className="p-5 pt-0 text-sm">
            <Markdown channelNames={channelNames} content={card.body} />
          </CardContent>
        ) : null}
      </Card>
    </div>
  );
}

function BoardCardDragOverlay({ card }: { card: CanvasBoardCard }) {
  const Icon = CARD_ICONS[card.kind];

  return (
    <Card
      className={cn(
        "w-72 overflow-hidden shadow-xl ring-1 ring-primary/30",
        CARD_STYLES[card.kind],
      )}
      data-testid="magic-board-drag-overlay"
    >
      <CardHeader className="gap-2 p-4">
        <div className="flex items-center gap-2 text-xs font-semibold uppercase tracking-[0.14em] text-muted-foreground">
          <Icon className="h-3.5 w-3.5" />
          {CARD_LABELS[card.kind]}
        </div>
        <CardTitle className="text-base leading-5">{card.title}</CardTitle>
      </CardHeader>
    </Card>
  );
}

function LiveBoardCards({
  agentCount,
  memberCount,
  onOpenMembers,
  onOpenStream,
}: {
  agentCount: number;
  memberCount: number;
  onOpenMembers: () => void;
  onOpenStream: () => void;
}) {
  return (
    <React.Fragment>
      <Card className="mb-4 break-inside-avoid border-sky-500/25 bg-sky-500/8 shadow-sm">
        <CardHeader className="gap-3 p-5 pb-3">
          <div className="flex items-center gap-2 text-xs font-semibold uppercase tracking-[0.14em] text-muted-foreground">
            <Users className="h-3.5 w-3.5" />
            Live from Buzz
          </div>
          <CardTitle className="text-lg leading-6">People & agents</CardTitle>
        </CardHeader>
        <CardContent className="space-y-4 p-5 pt-0 text-sm">
          <p className="text-muted-foreground">
            {memberCount === 1 ? "1 member" : `${memberCount} members`} ·{" "}
            {agentCount === 1 ? "1 agent" : `${agentCount} agents`} tending this
            room.
          </p>
          <Button
            onClick={onOpenMembers}
            size="sm"
            type="button"
            variant="outline"
          >
            <Users className="h-4 w-4" />
            View people
          </Button>
        </CardContent>
      </Card>

      <Card className="mb-4 break-inside-avoid border-border/70 bg-card/70 shadow-sm">
        <CardHeader className="gap-3 p-5 pb-3">
          <div className="flex items-center gap-2 text-xs font-semibold uppercase tracking-[0.14em] text-muted-foreground">
            <MessageSquareText className="h-3.5 w-3.5" />
            Contained conversation
          </div>
          <CardTitle className="text-lg leading-6">Open the room</CardTitle>
        </CardHeader>
        <CardContent className="space-y-4 p-5 pt-0 text-sm">
          <p className="text-muted-foreground">
            Continue into the channel stream without losing this shared board.
          </p>
          <Button onClick={onOpenStream} size="sm" type="button">
            <MessageSquareText className="h-4 w-4" />
            Open conversation
          </Button>
        </CardContent>
      </Card>
    </React.Fragment>
  );
}

export function ChannelBoard({
  actionErrorMessage,
  agentCount,
  author,
  canEdit,
  channelName,
  content,
  errorMessage,
  isLoading,
  isSaving,
  memberCount,
  onCreateCard,
  onEditCard,
  onManageBoard,
  onMoveCard,
  onOpenMembers,
  onOpenStream,
  updatedAt,
}: ChannelBoardProps) {
  const { channels } = useChannelNavigation();
  const channelNames = React.useMemo(
    () =>
      channels
        .filter((channel) => channel.channelType !== "dm")
        .map((channel) => channel.name),
    [channels],
  );
  const board = React.useMemo(() => parseCanvasBoard(content ?? ""), [content]);
  const updatedLabel = formatCanvasUpdatedAt(updatedAt);
  const [activeCardId, setActiveCardId] = React.useState<string | null>(null);
  const activeCard = activeCardId
    ? (board.cards.find((card) => card.id === activeCardId) ?? null)
    : null;
  const sensors = useSensors(
    useSensor(PointerSensor, { activationConstraint: { distance: 6 } }),
    useSensor(KeyboardSensor, {
      coordinateGetter: sortableKeyboardCoordinates,
    }),
  );

  function handleDragStart(event: DragStartEvent) {
    setActiveCardId(String(event.active.id));
  }

  function handleDragEnd(event: DragEndEvent) {
    setActiveCardId(null);
    if (!event.over || event.active.id === event.over.id) {
      return;
    }
    onMoveCard(String(event.active.id), String(event.over.id));
  }

  return (
    <div
      className={cn(
        "min-h-0 flex-1 overflow-y-auto bg-[radial-gradient(circle_at_top_left,hsl(var(--muted)/0.45),transparent_38%)]",
        channelChrome.contentPadding,
      )}
      data-testid="channel-magic-board"
    >
      <div className="mx-auto w-full max-w-7xl px-5 pb-10 pt-6 sm:px-7 lg:px-10">
        <section className="mb-6 rounded-2xl border border-border/60 bg-background/75 p-5 shadow-sm backdrop-blur-sm sm:p-7">
          <div className="flex flex-col gap-5 lg:flex-row lg:items-start lg:justify-between">
            <div className="max-w-3xl space-y-3">
              <div className="flex items-center gap-2 text-xs font-semibold uppercase tracking-[0.16em] text-muted-foreground">
                <Boxes className="h-4 w-4" />
                Shared community board
              </div>
              <h2 className="text-2xl font-semibold tracking-tight sm:text-3xl">
                {board.title ?? channelName}
              </h2>
              {board.introduction ? (
                <div className="text-sm text-muted-foreground sm:text-base">
                  <Markdown
                    channelNames={channelNames}
                    content={board.introduction}
                  />
                </div>
              ) : (
                <p className="text-sm text-muted-foreground">
                  See what matters now, where to join, and what this community
                  has made.
                </p>
              )}
              <div className="flex flex-wrap items-center gap-x-2 gap-y-1 text-xs text-muted-foreground">
                <span>Snapshot of this channel’s shared canvas</span>
                {author ? (
                  <React.Fragment>
                    <span aria-hidden>·</span>
                    <span className="inline-flex items-center gap-1">
                      by <PubKey pubkey={author} />
                    </span>
                  </React.Fragment>
                ) : null}
                {updatedLabel ? (
                  <React.Fragment>
                    <span aria-hidden>·</span>
                    <span>Updated {updatedLabel}</span>
                  </React.Fragment>
                ) : null}
              </div>
            </div>
            <div className="flex shrink-0 flex-wrap gap-2">
              {canEdit ? (
                <Button
                  data-testid="magic-board-create-card"
                  disabled={isSaving}
                  onClick={onCreateCard}
                  size="sm"
                  type="button"
                >
                  <Plus className="h-4 w-4" />
                  New card
                </Button>
              ) : null}
              <Button onClick={onOpenStream} size="sm" type="button">
                <MessageSquareText className="h-4 w-4" />
                Open stream
              </Button>
              <Button
                onClick={onManageBoard}
                size="sm"
                type="button"
                variant="outline"
              >
                <Settings2 className="h-4 w-4" />
                Board settings
              </Button>
            </div>
          </div>
        </section>

        {actionErrorMessage ? (
          <p
            className="mb-4 rounded-xl border border-destructive/30 bg-destructive/10 px-4 py-3 text-sm text-destructive"
            data-testid="magic-board-action-error"
          >
            {actionErrorMessage}
          </p>
        ) : null}

        {isLoading ? (
          <div className="rounded-xl border border-border/60 bg-background/60 px-5 py-8 text-sm text-muted-foreground">
            Loading community board…
          </div>
        ) : errorMessage ? (
          <div className="rounded-xl border border-destructive/30 bg-destructive/10 px-5 py-4 text-sm text-destructive">
            {errorMessage}
          </div>
        ) : board.cards.length === 0 ? (
          <div className="rounded-xl border border-dashed border-border bg-background/60 px-5 py-10 text-center">
            <StickyNote className="mx-auto mb-3 h-7 w-7 text-muted-foreground" />
            <h3 className="text-base font-semibold">
              This board is ready for its first note.
            </h3>
            <p className="mx-auto mt-1 max-w-lg text-sm text-muted-foreground">
              Add Markdown sections to the channel canvas. Each level-two
              heading becomes a shared module here.
            </p>
            <Button
              className="mt-4"
              onClick={onManageBoard}
              size="sm"
              type="button"
              variant="outline"
            >
              <Settings2 className="h-4 w-4" />
              Open board settings
            </Button>
          </div>
        ) : (
          <DndContext
            collisionDetection={closestCenter}
            onDragCancel={() => setActiveCardId(null)}
            onDragEnd={handleDragEnd}
            onDragStart={handleDragStart}
            sensors={sensors}
          >
            <SortableContext
              items={board.cards.map((card) => card.id)}
              strategy={rectSortingStrategy}
            >
              <div
                className="columns-1 gap-4 md:columns-2 xl:columns-3"
                data-testid="magic-board-grid"
              >
                {board.cards.map((card) => (
                  <BoardCard
                    canEdit={canEdit}
                    card={card}
                    channelNames={channelNames}
                    isSaving={isSaving}
                    key={card.id}
                    onEdit={() => onEditCard(card)}
                  />
                ))}
                <LiveBoardCards
                  agentCount={agentCount}
                  memberCount={memberCount}
                  onOpenMembers={onOpenMembers}
                  onOpenStream={onOpenStream}
                />
              </div>
            </SortableContext>
            <DragOverlay dropAnimation={null}>
              {activeCard ? <BoardCardDragOverlay card={activeCard} /> : null}
            </DragOverlay>
          </DndContext>
        )}
      </div>
    </div>
  );
}
