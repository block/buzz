import { Bell, Briefcase, Check, Coffee, ListTodo, MessagesSquare, Plus, StickyNote, Trash2 } from "lucide-react";
import * as React from "react";

import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";
import {
  addBestieListItemForScope,
  removeBestieListItemForScope,
  setBestieListItemStatusForScope,
  useBestieList,
} from "./bestieListStore";
import {
  formatBestieReminderDueAt,
  presentBestieReminderRepeat,
} from "./bestieListStorage";
import type {
  BestieListItem,
  BestieListKind,
  BestieListScope,
  BestieReminderRepeat,
} from "./bestieListTypes";
import {
  dueAtFromDatetimeLocal,
  presentBestieContextCount,
  soonestEnabledJobDueAt,
  soonestOpenReminderDueAt,
  type BestieRhsKind,
} from "./bestieDmRhsHelpers";
import { enabledJobs } from "./bestieJobStorage";
import { useBestieJobs } from "./bestieJobStore";
import { BESTIE_COFFEE_LIVE_LABEL } from "./bestieCoffeeLive";
import { BESTIE_THREAD_SUMMARIZE_LIVE_LABEL } from "./bestieThreadSummarizeLive";
import { useBestieCoffee } from "./bestieCoffeeStore";
import { useBestieThreads } from "./bestieThreadStore";
import { useBestieScratch } from "./bestieScratchStore";
import { BestieDueCountdownChip } from "./BestieDueCountdownChip";

export {
  bestieCategoryTitle,
  datetimeLocalFromDueAt,
  dueAtFromDatetimeLocal,
  presentBestieContextCount,
} from "./bestieDmRhsHelpers";
export type { BestieRhsKind } from "./bestieDmRhsHelpers";

/** Match project home context row chrome (icon + label + count). */
const BESTIE_RHS_ROW_CLASS =
  "h-8 w-full justify-start gap-2 rounded-md px-2 py-1.5 text-left text-sm font-normal text-sidebar-foreground/80 transition-[background-color,color] hover:bg-sidebar-accent hover:text-sidebar-accent-foreground disabled:pointer-events-none disabled:opacity-50";

function CategoryRowContent({
  children,
  count,
  dueAt,
  dueChipTestId,
  icon,
}: {
  children: React.ReactNode;
  count?: number;
  dueAt?: number | null;
  dueChipTestId?: string;
  icon: React.ReactNode;
}) {
  return (
    <>
      <span className="flex size-4 shrink-0 items-center justify-center text-current">
        {icon}
      </span>
      <span className="min-w-0 flex-1 truncate text-left">{children}</span>
      {dueAt != null ? (
        <BestieDueCountdownChip dueAt={dueAt} testId={dueChipTestId} />
      ) : null}
      <span className="w-8 shrink-0 text-right tabular-nums text-current opacity-60">
        {count ?? ""}
      </span>
    </>
  );
}

function CategoryNavButton({
  children,
  count,
  dueAt,
  dueChipTestId,
  icon,
  onClick,
  pressed,
  testId,
}: {
  children: React.ReactNode;
  count?: number;
  dueAt?: number | null;
  dueChipTestId?: string;
  icon: React.ReactNode;
  onClick?: () => void;
  pressed?: boolean;
  testId?: string;
}) {
  return (
    <Button
      aria-pressed={pressed}
      className={cn(
        BESTIE_RHS_ROW_CLASS,
        pressed &&
          "bg-sidebar-active text-sidebar-active-foreground shadow-xs hover:bg-sidebar-active hover:text-sidebar-active-foreground",
      )}
      data-testid={testId}
      onClick={onClick}
      size="sm"
      type="button"
      variant="ghost"
    >
      <CategoryRowContent
        count={count}
        dueAt={dueAt}
        dueChipTestId={dueChipTestId}
        icon={icon}
      >
        {children}
      </CategoryRowContent>
    </Button>
  );
}

