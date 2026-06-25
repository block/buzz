import * as React from "react";

import { Button, type ButtonProps } from "@/shared/ui/button";
import { cn } from "@/shared/lib/cn";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";

/**
 * An icon button for the chat composer toolbar, with its label tooltip baked
 * in.
 *
 * The composer's icon buttons surface short, label-only tooltips that float
 * over the message textarea. Radix renders TooltipContent as a hoverable
 * Portal popup, so by default it intercepts the mouse and you can't click into
 * the field underneath while a tooltip is visible.
 *
 * ComposerIconButton owns the whole `Tooltip → TooltipTrigger → Button →
 * TooltipContent` shape and applies `pointer-events-none` to the popup, so the
 * tooltip is click-through on this surface only — we don't make an app-wide
 * promise about every tooltip. Because the button itself owns its tooltip,
 * every current and future composer icon button gets the click-through
 * behavior without re-deriving the override at each call site.
 *
 * `pointer-events-none` touches the floating popup exclusively; the trigger
 * keeps its pointer/focus behavior, so focus-to-show (screen-magnification
 * accommodation) and the hover/show lifecycle are untouched (WCAG
 * content-on-hover-or-focus). It only fits short, non-interactive labels —
 * keep interactive content out of the tooltip.
 *
 * `disableHoverableContent` turns off Radix's "safe bridge" — the keep-alive
 * window that normally lets the cursor slide off the trigger onto the popup
 * and persist it. Without it these label tooltips would camp open (and even be
 * text-selectable) while the pointer hovers the popup; with it they dismiss the
 * instant the cursor leaves the trigger. Scoped to this composer Root only —
 * the shared TooltipProvider keeps its app-wide default.
 */
export interface ComposerIconButtonProps extends ButtonProps {
  /** Short, non-interactive label shown in the click-through tooltip. */
  tooltip: React.ReactNode;
  /** Optional className for the tooltip popup (e.g. to tune placement). */
  tooltipClassName?: string;
}

const ComposerIconButton = React.forwardRef<
  HTMLButtonElement,
  ComposerIconButtonProps
>(
  (
    { tooltip, tooltipClassName, size = "icon", type = "button", ...props },
    ref,
  ) => (
    <Tooltip disableHoverableContent>
      <TooltipTrigger asChild>
        <Button ref={ref} size={size} type={type} {...props} />
      </TooltipTrigger>
      <TooltipContent className={cn("pointer-events-none", tooltipClassName)}>
        {tooltip}
      </TooltipContent>
    </Tooltip>
  ),
);
ComposerIconButton.displayName = "ComposerIconButton";

export { ComposerIconButton };
