import { Bell, Check, ListTodo, Plus, Trash2 } from "lucide-react";
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
import type {
  BestieListItem,
  BestieListKind,
  BestieListScope,
} from "./bestieListTypes";
import {
  dueAtFromDatetimeLocal,
  presentBestieContextCount,
} from "./bestieDmRhsHelpers";

export {
  bestieCategoryTitle,
  datetimeLocalFromDueAt,
  dueAtFromDatetimeLocal,
  presentBestieContextCount,
} from "./bestieDmRhsHelpers";

/** Match project home context row chrome (icon + label + count). */
const BESTIE_RHS_ROW_CLASS =
  "h-8 w-full justify-start gap-2 rounded-md px-2 py-1.5 text-left text-sm font-normal text-sidebar-foreground/80 transition-[background-color,color] hover:bg-sidebar-accent hover:text-sidebar-accent-foreground disabled:pointer-events-none disabled:opacity-50";

function CategoryRowContent({
  children,
  count,
  icon,
}: {
  children: React.ReactNode;
  count?: number;
  icon: React.ReactNode;
}) {
  return (
    <>
      <span className="flex size-4 shrink-0 items-center justify-center text-current">
        {icon}
      </span>
      <span className="min-w-0 flex-1 truncate text-left">{children}</span>
      <span className="w-8 shrink-0 text-right tabular-nums text-current opacity-60">
        {count ?? ""}
      </span>
    </>
  );
}

function CategoryNavButton({
  children,
  count,
  icon,
  onClick,
  pressed,
  testId,
}: {
  children: React.ReactNode;
  count?: number;
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
      <CategoryRowContent count={count} icon={icon}>
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
        <p
          className={cn(
            "text-sm leading-snug",
            done && "line-through text-muted-foreground",
          )}
        >
          {item.text}
        </p>
        {item.kind === "reminder" && item.dueAt != null ? (
          <p className="mt-0.5 text-2xs text-muted-foreground">
            Due {new Date(item.dueAt * 1000).toLocaleString()}
          </p>
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
  onAdd: (text: string, dueAt: number | null) => void;
  placeholder: string;
  testId: string;
}) {
  const [text, setText] = React.useState("");
  const [dueLocal, setDueLocal] = React.useState("");
  const inputRef = React.useRef<HTMLInputElement>(null);
  React.useEffect(() => {
    inputRef.current?.focus();
  }, []);
  const submit = () => {
    const trimmed = text.trim();
    if (!trimmed) return;
    onAdd(trimmed, kind === "reminder" ? dueAtFromDatetimeLocal(dueLocal) : null);
    setText("");
    setDueLocal("");
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
  onOpenKind,
  scope,
}: {
  activeKind?: BestieListKind | null;
  onOpenKind: (kind: BestieListKind) => void;
  scope: BestieListScope;
}) {
  const state = useBestieList(scope);
  const openReminders = state.items.filter(
    (item) => item.kind === "reminder" && item.status === "open",
  ).length;
  const openTodos = state.items.filter(
    (item) => item.kind === "todo" && item.status === "open",
  ).length;

  return (
    <div className="space-y-1 px-2 pb-8 pt-3" data-testid="bestie-dm-rhs-panel">
      <CategoryNavButton
        count={presentBestieContextCount(openReminders)}
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
          onAdd={(text, dueAt) =>
            addBestieListItemForScope(scope, { dueAt, kind, text })
          }
          placeholder={placeholder}
          testId={addTestId}
        />
      ) : (
        <p className="px-0.5 text-xs text-muted-foreground">
          Use + to add by hand (reminders can set a due time), or ask Bestie
          in natural language / with a{" "}
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
