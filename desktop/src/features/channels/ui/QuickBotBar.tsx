import * as React from "react";
import { Spinner } from "@/shared/ui/spinner";

import type { AgentPersona } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import { rewriteRelayUrl } from "@/shared/lib/mediaUrl";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";

type QuickBotBarProps = {
  personas: Array<{
    persona: AgentPersona;
    instanceName: string;
  }>;
  pending: boolean;
  onAdd: (persona: AgentPersona, instanceName: string) => void;
};

export function QuickBotBar({ personas, pending, onAdd }: QuickBotBarProps) {
  const [pendingId, setPendingId] = React.useState<string | null>(null);

  // Clear pending state when the mutation finishes
  React.useEffect(() => {
    if (!pending && pendingId) {
      setPendingId(null);
    }
  }, [pending, pendingId]);

  if (personas.length === 0) return null;

  return (
    <div
      className="flex items-center"
      role="toolbar"
      aria-label="Quick add bots"
    >
      <div
        className={cn(
          "flex items-center gap-1 overflow-hidden transition-all duration-200 ease-out",
          "max-w-0 opacity-0 group-hover/quick:mr-1 group-hover/quick:max-w-[120px] group-hover/quick:opacity-100",
        )}
      >
        {personas.slice(0, 3).map(({ persona, instanceName }) => {
          const initials = persona.displayName
            .split(" ")
            .map((p) => p[0])
            .join("")
            .slice(0, 2)
            .toUpperCase();
          const isThisPending = pendingId === persona.id;

          return (
            <Tooltip key={persona.id}>
              <TooltipTrigger asChild>
                <button
                  aria-label={`Add ${persona.displayName}`}
                  className={cn(
                    "relative flex h-7 w-7 shrink-0 items-center justify-center rounded-lg",
                    "border border-border/50 shadow-xs",
                    "transition-transform duration-150 hover:scale-110",
                    "focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring",
                    isThisPending && "pointer-events-none opacity-60",
                  )}
                  disabled={pending}
                  onClick={() => {
                    setPendingId(persona.id);
                    onAdd(persona, instanceName);
                  }}
                  type="button"
                >
                  {persona.avatarUrl ? (
                    <img
                      alt={persona.displayName}
                      className="h-full w-full rounded-lg object-cover"
                      referrerPolicy="no-referrer"
                      src={rewriteRelayUrl(persona.avatarUrl)}
                    />
                  ) : (
                    <span className="text-2xs font-semibold text-primary">
                      {initials}
                    </span>
                  )}
                  {isThisPending ? (
                    <div className="absolute inset-0 flex items-center justify-center rounded-lg bg-background/70">
                      <Spinner className="h-4 w-4 border-2 text-primary" />
                    </div>
                  ) : null}
                </button>
              </TooltipTrigger>
              <TooltipContent side="bottom" className="text-xs">
                Add {instanceName} ({persona.displayName})
              </TooltipContent>
            </Tooltip>
          );
        })}
      </div>
    </div>
  );
}
