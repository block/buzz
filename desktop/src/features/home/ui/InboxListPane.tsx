import { ChevronDown, Inbox } from "lucide-react";

import {
  formatInboxTypeLabel,
  type InboxFilter,
  type InboxItem,
} from "@/features/home/lib/inbox";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import { Markdown } from "@/shared/ui/markdown";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";
import { UserAvatar } from "@/shared/ui/UserAvatar";

const FILTER_OPTIONS: Array<{ label: string; value: InboxFilter }> = [
  { value: "all", label: "All" },
  { value: "mention", label: "Mentions" },
  { value: "needs_action", label: "Needs Action" },
  { value: "activity", label: "Activity" },
  { value: "agent_activity", label: "Agents" },
];

type InboxListPaneProps = {
  doneSet: ReadonlySet<string>;
  filter: InboxFilter;
  items: InboxItem[];
  onFilterChange: (filter: InboxFilter) => void;
  onSelect: (itemId: string) => void;
  selectedId: string | null;
};

export function InboxListPane({
  doneSet,
  filter,
  items,
  onFilterChange,
  onSelect,
  selectedId,
}: InboxListPaneProps) {
  const activeFilter = FILTER_OPTIONS.find((option) => option.value === filter);

  return (
    <section className="relative flex min-h-0 min-w-0 flex-col overflow-hidden bg-background/60">
      <div
        aria-hidden="true"
        className="pointer-events-none absolute inset-x-0 top-0 z-40 h-[76px] bg-background/75 backdrop-blur-md supports-[backdrop-filter]:bg-background/65 dark:bg-background/45 dark:backdrop-blur-xl dark:supports-[backdrop-filter]:bg-background/35"
      />
      <div className="absolute inset-x-0 top-[42px] z-50 min-h-[32px] px-5 py-[4px]">
        <div className="flex min-w-0 items-center justify-between gap-3">
          <div className="flex min-w-0 items-center gap-[6px]">
            <Inbox className="h-[14px] w-[14px] shrink-0 text-muted-foreground" />
            <h2 className="truncate text-sm font-semibold leading-5 tracking-tight">
              Inbox
            </h2>
          </div>
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <Button
                className="inline-flex h-6 shrink-0 items-center gap-1.5 rounded-full border-border/70 bg-background/70 px-2.5 text-[11px] font-medium leading-[1] text-muted-foreground shadow-xs backdrop-blur-sm hover:bg-muted/60 hover:text-foreground"
                size="sm"
                type="button"
                variant="outline"
              >
                <span>{activeFilter?.label ?? "All"}</span>
                <ChevronDown className="h-3 w-3" />
              </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end" className="min-w-[10rem]">
              <DropdownMenuRadioGroup
                onValueChange={(value) => onFilterChange(value as InboxFilter)}
                value={filter}
              >
                {FILTER_OPTIONS.map((option) => (
                  <DropdownMenuRadioItem
                    key={option.value}
                    value={option.value}
                  >
                    {option.label}
                  </DropdownMenuRadioItem>
                ))}
              </DropdownMenuRadioGroup>
            </DropdownMenuContent>
          </DropdownMenu>
        </div>
      </div>

      <div
        className="min-h-0 flex-1 overflow-y-auto overscroll-contain pt-[76px]"
        data-testid="home-inbox-list"
      >
        {items.length === 0 ? (
          <div className="flex h-full min-h-64 items-center justify-center px-6 text-center">
            <div>
              <p className="text-sm font-medium text-foreground">
                No messages found
              </p>
              <p className="mt-1 text-sm text-muted-foreground">
                Switch back to all mail to see more messages.
              </p>
            </div>
          </div>
        ) : (
          <div>
            {items.map((item) => {
              const isSelected = item.id === selectedId;
              const isDone = doneSet.has(item.id);
              const typeLabel = formatInboxTypeLabel(item);

              return (
                <button
                  className={cn(
                    "flex w-full items-start gap-2.5 border-l px-5 py-2 text-left transition-colors",
                    isSelected
                      ? "border-l-primary bg-muted/30"
                      : "border-l-transparent hover:bg-muted/25 active:bg-muted/40",
                  )}
                  data-testid={`home-inbox-item-${item.id}`}
                  key={item.id}
                  onClick={() => onSelect(item.id)}
                  type="button"
                >
                  <div className="relative">
                    <UserAvatar
                      avatarUrl={item.avatarUrl}
                      className="h-8 w-8 rounded-xl"
                      displayName={item.senderLabel}
                      size="md"
                    />
                    {!isDone ? (
                      <span className="absolute -right-1 -top-1 h-2.5 w-2.5 rounded-full border-2 border-background bg-primary" />
                    ) : null}
                  </div>

                  <div className="min-w-0 flex-1">
                    <div className="flex items-start gap-2">
                      <div className="min-w-0 flex-1">
                        <div className="flex items-center gap-2">
                          <p className="truncate text-sm font-semibold text-foreground">
                            {item.senderLabel}
                          </p>
                          {item.isActionRequired ? (
                            <span className="inline-flex shrink-0 items-center text-[10px] font-semibold uppercase tracking-[0.14em] text-amber-600 dark:text-amber-300">
                              Needs action
                            </span>
                          ) : null}
                        </div>
                      </div>
                      <span
                        className={cn(
                          "shrink-0 text-xs text-muted-foreground",
                          isDone ? "font-normal" : "font-semibold",
                        )}
                      >
                        {item.timestampLabel}
                      </span>
                    </div>

                    <div
                      className={cn(
                        "mt-0.5 line-clamp-2 text-sm leading-5 [&_*]:inline [&_a]:font-medium [&_a]:text-current [&_br]:hidden [&_p]:inline",
                        isDone
                          ? "font-normal text-muted-foreground"
                          : "font-semibold text-foreground",
                      )}
                    >
                      <Markdown
                        className="inline max-w-full text-inherit"
                        content={item.preview}
                        interactive={false}
                        mentionNames={item.mentionNames}
                        tight
                      />
                    </div>

                    <div className="mt-1 flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
                      <span
                        className={cn(
                          "text-[11px] text-muted-foreground",
                          isDone ? "font-normal" : "font-semibold",
                        )}
                      >
                        {typeLabel}
                      </span>
                    </div>
                  </div>
                </button>
              );
            })}
          </div>
        )}
      </div>
    </section>
  );
}
