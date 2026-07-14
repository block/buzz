import * as React from "react";

import { isMaskedLink } from "@/shared/lib/maskedLink";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";
import { SpoilerHiddenContext } from "./SpoilerInline";

/**
 * Masked link (`[text](url)` where the text hides the destination): reveal
 * the full URL on hover so readers can see where a "click here" actually
 * goes before clicking. Bare/autolinked URLs render children unchanged —
 * their label already is the destination.
 */
export function MaskedLinkTooltip({
  href,
  label,
  disabled = false,
  children,
}: {
  href: string | undefined;
  label: string;
  disabled?: boolean;
  children: React.ReactElement;
}) {
  // A masked link inside an unrevealed spoiler must not expose its URL via a
  // hover/focus tooltip — that would leak the spoiler's content early.
  const hiddenInSpoiler = React.useContext(SpoilerHiddenContext);
  if (disabled || hiddenInSpoiler || !href || !isMaskedLink(label, href)) {
    return children;
  }
  return (
    <Tooltip>
      <TooltipTrigger asChild>{children}</TooltipTrigger>
      {/* pointer-events-none: the tooltip is purely informational and must
          never intercept clicks meant for the link (or a spoiler wrapping
          it) when it opens mid-interaction.
          overflow-wrap:anywhere (not break-all): most URLs fit on one line
          at max-w-md; only genuinely long ones wrap, breaking at natural
          boundaries rather than mid-word. Never truncate — the full
          destination must stay auditable for the anti-phishing use case. */}
      <TooltipContent className="pointer-events-none max-w-md [overflow-wrap:anywhere]">
        {href}
      </TooltipContent>
    </Tooltip>
  );
}
