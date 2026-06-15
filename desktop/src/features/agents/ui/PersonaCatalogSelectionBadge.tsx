import { Check } from "lucide-react";

import { cn } from "@/shared/lib/cn";

import { personaCatalogCopy } from "./personaLibraryCopy";

type PersonaCatalogSelectionBadgeProps = {
  isActive: boolean;
};

export function PersonaCatalogSelectionBadge({
  isActive,
}: PersonaCatalogSelectionBadgeProps) {
  return (
    <span
      className={cn(
        "inline-flex items-center gap-1 whitespace-nowrap rounded-full px-2.5 py-1 text-[10px] font-semibold uppercase tracking-[0.14em]",
        isActive
          ? "bg-primary text-primary-foreground shadow-xs"
          : "border border-border/70 bg-background/85 text-muted-foreground",
      )}
    >
      {isActive ? <Check className="h-4 w-4" /> : null}
      {isActive
        ? personaCatalogCopy.selectedState
        : personaCatalogCopy.availableState}
    </span>
  );
}
