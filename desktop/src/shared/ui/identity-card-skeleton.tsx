import { cn } from "@/shared/lib/cn";
import { Skeleton } from "@/shared/ui/skeleton";

type IdentityCardSkeletonProps = {
  className?: string;
  footerSubtitleWidthClass?: string;
  footerTitleWidthClass?: string;
  showAction?: boolean;
};

export function IdentityCardSkeleton({
  className,
  footerSubtitleWidthClass = "w-16",
  footerTitleWidthClass = "w-28",
  showAction = false,
}: IdentityCardSkeletonProps) {
  return (
    <div
      className={cn(
        "relative aspect-[4/5] w-full min-w-0 overflow-hidden rounded-xl border border-border/70 bg-muted/50 shadow-xs",
        className,
      )}
    >
      {showAction ? (
        <Skeleton className="absolute top-3 right-3 z-30 h-7 w-7 rounded-md bg-background/70" />
      ) : null}

      <SingleAvatarSkeleton />

      <div className="absolute right-3 bottom-3 left-3 z-30 flex min-w-0 flex-col gap-1 text-left">
        <Skeleton className={cn("h-4 max-w-full", footerTitleWidthClass)} />
        <Skeleton className={cn("h-4 max-w-full", footerSubtitleWidthClass)} />
      </div>
    </div>
  );
}

function SingleAvatarSkeleton() {
  return (
    <div className="absolute inset-x-0 top-0 bottom-12 flex items-center justify-center">
      <Skeleton className="h-24 w-24 rounded-full" />
    </div>
  );
}
