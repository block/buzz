import { Skeleton } from "@/shared/ui/skeleton";

export function HomeLoadingState() {
  return (
    <div className="min-h-0 flex-1 overflow-hidden">
      <div className="grid h-full min-h-0 w-full lg:grid-cols-[320px_minmax(0,1fr)]">
        <div className="relative overflow-hidden bg-background/60 after:absolute after:bottom-0 after:right-0 after:top-10 after:w-px after:bg-border/70 after:content-['']">
          <div className="px-5 py-1">
            <div className="flex min-w-0 items-center justify-between gap-3">
              <div className="flex min-w-0 items-center gap-[6px]">
                <Skeleton className="h-4 w-4 shrink-0 rounded-md" />
                <Skeleton className="h-4 w-14" />
              </div>
              <Skeleton className="h-7 w-14 rounded-full" />
            </div>
          </div>

          <div className="min-h-0 flex-1 overflow-y-auto">
            {["first", "second", "third", "fourth", "fifth"].map((row) => (
              <div
                className="flex w-full items-start gap-2.5 px-5 py-2"
                key={row}
              >
                <div className="relative shrink-0">
                  <Skeleton className="h-8 w-8 rounded-full" />
                  {row === "first" || row === "third" ? (
                    <Skeleton className="absolute -right-1 -top-1 h-2.5 w-2.5 rounded-full" />
                  ) : null}
                </div>
                <div className="min-w-0 flex-1">
                  <div className="flex items-start gap-2">
                    <div className="min-w-0 flex-1">
                      <div className="flex items-center gap-2">
                        <Skeleton className="h-4 w-24" />
                        {row === "second" ? (
                          <Skeleton className="h-3 w-20" />
                        ) : null}
                      </div>
                    </div>
                    <Skeleton className="h-3 w-10" />
                  </div>
                  <div className="mt-1 space-y-1.5">
                    <Skeleton className="h-4 w-full" />
                    <Skeleton className="h-4 w-4/5" />
                  </div>
                  <Skeleton className="mt-2 h-3 w-16" />
                </div>
              </div>
            ))}
          </div>
        </div>

        <div className="relative flex min-h-0 flex-col overflow-hidden bg-background/60">
          <div className="px-5 py-1 pr-3">
            <div className="flex min-w-0 items-center justify-between gap-3">
              <div className="flex min-w-0 items-center gap-[4px]">
                <Skeleton className="h-4 w-4 shrink-0 rounded-md" />
                <Skeleton className="h-4 w-40" />
              </div>
              <div className="flex shrink-0 items-center gap-1">
                <Skeleton className="h-8 w-20 rounded-full" />
                <Skeleton className="h-8 w-8 rounded-full" />
              </div>
            </div>
          </div>

          <div className="min-h-0 flex-1 overflow-y-auto pb-32">
            {["selected", "context-a", "context-b"].map((row, index) => (
              <div className="relative px-5 py-2" key={row}>
                {index === 1 ? (
                  <div className="mx-1 mb-3 border-t border-border/60" />
                ) : null}
                <article className="relative flex items-start gap-2.5">
                  <Skeleton className="h-8 w-8 shrink-0 rounded-full" />
                  <div className="min-w-0 flex-1">
                    <div className="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-0">
                      <Skeleton className="h-4 w-24" />
                      <Skeleton className="h-3 w-24" />
                    </div>
                    <div className="mt-1 space-y-1.5">
                      <Skeleton className="h-4 w-full max-w-2xl" />
                      <Skeleton
                        className={
                          index === 0
                            ? "h-4 w-5/6 max-w-xl"
                            : "h-4 w-2/3 max-w-lg"
                        }
                      />
                    </div>
                    <div className="mt-2 flex items-center gap-4">
                      <Skeleton className="h-4 w-8 rounded-full" />
                      <Skeleton className="h-4 w-8 rounded-full" />
                    </div>
                  </div>
                </article>
              </div>
            ))}
          </div>

          <div className="pointer-events-none absolute inset-x-0 bottom-0 z-10">
            <div className="pointer-events-auto px-4 pb-4 sm:px-4">
              <div className="relative isolate rounded-2xl border border-border/50 bg-background/80 px-3 pb-2 pt-3 shadow-none backdrop-blur-md sm:px-4">
                <Skeleton className="h-5 w-48" />
                <div className="mt-4 flex items-center gap-2">
                  <Skeleton className="h-8 w-8 rounded-lg" />
                  <Skeleton className="h-8 w-8 rounded-lg" />
                  <Skeleton className="ml-auto h-8 w-20 rounded-full" />
                </div>
              </div>
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}
