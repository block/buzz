import { Info } from "lucide-react";

import { ProfileAvatar } from "@/features/profile/ui/ProfileAvatar";
import type { AgentPersona } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import { promptPreview } from "@/shared/lib/promptPreview";
import { Badge } from "@/shared/ui/badge";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";

type PersonaIdentityProps = {
  className?: string;
  persona: AgentPersona;
  showBuiltInBadge?: boolean;
  showPromptTooltip?: boolean;
};

export function PersonaIdentity({
  className,
  persona,
  showBuiltInBadge = persona.isBuiltIn,
  showPromptTooltip = true,
}: PersonaIdentityProps) {
  const preview = promptPreview(persona.systemPrompt);

  return (
    <div className={cn("min-w-0 flex-1", className)}>
      <div className="flex min-w-0 items-center gap-2.5">
        <ProfileAvatar
          avatarUrl={persona.avatarUrl}
          className="h-8 w-8 rounded-lg text-xs"
          label={persona.displayName}
        />
        <div className="min-w-0">
          <div className="flex min-w-0 items-center gap-2">
            <p className="truncate text-sm font-semibold tracking-tight">
              {persona.displayName}
            </p>
            {showBuiltInBadge ? (
              <Badge variant="secondary">Built-in</Badge>
            ) : null}
            {showPromptTooltip && preview ? (
              <Tooltip>
                <TooltipTrigger asChild>
                  <button
                    aria-label="View system prompt"
                    className="flex h-4 w-4 shrink-0 items-center justify-center text-muted-foreground transition-colors hover:text-foreground"
                    type="button"
                  >
                    <Info className="h-4 w-4" />
                  </button>
                </TooltipTrigger>
                <TooltipContent side="bottom" className="max-w-xs">
                  <p>{preview}</p>
                </TooltipContent>
              </Tooltip>
            ) : null}
          </div>
        </div>
      </div>
    </div>
  );
}
