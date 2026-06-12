import { Card } from "@/shared/ui/card";
import { useSidebar } from "@/shared/ui/sidebar";
import { Skeleton } from "@/shared/ui/skeleton";
import { cn } from "@/shared/lib/cn";
import { channelChrome } from "@/shared/layout/chromeLayout";
import { TopChromeInsetHeader } from "@/shared/layout/TopChromeInsetHeader";

type ViewLoadingFallbackKind =
  | "agents"
  | "channel"
  | "forum"
  | "projects"
  | "pulse"
  | "workflows";

type ViewLoadingFallbackProps = {
  includeHeader?: boolean;
  kind: ViewLoadingFallbackKind;
};

function LoadingHeaderSkeleton() {
  const { state: sidebarState } = useSidebar();

  return (
    <TopChromeInsetHeader data-tauri-drag-region>
      <header
        className={cn(
          "flex min-h-[44px] min-w-0 cursor-default select-none items-center gap-[10px] py-[6px] pl-[16px] pr-[8px] transition-[padding] duration-200 ease-linear sm:pl-[24px] sm:pr-[12px]",
          sidebarState === "collapsed" && "md:pl-[160px]",
        )}
      >
        <div className="min-w-0 flex-1">
          <div className="flex min-w-0 items-center gap-[6px]">
            <Skeleton className="h-3.5 w-3.5 rounded-xs" />
            <Skeleton className="h-4 w-28 max-w-[50vw]" />
          </div>
          <Skeleton className="mt-2 h-4 w-full max-w-2xl" />
        </div>
        <div className="hidden shrink-0 items-center gap-2 sm:flex">
          <Skeleton className="h-6 w-6 rounded-lg" />
          <Skeleton className="h-6 w-6 rounded-lg" />
        </div>
      </header>
    </TopChromeInsetHeader>
  );
}

