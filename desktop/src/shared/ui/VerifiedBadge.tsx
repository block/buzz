import { BadgeCheck } from "lucide-react";

import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";

export function VerifiedBadge({ verifiedName }: { verifiedName: string }) {
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <span
          aria-label={`Verified corporate identity: ${verifiedName}`}
          className="inline-flex shrink-0 items-center text-blue-500"
          data-testid="verified-corporate-identity"
        >
          <BadgeCheck aria-hidden="true" className="h-4 w-4" fill="currentColor" />
        </span>
      </TooltipTrigger>
      <TooltipContent side="top">
        <p className="text-xs">Verified as {verifiedName}</p>
      </TooltipContent>
    </Tooltip>
  );
}
