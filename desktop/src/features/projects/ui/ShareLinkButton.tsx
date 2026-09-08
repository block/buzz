import { Check, Link2 } from "lucide-react";
import * as React from "react";

import { cn } from "@/shared/lib/cn";
import { writeTextToClipboard } from "@/shared/lib/clipboard";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";

/**
 * Copies a `buzz://` share link for the surrounding entity. Renders nothing
 * when `link` is null — see `lib/projectShareLinks` for when an entity has no
 * shareable coordinate.
 */
export function ShareLinkButton({
  className,
  label = "Copy link",
  link,
  testId,
}: {
  className?: string;
  label?: string;
  link: string | null;
  testId?: string;
}) {
  const [copied, setCopied] = React.useState(false);
  const timeoutRef = React.useRef<ReturnType<typeof setTimeout> | null>(null);

  React.useEffect(
    () => () => {
      if (timeoutRef.current !== null) clearTimeout(timeoutRef.current);
    },
    [],
  );

  const handleCopy = React.useCallback(() => {
    if (!link) return;
    void writeTextToClipboard(link).then(() => {
      setCopied(true);
      if (timeoutRef.current !== null) clearTimeout(timeoutRef.current);
      timeoutRef.current = setTimeout(() => setCopied(false), 2_000);
    });
  }, [link]);

  if (!link) return null;

  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <button
          aria-label={label}
          className={cn(
            "flex h-7 w-7 shrink-0 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-muted/40 hover:text-foreground focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring",
            className,
          )}
          data-testid={testId}
          onClick={handleCopy}
          type="button"
        >
          {copied ? (
            <Check className="h-3.5 w-3.5 text-green-500" />
          ) : (
            <Link2 className="h-3.5 w-3.5" />
          )}
        </button>
      </TooltipTrigger>
      <TooltipContent>{copied ? "Link copied" : label}</TooltipContent>
    </Tooltip>
  );
}
