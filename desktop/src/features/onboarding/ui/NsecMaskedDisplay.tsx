import { Check, Copy, Eye, EyeOff } from "lucide-react";
import * as React from "react";
import { Button } from "@/shared/ui/button";

type NsecMaskedDisplayProps = {
  nsec: string;
  /** "bare" drops the boxed chrome for the onboarding spotlight treatment. */
  variant?: "boxed" | "bare";
};

/**
 * Masked nsec display with reveal toggle and copy button.
 *
 * Security invariants:
 * - nsec is not present in the DOM until the user clicks Reveal
 * - select is disabled while masked (user-select: none)
 * - state is cleared when the component unmounts
 */
export function NsecMaskedDisplay({
  nsec,
  variant = "boxed",
}: NsecMaskedDisplayProps) {
  const [isRevealed, setIsRevealed] = React.useState(false);
  const [isCopied, setIsCopied] = React.useState(false);
  const copyTimerRef = React.useRef<ReturnType<typeof setTimeout> | null>(null);

  React.useEffect(() => {
    return () => {
      setIsRevealed(false);
      if (copyTimerRef.current) {
        clearTimeout(copyTimerRef.current);
      }
    };
  }, []);

  function handleRevealToggle() {
    setIsRevealed((prev) => !prev);
  }

  async function handleCopy() {
    await navigator.clipboard.writeText(nsec);
    setIsCopied(true);
    if (copyTimerRef.current) clearTimeout(copyTimerRef.current);
    copyTimerRef.current = setTimeout(() => setIsCopied(false), 2000);
  }

  const isBare = variant === "bare";
  // Mask every character (no plaintext prefix leak), matching the real key's
  // length so toggling reveal never reflows the monospace text (no layout shift).
  // Bullets are joined with a zero-width space: WebKit (WKWebView) will not
  // line-break a run of U+2022 even with break-all/overflow-wrap, so without an
  // explicit break opportunity the masked key overflows its container. The mask
  // is decorative and non-selectable, and copy uses the raw nsec, so the
  // injected ZWSP is never surfaced.
  const maskedNsec = React.useMemo(
    () => Array.from(nsec, () => "•").join("\u200b"),
    [nsec],
  );
  const iconSize = isBare ? "h-6 w-6" : "h-4 w-4";

  return (
    <div
      className={
        isBare
          ? ""
          : "overflow-hidden rounded-lg border border-border/70 bg-muted/30"
      }
    >
      <div
        className={`flex min-w-0 items-center ${isBare ? "gap-4" : "gap-2 px-3 py-2"}`}
      >
        {/* Wrapping element is a block inside the flex item, not the flex item
            itself: WebKit (WKWebView) does not wrap a long unbroken string when
            the text node is the flex child directly, even with break-all /
            overflow-wrap. A plain block wraps reliably in every engine. */}
        <div className="min-w-0 flex-1">
          <p
            className={`w-full break-all [overflow-wrap:anywhere] font-mono ${
              isBare
                ? "text-nsec-key text-[var(--buzz-onboarding-backup-ink)]"
                : "text-xs leading-5"
            } ${
              isRevealed
                ? `select-text ${isBare ? "" : "text-foreground"}`
                : `select-none blur-[4px] ${isBare ? "" : "text-muted-foreground"}`
            }`}
            data-testid="nsec-value"
          >
            {isRevealed ? nsec : maskedNsec}
          </p>
        </div>
        <div className={`flex shrink-0 ${isBare ? "gap-1.5" : "gap-1"}`}>
          <Button
            aria-label={isRevealed ? "Hide private key" : "Reveal private key"}
            className={`${isBare ? "h-10 w-10" : "h-7 w-7"} text-muted-foreground hover:text-foreground`}
            data-testid="nsec-reveal-toggle"
            onClick={handleRevealToggle}
            size="icon"
            type="button"
            variant="ghost"
          >
            {isRevealed ? (
              <EyeOff className={iconSize} aria-hidden="true" />
            ) : (
              <Eye className={iconSize} aria-hidden="true" />
            )}
          </Button>
          <Button
            aria-label="Copy private key"
            className={`${isBare ? "h-10 w-10" : "h-7 w-7"} text-muted-foreground hover:text-foreground`}
            data-testid="nsec-copy"
            onClick={() => void handleCopy()}
            size="icon"
            type="button"
            variant="ghost"
          >
            {isCopied ? (
              <Check
                className={`${iconSize} text-primary`}
                aria-hidden="true"
              />
            ) : (
              <Copy className={iconSize} aria-hidden="true" />
            )}
          </Button>
        </div>
      </div>
    </div>
  );
}