function MessageRowsSkeleton() {
  return (
    <>
      {["first", "second", "third", "fourth", "fifth"].map((row, index) => (
        <div className="flex gap-3" key={row}>
          <Skeleton className="h-9 w-9 shrink-0 rounded-full" />
          <div className="min-w-0 flex-1 space-y-1 pt-0.5">
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

function AgentsLoadingBody() {
  return (
    <div className="flex min-h-0 min-w-0 flex-1 flex-col overflow-y-auto px-4 pb-4 pt-14 sm:px-6">
      <div className="space-y-6">
        <section className="space-y-4">
          <div className="flex items-center justify-between gap-3">
            <div className="space-y-2">
              <Skeleton className="h-5 w-28" />
              <Skeleton className="h-4 w-72 max-w-full" />
            </div>
            <Skeleton className="h-8 w-8 rounded-lg" />
          </div>
          <div className="grid gap-3 md:grid-cols-3 xl:grid-cols-4">
            {["first", "second", "third", "fourth"].map((card) => (
              <Card className="p-3" key={card}>
                <div className="flex items-center gap-2.5">
                  <Skeleton className="h-8 w-8 rounded-lg" />
                  <div className="min-w-0 flex-1 space-y-2">
                    <Skeleton className="h-4 w-24" />
                    <Skeleton className="h-3 w-16 rounded-full" />
                  </div>
                </div>
              </Card>
            ))}
          </div>
        </section>

        <section className="space-y-4">
          <div className="space-y-2">
            <Skeleton className="h-5 w-24" />
            <Skeleton className="h-4 w-80 max-w-full" />
          </div>
          <div className="grid gap-3 md:grid-cols-2 xl:grid-cols-3">
            {["alpha", "beta", "gamma"].map((card) => (
              <Card className="p-4" key={card}>
                <div className="space-y-3">
                  <div className="flex items-center justify-between gap-3">
                    <div className="space-y-2">
                      <Skeleton className="h-4 w-28" />
                      <Skeleton className="h-3 w-20 rounded-full" />
                    </div>
                    <Skeleton className="h-7 w-16 rounded-full" />
                  </div>
                  <Skeleton className="h-4 w-full" />
                  <Skeleton className="h-4 w-4/5" />
                </div>
              </Card>
            ))}
          </div>
        </section>

        <section className="space-y-4">
          <div className="space-y-2">
            <Skeleton className="h-5 w-36" />
            <Skeleton className="h-4 w-72 max-w-full" />
          </div>
          <Card className="overflow-hidden">
            {["one", "two", "three"].map((row) => (
              <div
                className="flex items-center gap-4 border-b border-border/60 px-4 py-3 last:border-b-0"
                key={row}
              >
                <Skeleton className="h-4 w-28" />
                <Skeleton className="h-5 w-16 rounded-full" />
                <Skeleton className="h-4 w-24" />
                <Skeleton className="ml-auto h-8 w-24 rounded-lg" />
              </div>
            ))}
          </Card>
        </section>
      </div>
    </div>
  );
}

function CardListLoadingBody() {
  return (
    <div className="flex min-h-0 min-w-0 flex-1 flex-col overflow-y-auto px-4 pb-4 pt-14 sm:px-6">
      <div className="mb-4 flex items-center justify-between gap-3">
        <div className="flex items-center gap-2">
          <Skeleton className="h-6 w-28" />
          <Skeleton className="h-8 w-8 rounded-lg" />
        </div>
        <Skeleton className="h-9 w-36 rounded-lg" />
      </div>

      <div className="space-y-2">
        {["first", "second", "third", "fourth"].map((card) => (
          <Card className="p-4" key={card}>
            <div className="flex items-start justify-between gap-4">
              <div className="min-w-0 flex-1 space-y-3">
                <div className="flex items-center gap-2">
                  <Skeleton className="h-5 w-44" />
                  <Skeleton className="h-5 w-16 rounded-full" />
                </div>
                <Skeleton className="h-4 w-full max-w-2xl" />
                <div className="flex flex-wrap gap-2">
                  <Skeleton className="h-5 w-20 rounded-full" />
                  <Skeleton className="h-5 w-24 rounded-full" />
                  <Skeleton className="h-5 w-16 rounded-full" />
                </div>
              </div>
              <div className="hidden shrink-0 gap-2 sm:flex">
                <Skeleton className="h-8 w-8 rounded-lg" />
                <Skeleton className="h-8 w-8 rounded-lg" />
              </div>
            </div>
          </Card>
        ))}
      </div>
    </div>
  );
}

function ChannelLoadingBody({ hasHeader = false }: { hasHeader?: boolean }) {
  return (
    <div className="flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden">
      <div className="flex-1 overflow-y-auto px-4 pt-1 sm:px-6">
        <div
          className={cn(
            "flex w-full flex-col gap-4",
            // The real channel header overlays content, so reserve its
            // measured height — unless an in-flow header skeleton is above.
            hasHeader ? "pt-3" : channelChrome.contentPadding,
          )}
        >
          <MessageRowsSkeleton />
        </div>
      </div>

      <div className="border-t border-border/60 bg-background px-4 py-4 sm:px-6">
        <div className="w-full space-y-3">
          <Skeleton className="h-10 w-full rounded-2xl" />
          <div className="flex items-center gap-2">
            <Skeleton className="h-8 w-20 rounded-lg" />
            <Skeleton className="h-8 w-16 rounded-lg" />
            <Skeleton className="ml-auto h-8 w-24 rounded-lg" />
          </div>
        </div>
      </div>
    </div>
  );
}

function ForumLoadingBody({ hasHeader = false }: { hasHeader?: boolean }) {
  return (
    <div
      className={cn(
        "flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden",
        !hasHeader && channelChrome.contentPadding,
      )}
    >
      <div className="border-b border-border/60 p-4">
        <Skeleton className="h-10 w-full rounded-xl" />
      </div>

      <div className="flex-1 overflow-y-auto p-4">
        <div className="space-y-3">
          {["first", "second", "third"].map((card) => (
            <Card className="p-4" key={card}>
              <div className="space-y-3">
                <div className="flex items-center gap-2">
                  <Skeleton className="h-4 w-40" />
                  <Skeleton className="h-5 w-16 rounded-full" />
                </div>
                <Skeleton className="h-4 w-full" />
                <Skeleton className="h-4 w-5/6" />
                <div className="flex items-center gap-3">
                  <Skeleton className="h-4 w-20" />
                  <Skeleton className="h-4 w-16" />
                  <Skeleton className="h-4 w-14" />
                </div>
              </div>
            </Card>
          ))}
        </div>
      </div>
    </div>
  );
}

export function ViewLoadingFallback({
  includeHeader = false,
  kind,
}: ViewLoadingFallbackProps) {
  return (
    <div className="flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden">
      {includeHeader ? <LoadingHeaderSkeleton /> : null}
      {kind === "agents" ? <AgentsLoadingBody /> : null}
      {kind === "workflows" ? <CardListLoadingBody /> : null}
      {kind === "projects" ? <CardListLoadingBody /> : null}
      {kind === "channel" ? (
        <ChannelLoadingBody hasHeader={includeHeader} />
      ) : null}
      {kind === "forum" ? <ForumLoadingBody hasHeader={includeHeader} /> : null}
      {kind === "pulse" ? (
        <ChannelLoadingBody hasHeader={includeHeader} />
      ) : null}
    </div>
  );
}
