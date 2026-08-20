import { Ellipsis } from "lucide-react";

import type { Fibre } from "@/features/triage/api";
import { FibreRow } from "@/features/home/ui/fibre/FibreRow";
import { TopChromeInsetHeader } from "@/shared/layout/TopChromeInsetHeader";
import { cn } from "@/shared/lib/cn";
import { topChromeInset } from "@/shared/layout/chromeLayout";

type FibreListPaneProps = {
  clearedCount: number;
  fibres: readonly Fibre[];
  nowMs: number;
  selectedId: string | null;
  onSelect: (id: string) => void;
};

export function FibreListPane({
  clearedCount,
  fibres,
  nowMs,
  selectedId,
  onSelect,
}: FibreListPaneProps) {
  return (
    <div
      className={cn(
        "relative flex w-[27rem] shrink-0 flex-col overflow-hidden",
        topChromeInset.verticalDivider,
      )}
      data-testid="fibre-list"
    >
      <TopChromeInsetHeader
        className="flex h-[3.25rem] items-center gap-2.5 px-4"
        flush
      >
        <div className="flex items-center gap-1.5 text-base font-medium tracking-tight">
          Fibres
        </div>
        <div className="text-xs text-muted-foreground">
          {fibres.length} open · {clearedCount} cleared
        </div>
        <div className="ml-auto flex h-8 w-8 items-center justify-center text-muted-foreground">
          <Ellipsis className="h-4 w-4" />
        </div>
      </TopChromeInsetHeader>
      <div className="min-h-0 flex-1 overflow-y-auto px-2 pb-2.5">
        {fibres.length === 0 ? (
          <div className="px-8 py-14 text-center">
            <div className="mb-1.5 text-sm text-foreground/80">Fibre Zero</div>
            <div className="text-xs leading-relaxed text-muted-foreground">
              Nothing left to triage.
            </div>
          </div>
        ) : (
          fibres.map((fibre) => (
            <FibreRow
              fibre={fibre}
              isSelected={fibre.id === selectedId}
              key={fibre.id}
              nowMs={nowMs}
              onSelect={onSelect}
            />
          ))
        )}
      </div>
    </div>
  );
}
