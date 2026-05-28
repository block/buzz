import { Skeleton } from "@/shared/ui/skeleton";

export function HomeLoadingState() {
  return (
    <div className="flex-1 overflow-hidden">
      <div className="grid h-full min-h-0 w-full lg:grid-cols-[320px_minmax(0,1fr)]">
        <div className="overflow-hidden border-r border-border/70 bg-background/60">
          <div className="border-b border-border/70 px-4 pb-4 pt-14">
            <Skeleton className="h-4 w-20" />
            <Skeleton className="mt-2 h-4 w-28" />
            <Skeleton className="mt-4 h-10 rounded-md" />
          </div>
          <div className="space-y-3 px-4 py-4">
            {["a", "b", "c", "d"].map((row) => (
              <Skeleton className="h-20 rounded-md" key={row} />
            ))}
          </div>
        </div>

        <div className="overflow-hidden bg-background/60">
          <div className="border-b border-border/70 px-5 pb-4 pt-14">
            <Skeleton className="h-5 w-48" />
            <Skeleton className="mt-3 h-8 w-72" />
          </div>
          <div className="px-5 py-5">
            <Skeleton className="h-64 rounded-md" />
          </div>
        </div>
      </div>
    </div>
  );
}
