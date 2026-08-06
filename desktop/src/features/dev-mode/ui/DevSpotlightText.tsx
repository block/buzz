import { cn } from "@/shared/lib/cn";

/**
 * Spotlight sweep used on channel names while an agent is working there.
 * The label keeps its inherited color: an animated mask dims the text except
 * for a bright band that sweeps back and forth (`dev-spotlight-text` in
 * animations.css), so it works over any label color (default, warning,
 * destructive).
 */
export function DevSpotlightText({
  text,
  className,
}: {
  text: string;
  className?: string;
}) {
  return (
    <span
      className={cn("dev-spotlight-text", className)}
      data-testid="dev-mode-spotlight-text"
    >
      {text}
    </span>
  );
}
