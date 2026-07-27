import { cn } from "@/shared/lib/cn";

type ComposerDockBackdropProps = {
  gutterClassName: string;
};

/**
 * Owns the dock's stable backdrop blur separately from the resizing composer.
 * An opaque rail mask covers the portion released for activity content.
 */
export function ComposerDockBackdrop({
  gutterClassName,
}: ComposerDockBackdropProps) {
  return (
    <>
      <div
        aria-hidden="true"
        className={cn(
          "pointer-events-none absolute inset-y-0 z-0",
          gutterClassName,
        )}
        data-testid="composer-dock-backdrop"
      >
        <div className="h-full w-full rounded-2xl backdrop-blur-md dark:backdrop-blur-xl" />
      </div>
      <div
        aria-hidden="true"
        className="composer-dock-rail-mask pointer-events-none absolute inset-x-0 bottom-0 z-[5] bg-background"
        data-testid="composer-dock-rail-mask"
      />
    </>
  );
}
