import { cn } from "@/shared/lib/cn";

/**
 * Blurred strip pinned to the top of a scroll container so content scrolls
 * *under* the global search chrome instead of showing through it.
 *
 * Render as the first child of a `relative` (non-scrolling) parent, alongside
 * the scrollable content. It is purely decorative (aria-hidden,
 * pointer-events-none) and sits at z-40 — below the global chrome controls
 * (z-[45]) but above page content.
 *
 * Pass the height via `className` (e.g. `h-10` for pages with no sub-header,
 * `h-[76px]` for panels whose own header occupies the top). `cn`/twMerge lets
 * the passed height override the default.
 */
export function TopChromeBackdrop({ className }: { className?: string }) {
  return (
    <div
      aria-hidden="true"
      className={cn(
        "pointer-events-none absolute inset-x-0 top-0 z-40 h-10 bg-background/75 backdrop-blur-md supports-[backdrop-filter]:bg-background/65 after:absolute after:inset-x-0 after:top-10 after:h-px after:bg-border/35 after:content-[''] dark:bg-background/45 dark:backdrop-blur-xl dark:supports-[backdrop-filter]:bg-background/35",
        className,
      )}
    />
  );
}
