import {
  closestCenter,
  DndContext,
  DragOverlay,
  KeyboardSensor,
  pointerWithin,
  PointerSensor,
  useDroppable,
  useSensor,
  useSensors,
} from "@dnd-kit/core";
import type {
  CollisionDetection,
  DragEndEvent,
  DragStartEvent,
} from "@dnd-kit/core";
import {
  rectSortingStrategy,
  sortableKeyboardCoordinates,
  SortableContext,
  useSortable,
} from "@dnd-kit/sortable";
import {
  Bot,
  Boxes,
  CheckCircle2,
  CircleDot,
  Columns3,
  FileCheck2,
  FolderKanban,
  Gavel,
  GripVertical,
  LayoutGrid,
  ListTodo,
  MessageCircle,
  MessageSquareText,
  Pencil,
  Plus,
  Settings2,
  Sparkles,
  StickyNote,
  UserRound,
  Users,
} from "lucide-react";
import * as React from "react";

import {
  type CanvasBoardCard,
  type CanvasBoardCardStatus,
  type CanvasBoardCardType,
  parseCanvasBoard,
} from "@/features/channels/lib/canvasBoard";
import { useChannelNavigation } from "@/shared/context/ChannelNavigationContext";
import { cn } from "@/shared/lib/cn";
import { channelChrome } from "@/shared/layout/chromeLayout";
import { Button } from "@/shared/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/shared/ui/card";
import { Markdown } from "@/shared/ui/markdown";
import { PubKey } from "@/shared/ui/PubKey";

const CARD_TYPE_STYLES: Record<CanvasBoardCardType, string> = {
  agent: "border-fuchsia-500/25 bg-fuchsia-500/8",
  artifact: "border-emerald-500/25 bg-emerald-500/8",
  conversation: "border-blue-500/25 bg-blue-500/8",
  decision: "border-orange-500/25 bg-orange-500/8",
  note: "border-violet-500/20 bg-violet-500/7",
  person: "border-sky-500/25 bg-sky-500/8",
  project: "border-cyan-500/25 bg-cyan-500/8",
  task: "border-amber-500/30 bg-amber-500/10",
};

const CARD_TYPE_LABELS: Record<CanvasBoardCardType, string> = {
  agent: "Agent",
  artifact: "Artifact",
  conversation: "Conversation",
  decision: "Decision",
  note: "Note",
  person: "Person",
  project: "Project",
  task: "Task",
};

const CARD_TYPE_ICONS = {
  agent: Bot,
  artifact: FileCheck2,
  conversation: MessageCircle,
  decision: Gavel,
  note: StickyNote,
  person: UserRound,
  project: FolderKanban,
  task: ListTodo,
} satisfies Record<CanvasBoardCardType, typeof StickyNote>;

const STATUS_LABELS: Record<CanvasBoardCardStatus, string> = {
  backlog: "Backlog",
  doing: "Doing",
  done: "Done",
};

const STATUS_ICONS = {
  backlog: CircleDot,
  doing: Sparkles,
  done: CheckCircle2,
} satisfies Record<CanvasBoardCardStatus, typeof CircleDot>;

const KANBAN_STATUSES: CanvasBoardCardStatus[] = ["backlog", "doing", "done"];

const boardCollisionDetection: CollisionDetection = (args) => {
  const pointerCollisions = pointerWithin(args);
  return pointerCollisions.length > 0 ? pointerCollisions : closestCenter(args);
};

