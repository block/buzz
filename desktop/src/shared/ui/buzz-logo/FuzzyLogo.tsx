import { cn } from "@/shared/lib/cn";
import BuzzLogoAnimation, {
  type BuzzLogoAnimationProps,
} from "./BuzzLogoAnimation";

export type FuzzyLogoProps = {
  /** When false, skips the looping feTurbulence texture filter and uses a CSS pulse instead. */
  fuzz?: boolean;
  className?: string;
  ariaLabel?: string;
  loop?: boolean;
  /** When looping, hide the mark for this many seconds between plays. */
  loopRestSeconds?: number;
  /** Set false when a parent drives its own opacity animation over the mark. */
  pulse?: boolean;
  reverse?: boolean;
  variant?: BuzzLogoAnimationProps["variant"];
};

/**
 * The fuzzy Buzz mark. v8 ships a built-in animated texture (looping fractal-noise
 * turbulence + grain) applied via an SVG filter. Set `fuzz={false}` to render the
 * crisp geometry with a lightweight CSS pulse — recommended for long-lived mounts.
 */
export function FuzzyLogo({
  fuzz = true,
  className,
  ariaLabel = "Buzz logo",
  loop = false,
  loopRestSeconds = 0,
  pulse = true,
  reverse = false,
  variant = "v8",
}: FuzzyLogoProps) {
  // The rest-window loop already reads as "alive"; skip the pulse so the two
  // opacity animations don't fight.
  const hasRestWindow = loop && loopRestSeconds > 0;

  return (
    <BuzzLogoAnimation
      ariaLabel={ariaLabel}
      className={cn(
        pulse && !fuzz && !hasRestWindow && "buzz-logo--pulse",
        className,
      )}
      fullScreen={false}
      loop={loop}
      loopRestSeconds={loopRestSeconds}
      reverse={reverse}
      showBackground={false}
      textured={fuzz}
      variant={variant}
    />
  );
}
