import * as React from "react";
import { CornerDownRight } from "lucide-react";

import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import type { TimelineMessage } from "@/features/messages/types";
import { cn } from "@/shared/lib/cn";
import { resolveCatchUpSourceState } from "./threadCatchUp";

const SOURCE_STATE_LABEL = {
  changed: "edited since the recap",
  unloaded: "not loaded",
} as const;

export function useThreadMessagesById(messages: readonly TimelineMessage[]) {
  return React.useMemo(() => {
    const map = new Map<string, TimelineMessage>();
    for (const message of messages) map.set(message.id.toLowerCase(), message);
    return map;
  }, [messages]);
}

/**
 * Cited messages for one point. Each opens the exact message through the
 * same channel navigation as message links; nothing is marked read here.
 */
export function ThreadCatchUpSources({
  bodies,
  channelId,
  citedBodies,
  messagesById,
  rootEventId,
  sourceIds,
}: {
  bodies: ReadonlyMap<string, string>;
  channelId: string;
  citedBodies: ReadonlyMap<string, string> | null;
  messagesById: ReadonlyMap<string, TimelineMessage>;
  rootEventId: string;
  sourceIds: readonly string[];
}) {
  const { goChannel } = useAppNavigation();
  return (
    <ul aria-label="Sources" className="mt-1 flex flex-wrap gap-1">
      {sourceIds.map((id, index) => {
        const message = messagesById.get(id);
        const state = resolveCatchUpSourceState(id, bodies, citedBodies);
        const name = message
          ? `${message.author} · ${message.time}`
          : `Source ${index + 1}`;
        const note = state === "current" ? null : SOURCE_STATE_LABEL[state];
        return (
          <li key={id}>
            <button
              aria-label={`Open source: ${name}${note ? `, ${note}` : ""}`}
              className={cn(
                "inline-flex max-w-full items-center gap-1 rounded-md px-1.5 py-0.5 text-2xs text-muted-foreground transition-colors hover:bg-muted hover:text-foreground focus-visible:outline-hidden focus-visible:ring-1 focus-visible:ring-ring",
                state === "changed" && "text-amber-700 dark:text-amber-400",
              )}
              data-source-id={id}
              data-source-state={state}
              data-testid="thread-catch-up-source"
              onClick={() => {
                void goChannel(channelId, {
                  messageId: id,
                  threadRootId: rootEventId,
                });
              }}
              type="button"
            >
              <CornerDownRight aria-hidden className="size-3 shrink-0" />
              <span className="truncate">{name}</span>
              {note ? <span className="shrink-0">· {note}</span> : null}
            </button>
          </li>
        );
      })}
    </ul>
  );
}
