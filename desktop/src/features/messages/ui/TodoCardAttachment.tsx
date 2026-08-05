import { ListTodo } from "lucide-react";
import * as React from "react";
import { toast } from "sonner";

import {
  countDoneItems,
  reduceTodoResponses,
  type TodoCardPayload,
  type TodoItemState,
} from "@/features/messages/lib/todoCard";
import { useUsersBatchQuery } from "@/features/profile/hooks";
import {
  sendCardResponse,
  subscribeToCardResponses,
} from "@/shared/api/cardResponses";
import { useIdentityQuery } from "@/shared/api/hooks";
import type { RelayEvent, UserProfileSummary } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import { truncatePubkey } from "@/shared/lib/pubkey";
import {
  Attachment,
  AttachmentContent,
  AttachmentDescription,
  AttachmentMedia,
  AttachmentTitle,
} from "@/shared/ui/attachment";
import { UserAvatar } from "@/shared/ui/UserAvatar";

type TodoCardAttachmentProps = {
  card: TodoCardPayload;
  /** Channel the card message lives in — 40009 responses carry it as `h`. */
  channelId: string;
  /** Event id of the card message — 40009 responses reference it via `e`. */
  cardEventId: string;
  className?: string;
};

function profileName(
  profiles: Record<string, UserProfileSummary> | undefined,
  pubkey: string,
): string {
  const profile = profiles?.[pubkey.toLowerCase()];
  return profile?.displayName ?? profile?.name ?? truncatePubkey(pubkey);
}

export function TodoCardAttachment({
  card,
  channelId,
  cardEventId,
  className,
}: TodoCardAttachmentProps) {
  const identityQuery = useIdentityQuery();
  const ownPubkey = identityQuery.data?.pubkey ?? null;

  const [itemState, setItemState] = React.useState<Map<string, TodoItemState>>(
    () => reduceTodoResponses(card, cardEventId, []),
  );
  const [pendingItemIds, setPendingItemIds] = React.useState<Set<string>>(
    () => new Set(),
  );
  // The subscription effect owns the fold input; clicks append the publish
  // acknowledgement through this ref so both paths share one `seenEvents` map.
  const appendEventRef = React.useRef<(event: RelayEvent) => void>(() => {});

  React.useEffect(() => {
    let disposed = false;
    let cleanup: (() => void) | null = null;
    const seenEvents = new Map<string, RelayEvent>();

    function updateState() {
      if (disposed) return;
      setItemState(reduceTodoResponses(card, cardEventId, seenEvents.values()));
    }

    appendEventRef.current = (event: RelayEvent) => {
      if (disposed || seenEvents.has(event.id)) return;
      seenEvents.set(event.id, event);
      updateState();
    };

    updateState();
    subscribeToCardResponses(channelId, cardEventId, (event) => {
      appendEventRef.current(event);
    })
      .then((dispose) => {
        if (disposed) {
          void dispose();
          return;
        }
        cleanup = () => void dispose();
      })
      .catch((error) => {
        console.error("[TodoCardAttachment] subscription failed:", error);
      });

    return () => {
      disposed = true;
      cleanup?.();
    };
  }, [card, cardEventId, channelId]);

  const doneCount = countDoneItems(card, itemState);

  // Assignees + completers, for avatars and "completed by" attribution.
  const profilePubkeys = React.useMemo(() => {
    const pubkeys = new Set<string>();
    for (const item of card.items) {
      if (item.assignee) pubkeys.add(item.assignee);
      const completedBy = itemState.get(item.id)?.completedBy;
      if (completedBy) pubkeys.add(completedBy);
    }
    return [...pubkeys];
  }, [card.items, itemState]);
  const profilesQuery = useUsersBatchQuery(profilePubkeys);
  const profiles = profilesQuery.data?.profiles;

  async function handleToggle(itemId: string, nextDone: boolean) {
    if (pendingItemIds.has(itemId)) return;
    setPendingItemIds((prev) => new Set(prev).add(itemId));
    try {
      const event = await sendCardResponse(
        channelId,
        cardEventId,
        itemId,
        nextDone,
      );
      // The relay also fans the event back through the subscription; the
      // seen-map dedupes, so folding the acknowledgement here is just the
      // faster path.
      appendEventRef.current(event);
    } catch (error) {
      toast.error(
        error instanceof Error
          ? error.message
          : "Failed to update the to-do item.",
      );
    } finally {
      setPendingItemIds((prev) => {
        const next = new Set(prev);
        next.delete(itemId);
        return next;
      });
    }
  }

  return (
    <Attachment
      className={cn("w-96 max-w-full shadow-none", className)}
      data-testid="todo-card-attachment"
    >
      <AttachmentMedia>
        <ListTodo />
      </AttachmentMedia>
      <AttachmentContent>
        <AttachmentTitle>{card.title ?? "To-do"}</AttachmentTitle>
        <AttachmentDescription>
          {doneCount} of {card.items.length} done
        </AttachmentDescription>
        <ul
          className="mt-1.5 flex flex-col gap-1"
          data-testid="todo-card-items"
        >
          {card.items.map((item) => {
            const state = itemState.get(item.id);
            const done = state?.done ?? false;
            const completedBy = state?.completedBy ?? null;
            const pending = pendingItemIds.has(item.id);
            // Un-checking replays as "your latest response wins", so only the
            // standing completer can un-check their own completion.
            const canToggle =
              ownPubkey !== null &&
              !pending &&
              (!done || completedBy === ownPubkey);
            const completerProfile = completedBy
              ? profiles?.[completedBy.toLowerCase()]
              : undefined;
            const completedByOther =
              done && completedBy !== null && completedBy !== item.assignee;

            return (
              <li key={item.id}>
                <label
                  className={cn(
                    "flex w-full items-start gap-2 rounded-md px-1 py-0.5 text-left text-sm",
                    canToggle
                      ? "cursor-pointer hover:bg-accent"
                      : "cursor-default",
                    pending && "opacity-60",
                  )}
                  title={
                    done && !canToggle && completedBy
                      ? `Completed by ${profileName(profiles, completedBy)} — only they can un-check it`
                      : undefined
                  }
                >
                  <input
                    checked={done}
                    className="mt-0.5 h-4 w-4 shrink-0 accent-primary"
                    disabled={!canToggle}
                    onChange={() => void handleToggle(item.id, !done)}
                    type="checkbox"
                  />
                  <span
                    className={cn(
                      "min-w-0 flex-1",
                      done && "text-muted-foreground line-through",
                    )}
                  >
                    {item.text}
                  </span>
                  {done && completedBy ? (
                    <span
                      className="flex shrink-0 items-center gap-1 text-xs text-muted-foreground"
                      data-testid="todo-card-completer"
                    >
                      <UserAvatar
                        avatarUrl={completerProfile?.avatarUrl ?? null}
                        displayName={profileName(profiles, completedBy)}
                        size="xs"
                      />
                      {completedByOther
                        ? `by ${profileName(profiles, completedBy)}`
                        : null}
                    </span>
                  ) : null}
                </label>
              </li>
            );
          })}
        </ul>
      </AttachmentContent>
    </Attachment>
  );
}
