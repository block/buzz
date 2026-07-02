import { cn } from "@/shared/lib/cn";
import { FuzzyLogo } from "@/shared/ui/buzz-logo/FuzzyLogo";

export function TurnLivenessIndicator({
  className,
  fuzz = false,
}: {
  className?: string;
  /** Defaults to false — the indicator stays mounted for whole turns. */
  fuzz?: boolean;
}) {
  return (
    <div
      aria-label="Agent turn in progress"
      className={cn("opacity-25", className)}
      data-testid="turn-liveness-indicator"
      role="status"
    >
      <FuzzyLogo
        ariaLabel="Agent turn in progress"
        className="text-foreground"
        fuzz={fuzz}
        loop
        loopRestSeconds={2}
      />
    </div>
  );
}
