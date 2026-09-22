import { cn } from "@/shared/lib/cn";

type ComposerDockGlassBackdropProps = {
  className?: string;
  testId?: string;
};

/**
 * Applies the composer dock's shared blur without adding color or layout.
 * Reuse it for composer-adjacent surfaces that need the same glass treatment.
 */
export function ComposerDockGlassBackdrop({
  className,
  testId,
}: ComposerDockGlassBackdropProps) {
  return (
    <div
      aria-hidden="true"
      className={cn(
        "pointer-events-none backdrop-blur-md dark:backdrop-blur-xl",
        className,
      )}
      data-testid={testId}
    />
  );
}

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
        <ComposerDockGlassBackdrop className="h-full w-full rounded-2xl" />
      </div>
      <div
        aria-hidden="true"
        className="composer-dock-rail-mask pointer-events-none absolute inset-x-0 bottom-0 z-[5] bg-background"
        data-testid="composer-dock-rail-mask"
      />
    </>
  );
}
