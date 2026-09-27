import * as React from "react";

import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { cn } from "@/shared/lib/cn";
import { Popover, PopoverContent, PopoverTrigger } from "@/shared/ui/popover";
import { BestiePopover, BestieTriggerVisual } from "./BestiePopover";
import { useBestie } from "./useBestie";

/**
 * Anchored Bestie trigger for the sidebar profile footer.
 * Shows agent avatar + name here; left-nav label stays "Bestie".
 * Opens the Bestie popover chat. Does not navigate away.
 */
export function BestieProfileTrigger({ className }: { className?: string }) {
  const bestie = useBestie();
  const { goAgents } = useAppNavigation();
  const [open, setOpen] = React.useState(false);
  const agent = bestie.assignedAgent;

  return (
    <Popover
      onOpenChange={(nextOpen) => {
        if (nextOpen && !agent) {
          void goAgents();
          setOpen(false);
          return;
        }
        setOpen(nextOpen);
      }}
      open={open}
    >
      <PopoverTrigger asChild>
        <button
          aria-label={
            agent ? `Open Bestie chat with ${agent.name}` : "Choose a Bestie"
          }
          className={cn(
            "relative flex max-w-[42%] shrink-0 items-center gap-1.5 rounded-full outline-hidden focus-visible:ring-2 focus-visible:ring-ring",
            className,
          )}
          data-testid="bestie-profile-trigger"
          onClick={(event) => event.stopPropagation()}
          type="button"
        >
          <BestieTriggerVisual
            agent={agent}
            className="h-8 w-8"
            compact
            imageDraggable={false}
          />
          {agent ? (
            <span
              className="min-w-0 truncate text-xs font-medium text-sidebar-foreground"
              data-testid="bestie-profile-agent-name"
            >
              {agent.name}
            </span>
          ) : null}
        </button>
      </PopoverTrigger>
      {agent ? (
        <PopoverContent
          align="end"
          className="w-80"
          onClick={(event) => event.stopPropagation()}
          onOpenAutoFocus={(event) => {
            const target = event.currentTarget.querySelector(
              "[data-testid='bestie-composer']",
            );
            if (target instanceof HTMLElement) {
              event.preventDefault();
              target.focus();
            }
          }}
          side="top"
          sideOffset={10}
        >
          <BestiePopover onRequestClose={() => setOpen(false)} />
        </PopoverContent>
      ) : null}
    </Popover>
  );
}
