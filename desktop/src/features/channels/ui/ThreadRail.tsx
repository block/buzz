import { ChevronLeft, ChevronRight, MessageSquareText, X } from "lucide-react";

import {
  threadRailColumnClassName,
  threadRailEntryClassName,
  threadRailHeaderClassName,
  threadRailShellClassName,
} from "@/features/channels/threadRailLayout";
import {
  isThreadRailPinActive,
  threadRailPinToChannelNavigation,
} from "@/features/channels/threadRailNavigation";
import type { ThreadRailPin } from "@/features/channels/threadRailStorage";
import { Button } from "@/shared/ui/button";
import { cn } from "@/shared/lib/cn";

export function ThreadRail({
  collapsed,
  onNavigate,
  onToggleCollapsed,
  onUnpin,
  openThreadRootId,
  pins,
  selectedChannelId,
  unreadRootIds,
}: {
  collapsed: boolean;
  onNavigate: (
    destination: ReturnType<typeof threadRailPinToChannelNavigation>,
  ) => void;
  onToggleCollapsed: () => void;
  onUnpin: (pin: Pick<ThreadRailPin, "channelId" | "rootId">) => void;
  openThreadRootId: string | null;
  pins: ThreadRailPin[];
  selectedChannelId: string | null;
  unreadRootIds: ReadonlySet<string>;
}) {
  if (pins.length === 0) return null;
  return (
    <div
      className={threadRailColumnClassName()}
      data-testid="thread-rail-column"
    >
      <aside
        aria-label="Pinned threads"
        className={cn(
          threadRailShellClassName(collapsed),
          collapsed ? "min-[601px]:w-10" : "min-[601px]:w-56",
        )}
        data-collapsed={collapsed}
        data-testid="thread-rail"
      >
        <div
          className={cn(
            threadRailHeaderClassName(),
            collapsed ? "justify-center px-1" : "justify-between",
          )}
          data-testid="thread-rail-header"
        >
          {!collapsed ? (
            <span className="truncate text-base font-semibold leading-6 tracking-tight">
              Pinned threads
            </span>
          ) : null}
          <Button
            aria-expanded={!collapsed}
            aria-label={
              collapsed
                ? `Expand ${pins.length} pinned threads`
                : "Collapse pinned threads"
            }
            data-testid="thread-rail-toggle"
            onClick={onToggleCollapsed}
            size="icon"
            type="button"
            variant="ghost"
          >
            {collapsed ? (
              <ChevronLeft aria-hidden />
            ) : (
              <ChevronRight aria-hidden />
            )}
          </Button>
        </div>
        {!collapsed ? (
          <div className="min-h-0 space-y-1 overflow-y-auto px-2 pb-2 pt-1">
            {pins.map((pin) => {
              const active = isThreadRailPinActive(
                pin,
                selectedChannelId,
                openThreadRootId,
              );
              const hasUnread = unreadRootIds.has(pin.rootId);
              const label = `${pin.channelName ? `#${pin.channelName}: ` : ""}${
                pin.rootExcerpt || "Pinned thread"
              }`;
              return (
                <div
                  className={threadRailEntryClassName(active)}
                  data-testid={`thread-rail-row-${pin.rootId}`}
                  key={`${pin.channelId}:${pin.rootId}`}
                >
                  <button
                    aria-current={active ? "page" : undefined}
                    aria-label={hasUnread ? `${label}, unread reply` : label}
                    className="min-w-0 flex-1 px-2.5 py-2 text-left text-sm focus-visible:outline-hidden"
                    data-testid={`thread-rail-entry-${pin.rootId}`}
                    onClick={() =>
                      onNavigate(threadRailPinToChannelNavigation(pin))
                    }
                    title={label}
                    type="button"
                  >
                    <span className="flex items-center gap-2">
                      <MessageSquareText
                        aria-hidden
                        className="size-3.5 shrink-0 text-muted-foreground"
                      />
                      <span className="min-w-0 flex-1 truncate">{label}</span>
                      {hasUnread ? (
                        <span
                          aria-hidden
                          className="size-2 shrink-0 rounded-full bg-primary"
                          data-testid={`thread-rail-unread-${pin.rootId}`}
                        />
                      ) : null}
                    </span>
                  </button>
                  <Button
                    aria-label={`Unpin ${label}`}
                    className="mr-1 shrink-0 opacity-0 group-hover:opacity-100 focus-visible:opacity-100"
                    data-testid={`unpin-thread-rail-${pin.rootId}`}
                    onClick={(event) => {
                      event.stopPropagation();
                      onUnpin(pin);
                    }}
                    size="icon-xs"
                    title="Unpin thread"
                    type="button"
                    variant="ghost"
                  >
                    <X aria-hidden />
                  </Button>
                </div>
              );
            })}
          </div>
        ) : null}
      </aside>
    </div>
  );
}
