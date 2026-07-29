import type { NameResolver } from "@/features/dev-mode/lib/useMemberNameResolver";
import type { RelayEvent } from "@/shared/api/types";
import { KIND_SYSTEM_MESSAGE } from "@/shared/constants/kinds";
import { cn } from "@/shared/lib/cn";

function formatTime(createdAt: number) {
  return new Date(createdAt * 1_000).toLocaleTimeString([], {
    hour: "2-digit",
    minute: "2-digit",
  });
}

export function DevMessageRow({
  event,
  isSelf,
  resolveName,
}: {
  event: RelayEvent;
  isSelf: boolean;
  resolveName: NameResolver;
}) {
  if (event.kind === KIND_SYSTEM_MESSAGE) {
    return null;
  }

  return (
    <div className="flex gap-2 py-0.5 text-sm leading-6">
      <span className="shrink-0 select-none text-muted-foreground/50">
        {formatTime(event.created_at)}
      </span>
      <span
        className={cn(
          "shrink-0 font-medium",
          isSelf ? "text-foreground" : "text-primary",
        )}
      >
        {resolveName(event.pubkey)}
      </span>
      <span
        className={cn(
          "min-w-0 whitespace-pre-wrap break-words",
          event.pending && "text-muted-foreground",
        )}
      >
        {event.content}
      </span>
    </div>
  );
}
