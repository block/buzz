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

  return (
    <div
      className={
        variant === "boxed"
          ? "overflow-hidden rounded-lg border border-border/70 bg-muted/30"
          : ""
      }
    >
      <div className="flex items-center gap-2 px-3 py-2">
        <p
          className={`min-w-0 flex-1 break-all font-mono leading-5 ${
            variant === "bare" ? "text-base" : "text-xs"
          } ${
            isRevealed
              ? "select-text text-foreground"
              : "select-none text-muted-foreground blur-[4px]"
          }`}
          data-testid="nsec-value"
        >
          {isRevealed ? nsec : nsec.slice(0, 6) + "•".repeat(52)}
        </p>
        <div className="flex shrink-0 gap-1">
          <Button
            aria-label={isRevealed ? "Hide private key" : "Reveal private key"}
            className="h-7 w-7 text-muted-foreground hover:text-foreground"
            data-testid="nsec-reveal-toggle"
            onClick={handleRevealToggle}
            size="icon"
            type="button"
            variant="ghost"
          >
            {isRevealed ? (
              <EyeOff className="h-4 w-4" aria-hidden="true" />
            ) : (
              <Eye className="h-4 w-4" aria-hidden="true" />
            )}
          </Button>
          <Button
            aria-label="Copy private key"
            className="h-7 w-7 text-muted-foreground hover:text-foreground"
            data-testid="nsec-copy"
            disabled={!isRevealed}
            onClick={() => void handleCopy()}
            size="icon"
            type="button"
            variant="ghost"
          >
            {isCopied ? (
              <Check className="h-4 w-4 text-primary" aria-hidden="true" />
            ) : (
              <Copy className="h-4 w-4" aria-hidden="true" />
            )}
          </Button>
        </div>
      </div>
    </div>
  );
}
