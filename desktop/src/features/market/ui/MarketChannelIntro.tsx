import type * as React from "react";
import { useAppNavigation } from "@/app/navigation/useAppNavigation";

import { MessageCircle } from "lucide-react";

import { useMarketChannel } from "@/features/market/lib/MarketChannelContext";
import { resolveMarketBidIdentity } from "@/features/market/lib/marketBidIdentity";
import type { MarketBid } from "@/features/market/lib/marketProtocol";
import {
  isMarketProtocolMessage,
  marketBidsAfterAnchor,
} from "@/features/market/lib/marketTimeline";
import { MarketContractCard } from "@/features/market/ui/MarketContractCard";
import type { TimelineMessage } from "@/features/messages/types";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import { Button } from "@/shared/ui/button";
import { UserAvatar } from "@/shared/ui/UserAvatar";

const MARKET_BOARD_GUTTER_CLASS = "mx-4";

/** Aligns the market board with the channel header and timeline avatars. */
export function MarketBoardLayout({ children }: { children: React.ReactNode }) {
  return <div className={`${MARKET_BOARD_GUTTER_CLASS} mt-2`}>{children}</div>;
}

export function MarketBidTable({
  bids,
  onOpenBid,
  profiles,
}: {
  bids: MarketBid[];
  onOpenBid: (eventId: string) => void;
  profiles?: UserProfileLookup;
}) {
  return (
    <div className="overflow-x-auto rounded-xl border border-border/70">
      <table className="w-full min-w-[28rem] table-fixed border-collapse text-left text-sm">
        <thead className="bg-muted/35 text-xs text-muted-foreground">
          <tr>
            <th className="w-[12rem] px-3 py-2 font-medium" scope="col">
              Bidder
            </th>
            <th className="px-3 py-2 font-medium" scope="col">
              Bid
            </th>
            <th className="w-[6.5rem] px-3 py-2 font-medium" scope="col">
              <span className="sr-only">Action</span>
            </th>
          </tr>
        </thead>
        <tbody className="divide-y divide-border/60">
          {bids.map((bid) => {
            const { avatarUrl, displayName } = resolveMarketBidIdentity(
              bid,
              profiles,
            );
            return (
              <tr key={bid.eventId}>
                <th className="px-3 py-2 font-medium" scope="row">
                  <div className="flex min-w-0 items-center gap-2">
                    <UserAvatar
                      avatarUrl={avatarUrl}
                      className="h-7 w-7 shrink-0 text-xs"
                      displayName={displayName}
                      shape="squircle"
                      size="sm"
                    />
                    <span className="truncate">{displayName}</span>
                  </div>
                </th>
                <td className="whitespace-nowrap px-3 py-2 text-muted-foreground">
                  {bid.amountSats != null
                    ? `${bid.amountSats} sats per unit`
                    : "Terms in thread"}
                  {` · ${bid.quantity} ${bid.quantity === 1 ? "unit" : "units"}`}
                </td>
                <td className="px-2 py-1 text-right">
                  <Button
                    aria-label={`Open negotiation with ${displayName}`}
                    onClick={() => onOpenBid(bid.eventId)}
                    size="sm"
                    type="button"
                    variant="ghost"
                  >
                    <MessageCircle className="mr-1.5 h-4 w-4" />
                    Thread
                  </Button>
                </td>
              </tr>
            );
          })}
        </tbody>
      </table>
    </div>
  );
}

function BidList({
  bids,
  channelId,
  profiles,
}: {
  bids: MarketBid[];
  channelId: string;
  profiles?: UserProfileLookup;
}) {
  const { goChannel } = useAppNavigation();
  return (
    <section className="mt-4" data-testid="market-bid-list">
      <div className="mb-2 flex items-center justify-between gap-3">
        <h3 className="text-sm font-semibold">Bids</h3>
        <span className="text-xs text-muted-foreground">
          {bids.length} {bids.length === 1 ? "bid" : "bids"}
        </span>
      </div>
      {bids.length === 0 ? (
        <p className="text-sm text-muted-foreground">No bids yet.</p>
      ) : (
        <MarketBidTable
          bids={bids}
          onOpenBid={(eventId) => {
            void goChannel(channelId, { thread: eventId });
          }}
          profiles={profiles}
        />
      )}
    </section>
  );
}

export function MarketChannelIntro({
  anchorMessage,
  bids,
  profiles,
}: {
  anchorMessage: Pick<TimelineMessage, "body" | "createdAt" | "id">;
  bids: MarketBid[];
  profiles?: UserProfileLookup;
}): React.ReactNode {
  const projection = useMarketChannel();
  if (!projection || isMarketProtocolMessage(anchorMessage)) return undefined;
  const visibleBids = marketBidsAfterAnchor(bids, anchorMessage);
  return (
    <MarketBoardLayout>
      <MarketContractCard scenario={projection.scenario} />
      <BidList
        bids={visibleBids}
        channelId={projection.channelId}
        profiles={profiles}
      />
    </MarketBoardLayout>
  );
}
