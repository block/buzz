import * as React from "react";

import { IDENTITY_CARD_GRID_CLASS } from "@/features/agents/ui/UnifiedAgentsSection";
import {
  useCommunityBotsQuery,
  useCommunityBotsStatusQuery,
} from "@/features/community-bots/hooks";
import {
  communityBotDirectoryCard,
  visibleCommunityDirectoryBots,
} from "@/features/community-bots/lib/directory";
import {
  BotsDirectoryGrid,
  BotsEmptyState,
} from "@/features/community-bots/ui/BotsDirectoryPanels";
import { CatalogDiagnosticCard } from "@/features/community-bots/ui/CatalogDiagnosticCard";
import { useRelayMembersQuery } from "@/features/community-members/hooks";
import { useIsArchivedPredicate } from "@/features/identity-archive/hooks";
import { usePresenceQuery } from "@/features/presence/hooks";
import { useUsersBatchQuery } from "@/features/profile/hooks";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { IdentityCardSkeleton } from "@/shared/ui/identity-card-skeleton";
import { PageHeader } from "@/shared/ui/PageHeader";

type BotsViewProps = {
  onOpenBot: (botId: string) => void;
  onOpenBotsSettings: () => void;
};

export function BotsView({ onOpenBot, onOpenBotsSettings }: BotsViewProps) {
  const catalogQuery = useCommunityBotsQuery();
  const statusQuery = useCommunityBotsStatusQuery();
  const membersQuery = useRelayMembersQuery();
  const isArchived = useIsArchivedPredicate();

  const bots = React.useMemo(
    () => visibleCommunityDirectoryBots(catalogQuery.data ?? [], isArchived),
    [catalogQuery.data, isArchived],
  );
  const pubkeys = React.useMemo(() => bots.map((bot) => bot.pubkey), [bots]);
  const profilesQuery = useUsersBatchQuery(pubkeys, {
    enabled: pubkeys.length > 0,
  });
  const presenceQuery = usePresenceQuery(pubkeys, {
    enabled: pubkeys.length > 0,
  });
  const profiles = profilesQuery.data?.profiles;
  const memberPubkeys = React.useMemo(() => {
    if (membersQuery.data === undefined) return null;
    return new Set(
      membersQuery.data.map((member) => normalizePubkey(member.pubkey)),
    );
  }, [membersQuery.data]);
  const isLoading =
    catalogQuery.isLoading ||
    (pubkeys.length > 0 && profilesQuery.isLoading && profiles === undefined);
  const cards = bots.map((bot) => {
    const pubkey = normalizePubkey(bot.pubkey);
    return communityBotDirectoryCard(
      bot,
      profiles?.[bot.pubkey] ?? profiles?.[pubkey],
      {
        gatewayState: statusQuery.data?.state,
        isRelayMember: memberPubkeys ? memberPubkeys.has(pubkey) : null,
        presence: presenceQuery.data?.[pubkey] ?? null,
      },
    );
  });

  return (
    <div className="flex-1 overflow-y-auto overflow-x-hidden overscroll-contain px-4 py-7 sm:px-6 sm:py-8">
      <div
        className="mx-auto w-full max-w-6xl space-y-8 [container-type:inline-size]"
        data-testid="bots-page-content"
      >
        <PageHeader
          description="Community bots installed for this workspace. Install or remove them in Settings → Communities → Bots."
          title="Bots"
        />

        {isLoading ? (
          <div className={IDENTITY_CARD_GRID_CLASS} data-testid="bots-loading">
            {["one", "two", "three"].map((key) => (
              <IdentityCardSkeleton key={key} />
            ))}
          </div>
        ) : null}

        {!isLoading && cards.length === 0 ? (
          <>
            <BotsEmptyState onOpenSettings={onOpenBotsSettings} />
            <CatalogDiagnosticCard />
          </>
        ) : null}

        {!isLoading && cards.length > 0 ? (
          <BotsDirectoryGrid cards={cards} onOpenBot={onOpenBot} />
        ) : null}
      </div>
    </div>
  );
}
