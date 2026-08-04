import { cn } from "@/shared/lib/cn";

const STAGGER_MS = 70;

/**
 * Per-character wave used on channel names while an agent is working there.
 * Each character gets its own inline-block span with a staggered animation
 * delay — transforms silently no-op on plain inline spans, so the
 * `dev-wavy-char` class (animations.css) must keep them inline-block.
 * `whitespace-pre` stops inter-span spaces from collapsing.
 */
export function DevWavyText({
  text,
  className,
}: {
  text: string;
  className?: string;
}) {
  return (
    <span
      className={cn("whitespace-pre", className)}
      data-testid="dev-mode-wavy-text"
    >
      {[...text].map((char, index) => (
        <span
          className="dev-wavy-char"
          // biome-ignore lint/suspicious/noArrayIndexKey: characters are static per render
          key={index}
          style={{ animationDelay: `${index * STAGGER_MS}ms` }}
        >
          {char}
        </span>
      ))}
    </span>
  );
}
