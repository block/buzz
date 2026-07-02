import { cn } from "@/shared/lib/cn";
import { Skeleton } from "@/shared/ui/skeleton";

const chatListSkeletonRows = [
  { key: "project-primary", width: "w-32" },
  { key: "project-secondary", width: "w-24" },
  { key: "project-tertiary", width: "w-36" },
  { key: "chat-primary", width: "w-28" },
  { key: "chat-secondary", width: "w-20" },
] as const;

export function ChatListSkeleton() {
  return (
    <div
      aria-label="Loading chats"
      aria-live="polite"
      className="min-h-0 flex-1 overflow-hidden p-2 pt-3"
      role="status"
    >
      <div className="mb-3 space-y-1">
        <div className="flex h-8 items-center gap-1.5 rounded-md px-2">
          <Skeleton className="h-3.5 w-3.5 shrink-0 rounded-sm" />
          <Skeleton className="h-3 w-24" />
          <Skeleton className="ml-auto h-6 w-6 rounded-md" />
        </div>
        <div className="space-y-1">
          {chatListSkeletonRows.slice(0, 3).map((row) => (
            <div
              className="flex h-8 items-center gap-2 rounded-md px-3"
              key={row.key}
            >
              <Skeleton className={cn("h-3.5", row.width)} />
            </div>
          ))}
        </div>
      </div>
      <div className="space-y-1">
        <div className="flex h-8 items-center gap-2 rounded-md px-2">
          <Skeleton className="h-3 w-16" />
          <Skeleton className="ml-auto h-6 w-6 rounded-md" />
        </div>
        {chatListSkeletonRows.slice(3).map((row) => (
          <div
            className="flex h-8 items-center gap-2 rounded-md px-3"
            key={row.key}
          >
            <Skeleton className={cn("h-3.5", row.width)} />
          </div>
        ))}
      </div>
    </div>
  );
}
