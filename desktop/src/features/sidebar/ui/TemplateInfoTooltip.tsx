import { Info } from "lucide-react";

import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";

const TEMPLATE_TOOLTIP_COPY = {
  channel:
    "A template is a reusable channel configuration. Choose one to apply its settings to this channel.",
  section:
    "A template is a reusable channel configuration. New channels in this section automatically use the selected template.",
} as const;

export function TemplateInfoTooltip({
  context,
}: {
  context: keyof typeof TEMPLATE_TOOLTIP_COPY;
}) {
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <button
          aria-label={`About ${context} templates`}
          className="inline-flex size-6 shrink-0 items-center justify-center rounded-full text-muted-foreground/70 transition-colors hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          data-testid={`${context}-template-info`}
          type="button"
        >
          <Info aria-hidden className="size-4" />
        </button>
      </TooltipTrigger>
      <TooltipContent className="max-w-72 text-left" side="top">
        <p>{TEMPLATE_TOOLTIP_COPY[context]}</p>
      </TooltipContent>
    </Tooltip>
  );
}