type CanvasBoardLayout = "cards" | "kanban";

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
  onChangeCardStatus: (
    card: CanvasBoardCard,
    status: CanvasBoardCardStatus,
  ) => void;
  onEditCard: (card: CanvasBoardCard) => void;
  onManageBoard: () => void;
  onMoveCard: (activeCardId: string, overCardId: string) => void;
  onOpenMembers: () => void;
  onOpenCardConversation: (card: CanvasBoardCard) => void;
  onOpenStream: () => void;
  pendingConversationCardId?: string | null;
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
  isConversationPending,
  isSaving,
  onEdit,
  onOpenConversation,
}: {
  card: CanvasBoardCard;
  canEdit: boolean;
  channelNames: string[];
  isConversationPending: boolean;
  isSaving: boolean;
  onEdit: () => void;
  onOpenConversation: () => void;
}) {
  const Icon = CARD_TYPE_ICONS[card.type];
  const StatusIcon = STATUS_ICONS[card.status];
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
      data-board-status={card.status}
      data-board-type={card.type}
      data-testid={`magic-board-card-${card.id}`}
      ref={setNodeRef}
    >
      <Card
        className={cn(
          "overflow-hidden shadow-sm transition-shadow duration-200 hover:shadow-md",
          CARD_TYPE_STYLES[card.type],
        )}
      >
        <CardHeader className="gap-3 p-5 pb-3">
          <div className="flex items-start justify-between gap-3">
            <div className="flex items-center gap-2 pt-1 text-xs font-semibold uppercase tracking-[0.14em] text-muted-foreground">
              <Icon className="h-3.5 w-3.5" />
              {CARD_TYPE_LABELS[card.type]}
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
        <CardContent className="flex flex-wrap items-center justify-between gap-2 border-t border-border/40 p-4 text-xs text-muted-foreground">
          <span className="inline-flex flex-wrap items-center gap-1.5">
            <StatusIcon className="h-3.5 w-3.5" />
            {STATUS_LABELS[card.status]}
            {card.author ? (
              <React.Fragment>
                <span aria-hidden>·</span>
                <span className="inline-flex items-center gap-1">
                  by <PubKey pubkey={card.author} />
                </span>
              </React.Fragment>
            ) : null}
          </span>
          {card.threadId || canEdit ? (
            <Button
              data-testid={`magic-board-conversation-${card.id}`}
              disabled={isSaving || isConversationPending}
              onClick={onOpenConversation}
              size="xs"
              type="button"
              variant={card.threadId ? "secondary" : "outline"}
            >
              <MessageCircle className="h-3.5 w-3.5" />
              {isConversationPending
                ? "Linking…"
                : card.threadId
                  ? "Open thread"
                  : "Start thread"}
            </Button>
          ) : null}
        </CardContent>
      </Card>
    </div>
  );
}

function BoardCardDragOverlay({ card }: { card: CanvasBoardCard }) {
  const Icon = CARD_TYPE_ICONS[card.type];

  return (
    <Card
      className={cn(
        "w-72 overflow-hidden shadow-xl ring-1 ring-primary/30",
        CARD_TYPE_STYLES[card.type],
      )}
      data-testid="magic-board-drag-overlay"
    >
      <CardHeader className="gap-2 p-4">
        <div className="flex items-center gap-2 text-xs font-semibold uppercase tracking-[0.14em] text-muted-foreground">
          <Icon className="h-3.5 w-3.5" />
          {CARD_TYPE_LABELS[card.type]}
        </div>
        <CardTitle className="text-base leading-5">{card.title}</CardTitle>
      </CardHeader>
    </Card>
  );
}

