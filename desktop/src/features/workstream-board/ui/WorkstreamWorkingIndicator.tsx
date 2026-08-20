import { Loader2 } from "lucide-react";

import type { ActiveChannelTurnSummary } from "@/features/agents/activeAgentTurnsStore";
import { formatElapsed } from "@/features/agents/ui/agentSessionUtils";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import { normalizePubkey, truncatePubkey } from "@/shared/lib/pubkey";
import { useNow } from "@/shared/lib/useNow";
import { UserAvatar } from "@/shared/ui/UserAvatar";

type WorkstreamWorkingIndicatorProps = {
  activeWorking: ActiveChannelTurnSummary;
  profiles?: UserProfileLookup;
};

/**
 * Presentation-only projection of the shared active-turn store for one board
 * card. Ingestion, ordering, and stall pruning stay owned by
 * activeAgentTurnsStore.
 */
export function WorkstreamWorkingIndicator({
  activeWorking,
  profiles,
}: WorkstreamWorkingIndicatorProps) {
  const now = useNow(1000);
  const elapsed = formatElapsed(now - activeWorking.anchorAt);

  return (
    <section
      aria-label={`${activeWorking.agentCount} active agent${activeWorking.agentCount === 1 ? "" : "s"}`}
      className="mt-4 border-t border-border/60 pt-3"
      data-testid={`workstream-working-${activeWorking.channelId}`}
    >
      <div className="flex items-center gap-1.5 text-2xs font-medium text-primary">
        <Loader2 aria-hidden="true" className="size-3 animate-spin" />
        <span>
          {activeWorking.agentCount} working · {elapsed}
        </span>
      </div>
      <div className="mt-2 flex flex-wrap gap-1.5">
        {activeWorking.agentPubkeys.map((pubkey) => {
          const profile = profiles?.[normalizePubkey(pubkey)];
          const name =
            profile?.displayName?.trim() || `Agent ${truncatePubkey(pubkey)}`;
          return (
            <span
              className="inline-flex max-w-full items-center gap-1 rounded-full border border-primary/25 bg-primary/10 py-0.5 pl-1 pr-2 text-2xs text-foreground"
              data-testid={`workstream-working-agent-${pubkey}`}
              key={pubkey}
              title={name}
            >
              <UserAvatar
                avatarUrl={profile?.avatarUrl ?? null}
                className="size-3.5 shrink-0"
                displayName={name}
                size="sm"
              />
              <span className="truncate">{name}</span>
            </span>
          );
        })}
      </div>
    </section>
  );
}
