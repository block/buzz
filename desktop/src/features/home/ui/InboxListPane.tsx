import type { InboxFilter, InboxItem } from "@/features/home/lib/inbox";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
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
  return (
    <section className="flex min-h-0 min-w-0 flex-col overflow-hidden bg-background/60">
      <div className="px-5 pb-3 pt-14">
        <div className="flex flex-nowrap gap-1">
          {FILTER_OPTIONS.map((option) => (
            <Button
              className="h-7 rounded-full border border-transparent px-1.5 text-[10.5px] font-medium text-muted-foreground data-[active=true]:border-border/70 data-[active=true]:bg-background/80 data-[active=true]:text-foreground data-[active=true]:shadow-sm data-[active=true]:backdrop-blur-sm"
              data-active={filter === option.value}
              key={option.value}
              onClick={() => onFilterChange(option.value)}
              size="sm"
              type="button"
              variant="ghost"
            >
              {option.label}
            </Button>
          ))}
        </div>
      </div>

      <div
        className="min-h-0 flex-1 overflow-y-auto overscroll-contain"
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

                    <p
                      className={cn(
                        "mt-0.5 line-clamp-2 text-sm leading-5",
                        isDone
                          ? "font-normal text-muted-foreground"
                          : "font-semibold text-foreground",
                      )}
                    >
                      {item.preview}
                    </p>

                    <div className="mt-1 flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
                      {item.channelLabel ? (
                        <span
                          className={cn(
                            "text-[11px] text-muted-foreground",
                            isDone ? "font-normal" : "font-semibold",
                          )}
                        >
                          #{item.channelLabel}
                        </span>
                      ) : null}
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
