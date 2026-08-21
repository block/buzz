import { Ellipsis } from "lucide-react";

import { FibreRow } from "@/features/home/ui/fibre/FibreRow";
import { fibreDotState } from "@/features/home/ui/fibre/fibreSeen";
import {
  FIBRE_LIST_TABS,
  FIBRE_TAB_LABELS,
  type FibreListTab,
  type FibreSort,
} from "@/features/home/ui/fibre/fibreSort";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import type { Fibre } from "@/features/triage/api";
import { TopChromeInsetHeader } from "@/shared/layout/TopChromeInsetHeader";
import { topChromeInset } from "@/shared/layout/chromeLayout";
import { cn } from "@/shared/lib/cn";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";

type FibreListPaneProps = {
  currentPubkey?: string;
  fibres: readonly Fibre[];
  listTab: FibreListTab;
  nowMs: number;
  tabCounts: Record<FibreListTab, number>;
  /** Every lane is empty, so the inbox-zero wallpaper owns the empty state. */
  isInboxZero: boolean;
  profiles?: UserProfileLookup;
  seenAtById?: Record<string, number>;
  selectedId: string | null;
  sort: FibreSort;
  onListTabChange: (tab: FibreListTab) => void;
  onSelect: (id: string) => void;
  onSortChange: (sort: FibreSort) => void;
};

const EMPTY_COPY: Record<FibreListTab, { title: string; body: string }> = {
  important: {
    title: "Nothing needs you",
    body: "Fibres blocking someone, or waiting on you, land here first.",
  },
  hot: {
    title: "No live discussions",
    body: "Busy threads show up here, even when nothing is asked of you.",
  },
  other: {
    title: "Nothing else waiting",
    body: "Fibres worth keeping but not urgent collect here.",
  },
  done: {
    title: "Nothing completed yet",
    body: "Mark a fibre done and it will land here.",
  },
};

const TAB_TRIGGER_CLASS =
  "inline-flex items-center rounded-none border-b-2 border-transparent bg-transparent px-0 py-1 text-sm font-medium text-muted-foreground transition-colors";

const TAB_COUNT_CLASS =
  "ml-1.5 inline-flex h-4 min-w-4 items-center justify-center rounded-full bg-muted px-1 text-2xs font-medium tabular-nums text-muted-foreground";

export function FibreListPane({
  currentPubkey,
  fibres,
  listTab,
  nowMs,
  tabCounts,
  isInboxZero,
  profiles,
  seenAtById,
  selectedId,
  sort,
  onListTabChange,
  onSelect,
  onSortChange,
}: FibreListPaneProps) {
  const showSeenDots = listTab !== "done";
  const empty = EMPTY_COPY[listTab];

  return (
    <div
      className={cn(
        "relative flex w-[27rem] shrink-0 flex-col overflow-hidden",
        topChromeInset.verticalDivider,
      )}
      data-testid="fibre-list"
    >
      <TopChromeInsetHeader
        className="flex h-[3.25rem] items-center gap-3 px-4"
        flush
      >
        <div
          className="flex items-center gap-3 text-muted-foreground"
          role="tablist"
        >
          {FIBRE_LIST_TABS.map((tab) => {
            const active = listTab === tab;
            return (
              <button
                aria-selected={active}
                className={cn(
                  TAB_TRIGGER_CLASS,
                  active && "border-foreground text-foreground",
                )}
                data-testid={`fibre-tab-${tab}`}
                key={tab}
                onClick={() => onListTabChange(tab)}
                role="tab"
                type="button"
              >
                {FIBRE_TAB_LABELS[tab]}
                <span
                  className={cn(
                    TAB_COUNT_CLASS,
                    active && "text-foreground/70",
                  )}
                  data-testid={`fibre-tab-${tab}-count`}
                >
                  {tabCounts[tab]}
                </span>
              </button>
            );
          })}
        </div>
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <button
              aria-label="Sort fibres"
              className="ml-auto flex h-8 w-8 items-center justify-center rounded-lg text-muted-foreground transition-colors hover:bg-muted/70 hover:text-foreground focus-visible:outline-hidden focus-visible:ring-1 focus-visible:ring-ring data-[state=open]:bg-muted/70 data-[state=open]:text-foreground"
              data-testid="fibre-sort-trigger"
              type="button"
            >
              <Ellipsis className="h-4 w-4" />
            </button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end" className="w-44">
            <DropdownMenuRadioGroup
              onValueChange={(value) => onSortChange(value as FibreSort)}
              value={sort}
            >
              <DropdownMenuRadioItem
                data-testid="fibre-sort-priority"
                value="priority"
              >
                Priority
              </DropdownMenuRadioItem>
              <DropdownMenuRadioItem
                data-testid="fibre-sort-newest"
                value="newest"
              >
                Newest
              </DropdownMenuRadioItem>
            </DropdownMenuRadioGroup>
          </DropdownMenuContent>
        </DropdownMenu>
      </TopChromeInsetHeader>
      <div className="min-h-0 flex-1 overflow-y-auto px-2 pb-2.5">
        {fibres.length === 0 ? (
          isInboxZero && listTab !== "done" ? null : (
            <div
              className="inbox-zero-copy px-8 py-14 text-center"
              data-testid="fibre-lane-empty"
            >
              <div className="mb-1.5 text-sm text-foreground/80">
                {empty.title}
              </div>
              <div className="text-xs leading-relaxed text-muted-foreground">
                {empty.body}
              </div>
            </div>
          )
        ) : (
          fibres.map((fibre) => {
            const seen = showSeenDots
              ? fibreDotState(fibre, seenAtById?.[fibre.id])
              : null;
            return (
              <FibreRow
                currentPubkey={currentPubkey}
                fibre={fibre}
                isSelected={fibre.id === selectedId}
                key={fibre.id}
                nowMs={nowMs}
                onSelect={onSelect}
                profiles={profiles}
                seen={seen}
              />
            );
          })
        )}
      </div>
    </div>
  );
}
