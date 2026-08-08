import { ArrowRight, CheckCheck } from "lucide-react";
import * as React from "react";

import { useAppShell } from "@/app/AppShellContext";
import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { buildCatchUpDigest } from "@/features/home/lib/catchUp";
import {
  formatInboxTimestamp,
  type InboxItem,
} from "@/features/home/lib/inbox";
import { resolveInboxItemReadAt } from "@/features/home/useHomeInboxReadState";
import { useUsersBatchQuery } from "@/features/profile/hooks";
import { resolveUserLabel } from "@/features/profile/lib/identity";
import { Button } from "@/shared/ui/button";

type CatchUpViewProps = {
  currentPubkey?: string;
  doneSet: ReadonlySet<string>;
  items: InboxItem[];
  /** Existing Inbox read machinery — routes to NIP-RS channel markers. */
  onMarkRead: (itemId: string) => void;
  onOpenMessage: (
    channelId: string,
    messageId: string,
    threadRootId?: string | null,
  ) => void;
};

/**
 * Catch up mode: a reading of what was said since the user's read boundary,
 * grouped by channel, newest channel first, oldest line first within a
 * channel. Rendering never moves any read marker — only the explicit
 * "Mark all as read" button does.
 */
export function CatchUpView({
  currentPubkey,
  doneSet,
  items,
  onMarkRead,
  onOpenMessage,
}: CatchUpViewProps) {
  const {
    getChannelReadAt,
    getMessageReadAt,
    getThreadReadAt,
    readStateVersion,
  } = useAppShell();
  const { goAttention, goChannel } = useAppNavigation();

  // biome-ignore lint/correctness/useExhaustiveDependencies: readStateVersion invalidates the stable marker getters
  const digest = React.useMemo(
    () =>
      buildCatchUpDigest({
        items,
        doneSet,
        getItemReadAt: (item) =>
          resolveInboxItemReadAt(item, {
            getChannelReadAt,
            getMessageReadAt,
            getThreadReadAt,
          }),
      }),
    [
      doneSet,
      getChannelReadAt,
      getMessageReadAt,
      getThreadReadAt,
      items,
      readStateVersion,
    ],
  );

  const authorPubkeys = React.useMemo(
    () => [
      ...new Set(
        digest.groups.flatMap((group) =>
          group.lines.map((line) => line.authorPubkey),
        ),
      ),
    ],
    [digest],
  );
  const profilesQuery = useUsersBatchQuery(authorPubkeys, {
    enabled: authorPubkeys.length > 0,
  });
  const profiles = profilesQuery.data?.profiles;

  const unreadItemIds = React.useMemo(
    () => items.filter((item) => !doneSet.has(item.id)).map((item) => item.id),
    [doneSet, items],
  );
  const handleMarkAllRead = () => {
    for (const id of unreadItemIds) {
      onMarkRead(id);
    }
  };

  const isEmpty = digest.groups.length === 0 && digest.needsYouCount === 0;

  return (
    <div
      className="-mt-13 min-h-0 flex-1 overflow-y-auto overflow-x-hidden overscroll-contain px-4 pb-6 pt-13"
      data-testid="catchup-view"
    >
      <div className="flex items-center gap-2 py-2">
        {digest.needsYouCount > 0 ? (
          <button
            className="flex items-center gap-1 rounded-full bg-primary/10 px-2.5 py-1 text-2xs font-medium text-primary transition-colors hover:bg-primary/15"
            data-testid="catchup-needs-count"
            onClick={() => void goAttention()}
            type="button"
          >
            {digest.needsYouCount} need you
            <ArrowRight className="h-3 w-3" />
          </button>
        ) : null}
        <div className="ml-auto">
          <Button
            data-testid="catchup-mark-all-read"
            disabled={unreadItemIds.length === 0}
            onClick={handleMarkAllRead}
            size="xs"
            type="button"
            variant="outline"
          >
            <CheckCheck />
            Mark all as read
          </Button>
        </div>
      </div>

      {isEmpty ? (
        <div
          className="flex flex-col items-center justify-center gap-2 rounded-2xl border border-dashed border-border/60 px-4 py-12 text-center"
          data-testid="catchup-empty"
        >
          <p className="text-sm text-muted-foreground">
            Nothing new since you last looked.
          </p>
        </div>
      ) : (
        <div className="flex flex-col gap-4">
          {digest.groups.map((group) => (
            <section data-testid="catchup-channel-group" key={group.channelId}>
              <h3 className="px-1 pb-1 text-2xs font-semibold uppercase tracking-wide text-muted-foreground">
                #{group.channelLabel}
              </h3>
              <div className="flex flex-col">
                {group.lines.map((line) => (
                  <button
                    className="flex w-full items-baseline gap-2 rounded-lg px-2 py-1.5 text-left transition-colors hover:bg-muted/50"
                    data-testid="catchup-line"
                    key={line.id}
                    onClick={() =>
                      onOpenMessage(line.channelId, line.id, line.threadRootId)
                    }
                    type="button"
                  >
                    <span className="shrink-0 text-sm font-medium text-foreground">
                      {resolveUserLabel({
                        pubkey: line.authorPubkey,
                        currentPubkey,
                        profiles,
                        preferResolvedSelfLabel: true,
                      })}
                    </span>
                    <span className="min-w-0 flex-1 truncate text-sm text-foreground/85">
                      {line.summary}
                    </span>
                    <span className="shrink-0 text-2xs text-muted-foreground">
                      {formatInboxTimestamp(line.createdAt)}
                    </span>
                  </button>
                ))}
                {group.moreCount > 0 ? (
                  <button
                    className="rounded-lg px-2 py-1.5 text-left text-2xs font-medium text-primary transition-colors hover:bg-muted/50"
                    data-testid="catchup-more"
                    onClick={() => void goChannel(group.channelId)}
                    type="button"
                  >
                    {group.moreCount} more in #{group.channelLabel}
                  </button>
                ) : null}
              </div>
            </section>
          ))}
        </div>
      )}
    </div>
  );
}
