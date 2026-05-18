import { Skeleton } from "@/shared/ui/skeleton";

export function TimelineSkeleton() {
  const skeletonRows = ["first", "second", "third", "fourth"];

  return (
    <>
      {skeletonRows.map((row, index) => (
        <div className="flex gap-2.5" key={row}>
          <Skeleton className="h-9 w-9 shrink-0 rounded-xl" />
          <div className="min-w-0 flex-1 space-y-1">
            <div className="flex items-baseline gap-2">
              <Skeleton className="h-3.5 w-28" />
              <Skeleton className="h-3 w-12" />
            </div>
            <Skeleton className="h-4 w-full" />
            <Skeleton className={index % 2 === 0 ? "h-4 w-4/5" : "h-4 w-2/3"} />
          </div>
        </div>
      ))}
    </>
  );
}