function KanbanColumn({
  cards,
  canEdit,
  channelNames,
  isSaving,
  onEditCard,
  onOpenCardConversation,
  pendingConversationCardId,
  status,
}: {
  cards: CanvasBoardCard[];
  canEdit: boolean;
  channelNames: string[];
  isSaving: boolean;
  onEditCard: (card: CanvasBoardCard) => void;
  onOpenCardConversation: (card: CanvasBoardCard) => void;
  pendingConversationCardId?: string | null;
  status: CanvasBoardCardStatus;
}) {
  const { isOver, setNodeRef } = useDroppable({ id: `status:${status}` });
  const StatusIcon = STATUS_ICONS[status];

  return (
    <section
      className={cn(
        "min-h-48 rounded-2xl border border-border/60 bg-background/55 p-3 transition-colors",
        isOver && "border-primary/50 bg-primary/5",
      )}
      data-testid={`magic-board-kanban-${status}`}
      ref={setNodeRef}
    >
      <header className="mb-3 flex items-center justify-between gap-2 px-1">
        <div className="flex items-center gap-2 text-sm font-semibold">
          <StatusIcon className="h-4 w-4 text-muted-foreground" />
          {STATUS_LABELS[status]}
        </div>
        <span className="rounded-full bg-muted px-2 py-0.5 text-xs text-muted-foreground">
          {cards.length}
        </span>
      </header>
      <SortableContext items={cards.map((card) => card.id)}>
        <div className="space-y-3">
          {cards.map((card) => (
            <BoardCard
              canEdit={canEdit}
              card={card}
              channelNames={channelNames}
              isConversationPending={pendingConversationCardId === card.id}
              isSaving={isSaving}
              key={card.id}
              onEdit={() => onEditCard(card)}
              onOpenConversation={() => onOpenCardConversation(card)}
            />
          ))}
          {cards.length === 0 ? (
            <div className="rounded-xl border border-dashed border-border/70 px-3 py-8 text-center text-xs text-muted-foreground">
              {canEdit ? "Drop a card here" : "No cards"}
            </div>
          ) : null}
        </div>
      </SortableContext>
    </section>
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
  onChangeCardStatus,
  onCreateCard,
  onEditCard,
  onManageBoard,
  onMoveCard,
  onOpenCardConversation,
  onOpenMembers,
  onOpenStream,
  pendingConversationCardId,
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
  const [layout, setLayout] = React.useState<CanvasBoardLayout>("cards");
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
    const activeCard = board.cards.find(
      (card) => card.id === String(event.active.id),
    );
    const overId = String(event.over.id);
    const overCard = board.cards.find((card) => card.id === overId);
    const targetStatus = overId.startsWith("status:")
      ? (overId.slice("status:".length) as CanvasBoardCardStatus)
      : overCard?.status;

    if (
      layout === "kanban" &&
      activeCard &&
      targetStatus &&
      targetStatus !== activeCard.status
    ) {
      onChangeCardStatus(activeCard, targetStatus);
      return;
    }
    if (overCard) {
      onMoveCard(activeCard?.id ?? String(event.active.id), overCard.id);
    }
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
              <p className="text-xs text-muted-foreground">
                Stewards curate the shared layout; every member can join linked
                card threads.
              </p>
            </div>
            <div className="flex shrink-0 flex-wrap gap-2">
              <fieldset
                aria-label="Board layout"
                className="inline-flex rounded-lg border border-border bg-muted/40 p-0.5"
                data-testid="magic-board-layout"
              >
                <Button
                  aria-pressed={layout === "cards"}
                  data-testid="magic-board-layout-cards"
                  onClick={() => setLayout("cards")}
                  size="sm"
                  type="button"
                  variant={layout === "cards" ? "secondary" : "ghost"}
                >
                  <LayoutGrid className="h-4 w-4" />
                  Cards
                </Button>
                <Button
                  aria-pressed={layout === "kanban"}
                  data-testid="magic-board-layout-kanban"
                  onClick={() => setLayout("kanban")}
                  size="sm"
                  type="button"
                  variant={layout === "kanban" ? "secondary" : "ghost"}
                >
                  <Columns3 className="h-4 w-4" />
                  Kanban
                </Button>
              </fieldset>
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
            collisionDetection={boardCollisionDetection}
            onDragCancel={() => setActiveCardId(null)}
            onDragEnd={handleDragEnd}
            onDragStart={handleDragStart}
            sensors={sensors}
          >
            {layout === "cards" ? (
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
                      isConversationPending={
                        pendingConversationCardId === card.id
                      }
                      isSaving={isSaving}
                      key={card.id}
                      onEdit={() => onEditCard(card)}
                      onOpenConversation={() => onOpenCardConversation(card)}
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
            ) : (
              <div
                className="grid gap-4 lg:grid-cols-3"
                data-testid="magic-board-kanban"
              >
                {KANBAN_STATUSES.map((status) => (
                  <KanbanColumn
                    canEdit={canEdit}
                    cards={board.cards.filter((card) => card.status === status)}
                    channelNames={channelNames}
                    isSaving={isSaving}
                    key={status}
                    onEditCard={onEditCard}
                    onOpenCardConversation={onOpenCardConversation}
                    pendingConversationCardId={pendingConversationCardId}
                    status={status}
                  />
                ))}
              </div>
            )}
            <DragOverlay dropAnimation={null}>
              {activeCard ? <BoardCardDragOverlay card={activeCard} /> : null}
            </DragOverlay>
          </DndContext>
        )}
      </div>
    </div>
  );
}
