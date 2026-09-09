import type { UserProfileSummary } from "@/shared/api/types";
import { Skeleton } from "@/shared/ui/skeleton";
import type { BriefingGroup, BriefingKind } from "../lib/pulseBriefing";
import type { PulseConversation } from "../lib/unifiedFeed";
import { ConversationCard } from "./ConversationCard";

export function PulseBriefing({
  groups,
  conversations,
  profiles,
  currentPubkey,
  onSelect,
  onRefresh,
  loading,
  hasError,
  onRetry,
}: {
  groups: BriefingGroup[];
  conversations: Map<string, PulseConversation>;
  profiles: Record<string, UserProfileSummary>;
  currentPubkey?: string;
  onSelect: (kind: BriefingKind) => void;
  onRefresh: () => void;
  loading: boolean;
  hasError: boolean;
  onRetry: () => void;
}) {
  return (
    <section aria-label="Your catch-up" data-testid="pulse-briefing">
      <header className="flex items-baseline justify-between gap-3 border-b border-border/50 px-5 py-5 sm:px-7">
        <h2 className="text-sm font-medium">Your catch-up</h2>
        <span className="text-xs text-muted-foreground">Past 48 hours</span>
      </header>
      {loading ? (
        <div
          role="status"
          aria-label="Finding what needs your attention"
          className="space-y-5 px-5 py-6 sm:px-7"
        >
          <Skeleton className="h-12 w-12 rounded-full" />
          <Skeleton className="h-5 w-4/5" />
          <Skeleton className="h-5 w-3/5" />
          <span className="sr-only">Finding what needs your attention…</span>
        </div>
      ) : groups.length ? (
        groups.map((group) => {
          const item = conversations.get([...group.ids][0]);
          if (!item) return null;
          return (
            <ConversationCard
              key={group.kind}
              item={item}
              summary={group.label}
              profiles={profiles}
              currentPubkey={currentPubkey}
              onRefresh={onRefresh}
              onOpenContext={() => onSelect(group.kind)}
            />
          );
        })
      ) : !hasError ? (
        <p className="px-5 py-10 text-sm text-muted-foreground sm:px-7">
          No recent conversations to recap yet.
        </p>
      ) : null}
      {hasError && !loading && (
        <div
          role="alert"
          className="px-5 py-6 text-sm text-muted-foreground sm:px-7"
        >
          <p>Your catch-up is incomplete. Some activity couldn’t be loaded.</p>
          <button
            type="button"
            onClick={onRetry}
            className="mt-3 rounded-sm underline underline-offset-4 focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring"
          >
            Try again
          </button>
        </div>
      )}
    </section>
  );
}
