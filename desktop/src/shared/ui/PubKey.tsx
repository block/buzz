import { Check, Copy } from "lucide-react";
import * as React from "react";

import { copyTextToClipboard } from "@/shared/lib/clipboard";
import { cn } from "@/shared/lib/cn";
import { safeNpub } from "@/shared/lib/nostrUtils";
import { truncatePubkey } from "@/shared/lib/pubkey";
import { Button } from "@/shared/ui/button";
import { Popover, PopoverContent, PopoverTrigger } from "@/shared/ui/popover";

const HOVER_OPEN_DELAY_MS = 500;
const HOVER_CLOSE_DELAY_MS = 200;

type PubKeyProps = {
  /** 64-char hex pubkey. */
  pubkey: string;
  /**
   * `compact` — truncated hex, click/tap opens a popover with the full npub,
   * full hex, and copy buttons. The default for identity display in lists,
   * cards, and metadata rows.
   *
   * `full` — the complete npub rendered inline with copy buttons. Required on
   * security-decision surfaces (invite/approve, removal, trust/pairing, new
   * DM, key import): a truncated key is forgeable by vanity grinding, so
   * decisions must be made against the whole key.
   */
  variant?: "compact" | "full";
  className?: string;
  testId?: string;
};

function CopyRow({ label, value }: { label: string; value: string }) {
  const [copied, setCopied] = React.useState(false);
  const resetTimer = React.useRef<number | undefined>(undefined);
  React.useEffect(() => () => window.clearTimeout(resetTimer.current), []);

  return (
    <div className="flex min-w-0 items-start gap-1.5">
      <div className="min-w-0 flex-1">
        <div className="text-2xs font-medium text-muted-foreground">
          {label}
        </div>
        <div className="break-all font-mono text-xs">{value}</div>
      </div>
      <Button
        aria-label={`Copy ${label}`}
        onClick={() => {
          copyTextToClipboard(value, `${label} copied`);
          setCopied(true);
          window.clearTimeout(resetTimer.current);
          resetTimer.current = window.setTimeout(() => setCopied(false), 1500);
        }}
        size="icon-xs"
        type="button"
        variant="ghost"
      >
        {copied ? <Check /> : <Copy />}
      </Button>
    </div>
  );
}

function PubKeyDetails({ pubkey }: { pubkey: string }) {
  const npub = safeNpub(pubkey);
  return (
    <div className="space-y-2">
      {npub ? <CopyRow label="npub" value={npub} /> : null}
      <CopyRow label="hex" value={pubkey} />
    </div>
  );
}

/**
 * Canonical pubkey display. See the `variant` prop for when each form is
 * appropriate; never render a hand-truncated pubkey outside this component.
 */
export function PubKey({
  pubkey,
  variant = "compact",
  className,
  testId,
}: PubKeyProps) {
  const [open, setOpen] = React.useState(false);
  const hoverTimerRef = React.useRef<ReturnType<typeof setTimeout> | null>(
    null,
  );

  const clearHoverTimer = React.useCallback(() => {
    if (hoverTimerRef.current !== null) {
      clearTimeout(hoverTimerRef.current);
      hoverTimerRef.current = null;
    }
  }, []);

  const handleTriggerMouseEnter = React.useCallback(() => {
    clearHoverTimer();
    hoverTimerRef.current = setTimeout(() => {
      setOpen(true);
    }, HOVER_OPEN_DELAY_MS);
  }, [clearHoverTimer]);

  const handleMouseLeave = React.useCallback(() => {
    clearHoverTimer();
    hoverTimerRef.current = setTimeout(() => {
      setOpen(false);
    }, HOVER_CLOSE_DELAY_MS);
  }, [clearHoverTimer]);

  const handleContentMouseEnter = React.useCallback(() => {
    clearHoverTimer();
  }, [clearHoverTimer]);

  React.useEffect(() => clearHoverTimer, [clearHoverTimer]);

  if (variant === "full") {
    const npub = safeNpub(pubkey);
    return (
      <span
        className={cn("inline-flex min-w-0 items-center gap-1", className)}
        data-testid={testId}
      >
        <span className="break-all font-mono text-xs">{npub ?? pubkey}</span>
        <Popover>
          <PopoverTrigger asChild>
            <Button
              aria-label="Copy public key"
              size="icon-xs"
              type="button"
              variant="ghost"
            >
              <Copy />
            </Button>
          </PopoverTrigger>
          <PopoverContent align="start" className="w-96 max-w-[90vw]">
            <PubKeyDetails pubkey={pubkey} />
          </PopoverContent>
        </Popover>
      </span>
    );
  }

  return (
    <Popover onOpenChange={setOpen} open={open}>
      <PopoverTrigger asChild>
        <button
          aria-label="Show full public key"
          className={cn(
            "cursor-pointer rounded font-mono hover:underline focus-visible:outline-hidden focus-visible:ring-1 focus-visible:ring-ring",
            className,
          )}
          data-testid={testId}
          onMouseEnter={handleTriggerMouseEnter}
          onMouseLeave={handleMouseLeave}
          type="button"
        >
          {truncatePubkey(pubkey)}
        </button>
      </PopoverTrigger>
      <PopoverContent
        align="start"
        className="w-96 max-w-[90vw]"
        onMouseEnter={handleContentMouseEnter}
        onMouseLeave={handleMouseLeave}
        onOpenAutoFocus={(event) => event.preventDefault()}
      >
        <PubKeyDetails pubkey={pubkey} />
      </PopoverContent>
    </Popover>
  );
}