function ListRow({
  item,
  onComplete,
  onRemove,
  onReopen,
}: {
  item: BestieListItem;
  onComplete: () => void;
  onRemove: () => void;
  onReopen: () => void;
}) {
  const done = item.status === "done";
  const isReminder = item.kind === "reminder";
  return (
    <div
      className={cn(
        "group flex items-start gap-2 rounded-md border border-border/60 bg-muted/25 px-2 py-1.5",
        done && "opacity-60",
      )}
      data-testid={`bestie-list-item-${item.id}`}
    >
      <Button
        aria-label={done ? "Reopen item" : "Mark done"}
        className="mt-0.5 size-6 shrink-0"
        data-testid={`bestie-list-toggle-${item.id}`}
        onClick={done ? onReopen : onComplete}
        size="icon-xs"
        type="button"
        variant="ghost"
      >
        <Check className={cn("size-3.5", done && "text-primary")} />
      </Button>
      <div className="min-w-0 flex-1">
        <div className="flex items-start gap-1.5">
          <p
            className={cn(
              "min-w-0 flex-1 text-sm leading-snug",
              done && "line-through text-muted-foreground",
            )}
            data-testid={
              isReminder ? `bestie-reminder-text-${item.id}` : undefined
            }
          >
            {item.text}
          </p>
          {isReminder && item.dueAt != null && !done ? (
            <BestieDueCountdownChip
              dueAt={item.dueAt}
              testId={`bestie-due-chip-${item.id}`}
            />
          ) : null}
        </div>
        {isReminder ? (
          <div className="mt-0.5 flex flex-wrap items-center gap-1.5 text-2xs text-muted-foreground">
            <span
              className="inline-flex items-center rounded-full border border-border/70 bg-background/70 px-1.5 py-0.5 font-medium uppercase tracking-wide text-[10px] text-muted-foreground"
              data-testid={`bestie-reminder-repeat-${item.id}`}
            >
              {presentBestieReminderRepeat(item.repeat)}
            </span>
            {item.dueAt != null ? (
              <span data-testid={`bestie-reminder-due-${item.id}`}>
                Next {formatBestieReminderDueAt(item.dueAt)}
              </span>
            ) : (
              <span>No due time</span>
            )}
          </div>
        ) : null}
      </div>
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

function AddRow({
  kind,
  onAdd,
  placeholder,
  testId,
}: {
  kind: BestieListKind;
  onAdd: (
    text: string,
    dueAt: number | null,
    repeat: BestieReminderRepeat | null,
  ) => void;
  placeholder: string;
  testId: string;
}) {
  const [text, setText] = React.useState("");
  const [dueLocal, setDueLocal] = React.useState("");
  const [repeatKind, setRepeatKind] = React.useState<"once" | "daily">("once");
  const inputRef = React.useRef<HTMLInputElement>(null);
  React.useEffect(() => {
    inputRef.current?.focus();
  }, []);
  const submit = () => {
    const trimmed = text.trim();
    if (!trimmed) return;
    const repeat: BestieReminderRepeat | null =
      kind === "reminder" && repeatKind === "daily" ? { kind: "daily" } : null;
    onAdd(
      trimmed,
      kind === "reminder" ? dueAtFromDatetimeLocal(dueLocal) : null,
      repeat,
    );
    setText("");
    setDueLocal("");
    setRepeatKind("once");
  };
  return (
    <div className="flex flex-col gap-1.5" data-testid={testId}>
      <div className="flex items-center gap-1.5">
        <Input
          aria-label={placeholder}
          className="h-8 text-sm"
          onChange={(event) => setText(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Enter") {
              event.preventDefault();
              submit();
            }
            if (event.key === "Escape") {
              event.preventDefault();
              setText("");
              setDueLocal("");
              setRepeatKind("once");
            }
          }}
          placeholder={placeholder}
          ref={inputRef}
          value={text}
        />
        <Button
          aria-label={`Add ${kind}`}
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
      {kind === "reminder" ? (
        <>
          <div className="flex items-center gap-1.5">
            <Input
              aria-label="Due date and time"
              className="h-8 text-sm"
              data-testid="bestie-add-reminder-due"
              onChange={(event) => setDueLocal(event.target.value)}
              type="datetime-local"
              value={dueLocal}
            />
            {dueLocal ? (
              <Button
                aria-label="Clear due time"
                className="h-8 shrink-0 px-2 text-xs"
                data-testid="bestie-add-reminder-due-clear"
                onClick={() => setDueLocal("")}
                type="button"
                variant="ghost"
              >
                Clear
              </Button>
            ) : null}
          </div>
          <select
            aria-label="Reminder repeat"
            className="h-8 rounded-md border border-input bg-background px-2 text-sm"
            data-testid="bestie-add-reminder-repeat"
            onChange={(event) =>
              setRepeatKind(event.target.value as "once" | "daily")
            }
            value={repeatKind}
          >
            <option value="once">One-off</option>
            <option value="daily">Daily</option>
          </select>
        </>
      ) : null}
    </div>
  );
}

/**
 * Bestie DM fixed RHS category list — project-home style rows (icon + label +
 * count). Lives in the native right column (not idleAuxiliary). Clicking a
 * category opens the category slide sheet for that kind’s items.
 */
export function BestieDmRhsPanel({
  activeKind = null,
  coffeeLive = false,
  compact = false,
  onOpenKind,
  scope,
  summarizeLive = false,
}: {
  activeKind?: BestieRhsKind | null;
  /** True while /hula-coffee ACP turn is live — show 🤔… on the Coffee row. */
  coffeeLive?: boolean;
  /**
   * Popover Lists: drop the DM RHS bottom padding that otherwise forces a
   * phantom scrollbar when every category row already fits.
   */
  compact?: boolean;
  onOpenKind: (kind: BestieRhsKind) => void;
  scope: BestieListScope;
  /** True while thread summarize ACP turn is live — show 🤔… on Threads. */
  summarizeLive?: boolean;
}) {
  const state = useBestieList(scope);
  const jobState = useBestieJobs(scope);
  const coffeeState = useBestieCoffee(scope);
  const threadState = useBestieThreads(scope);
  const scratchState = useBestieScratch(scope);
  const openReminders = state.items.filter(
    (item) => item.kind === "reminder" && item.status === "open",
  ).length;
  const openTodos = state.items.filter(
    (item) => item.kind === "todo" && item.status === "open",
  ).length;
  const openJobs = enabledJobs(jobState).length;
  const remindersSoonestDueAt = soonestOpenReminderDueAt(state);
  const jobsSoonestDueAt = soonestEnabledJobDueAt(jobState);
  const coffeeCount = coffeeState.entries.length;
  const threadCount = threadState.threads.length;
  const scratchCount = scratchState.notes.length;

  return (
    <div
      className={cn("space-y-1 px-2 pt-3", compact ? "pb-1" : "pb-8")}
      data-testid="bestie-dm-rhs-panel"
    >
      <CategoryNavButton
        count={presentBestieContextCount(openReminders)}
        dueAt={remindersSoonestDueAt}
        dueChipTestId="bestie-rhs-reminders-due-chip"
        icon={<Bell className="size-4" />}
        onClick={() => onOpenKind("reminder")}
        pressed={activeKind === "reminder"}
        testId="bestie-rhs-reminders"
      >
        Reminders
      </CategoryNavButton>
      <CategoryNavButton
        count={presentBestieContextCount(openTodos)}
        icon={<ListTodo className="size-4" />}
        onClick={() => onOpenKind("todo")}
        pressed={activeKind === "todo"}
        testId="bestie-rhs-todos"
      >
        To-dos
      </CategoryNavButton>
      <CategoryNavButton
        count={presentBestieContextCount(openJobs)}
        dueAt={jobsSoonestDueAt}
        dueChipTestId="bestie-rhs-jobs-due-chip"
        icon={<Briefcase className="size-4" />}
        onClick={() => onOpenKind("job")}
        pressed={activeKind === "job"}
        testId="bestie-rhs-jobs"
      >
        Jobs
      </CategoryNavButton>
      <CategoryNavButton
        count={
          coffeeLive ? undefined : presentBestieContextCount(coffeeCount)
        }
        icon={<Coffee className="size-4" />}
        onClick={() => onOpenKind("coffee")}
        pressed={activeKind === "coffee"}
        testId="bestie-rhs-coffee"
      >
        {coffeeLive ? (
          <span className="inline-flex min-w-0 items-center gap-1">
            <span className="truncate">Coffee</span>
            <span
              aria-label="Coffee brewing"
              className="shrink-0 tabular-nums opacity-80"
              data-testid="bestie-rhs-coffee-live"
            >
              {BESTIE_COFFEE_LIVE_LABEL}
            </span>
          </span>
        ) : (
          "Coffee"
        )}
      </CategoryNavButton>
      <CategoryNavButton
        count={
          summarizeLive ? undefined : presentBestieContextCount(threadCount)
        }
        icon={<MessagesSquare className="size-4" />}
        onClick={() => onOpenKind("thread")}
        pressed={activeKind === "thread"}
        testId="bestie-rhs-threads"
      >
        {summarizeLive ? (
          <span className="inline-flex min-w-0 items-center gap-1">
            <span className="truncate">Threads</span>
            <span
              aria-label="Thread summarize running"
              className="shrink-0 tabular-nums opacity-80"
              data-testid="bestie-rhs-threads-live"
            >
              {BESTIE_THREAD_SUMMARIZE_LIVE_LABEL}
            </span>
          </span>
        ) : (
          "Threads"
        )}
      </CategoryNavButton>
      <CategoryNavButton
        count={presentBestieContextCount(scratchCount)}
        icon={<StickyNote className="size-4" />}
        onClick={() => onOpenKind("scratch")}
        pressed={activeKind === "scratch"}
        testId="bestie-rhs-scratch"
      >
        Scratch
      </CategoryNavButton>
    </div>
  );
}

/**
 * Slide sheet for one Bestie category — lists items; header + reveals hand entry.
 */
export function BestieDmCategorySheet({
  adding,
  kind,
  onRequestAdd,
  scope,
}: {
  adding: boolean;
  kind: BestieListKind;
  onRequestAdd?: () => void;
  scope: BestieListScope;
}) {
  const state = useBestieList(scope);
  const items = state.items.filter((item) => item.kind === kind);
  const placeholder = kind === "reminder" ? "Add a reminder" : "Add a to-do";
  const addTestId =
    kind === "reminder" ? "bestie-add-reminder" : "bestie-add-todo";

  return (
    <div
      className="flex flex-col gap-3 py-1"
      data-testid={`bestie-dm-category-sheet-${kind}`}
    >
      {adding ? (
        <AddRow
          kind={kind}
          onAdd={(text, dueAt, repeat) =>
            addBestieListItemForScope(scope, { dueAt, kind, repeat, text })
          }
          placeholder={placeholder}
          testId={addTestId}
        />
      ) : (
        <p className="px-0.5 text-xs text-muted-foreground">
          Use + to add by hand (reminders can set a due time), or ask Assistant in
          natural language / with a{" "}
          <code className="text-2xs">bestie-list</code> fence.
          {onRequestAdd ? (
            <>
              {" "}
              <button
                className="underline-offset-2 hover:underline"
                onClick={onRequestAdd}
                type="button"
              >
                Add one
              </button>
            </>
          ) : null}
        </p>
      )}
      <div className="space-y-1.5">
        {items.length === 0 ? (
          <p className="px-0.5 text-xs text-muted-foreground">
            {kind === "reminder" ? "No reminders yet." : "No to-dos yet."}
          </p>
        ) : (
          items.map((item) => (
            <ListRow
              key={item.id}
              item={item}
              onComplete={() =>
                setBestieListItemStatusForScope(scope, item.id, "done")
              }
              onRemove={() => removeBestieListItemForScope(scope, item.id)}
              onReopen={() =>
                setBestieListItemStatusForScope(scope, item.id, "open")
              }
            />
          ))
        )}
      </div>
    </div>
  );
}
