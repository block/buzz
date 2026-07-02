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
  reverse = false,
  variant = "v8",
}: FuzzyLogoProps) {
  return (
    <BuzzLogoAnimation
      ariaLabel={ariaLabel}
      className={cn(!fuzz && "buzz-logo--pulse", className)}
      fullScreen={false}
      loop={loop}
      reverse={reverse}
      showBackground={false}
      textured={fuzz}
      variant={variant}
    />
  );
}
