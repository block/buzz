import { isTauri } from "@tauri-apps/api/core";
import { Eye, EyeOff } from "lucide-react";
import * as React from "react";

import { useCommunityBotsQuery } from "@/features/community-bots/hooks";
import {
  canViewCommunityBotSecret,
  VPS_SECRET_UNAVAILABLE,
  type RevealedBotSecret,
} from "@/features/community-bots/lib/types";
import { useMyRelayMembershipLookupQuery } from "@/features/community-members/hooks";
import { NsecMaskedDisplay } from "@/features/onboarding/ui/NsecMaskedDisplay";
import { ProfileSectionGroup } from "@/features/profile/ui/UserProfilePanelFields";
import { canManageCommunityMembers } from "@/shared/api/relayMembers";
import { invokeTauri } from "@/shared/api/tauri";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { Button } from "@/shared/ui/button";
import { PubKey } from "@/shared/ui/PubKey";

type CommunityBotIdentitySectionProps = {
  pubkey: string | null;
};

/** Desktop only: the web build never reveals or copies a bot's private key. */
function CommunityBotPrivateKeyRow({ pubkey }: { pubkey: string }) {
  if (!isTauri()) return null;
  return <CommunityBotPrivateKeyRowDesktop pubkey={pubkey} />;
}

function CommunityBotPrivateKeyRowDesktop({ pubkey }: { pubkey: string }) {
  const [isOpen, setIsOpen] = React.useState(false);
  const [nsec, setNsec] = React.useState<string | null>(null);
  const [unavailableReason, setUnavailableReason] = React.useState<
    string | null
  >(null);
  const [isLoading, setIsLoading] = React.useState(false);
  const [loadError, setLoadError] = React.useState<string | null>(null);
  const fetchCancelledRef = React.useRef(false);

  React.useEffect(() => {
    return () => {
      fetchCancelledRef.current = true;
      setNsec(null);
    };
  }, []);

  async function handleReveal() {
    if (!isOpen) {
      fetchCancelledRef.current = false;
      setIsOpen(true);
      setIsLoading(true);
      setLoadError(null);
      setUnavailableReason(null);
      try {
        const revealed = await invokeTauri<RevealedBotSecret>(
          "community_bots_reveal_identity_secret",
          { pubkey },
        );
        if (fetchCancelledRef.current) return;
        if (revealed.nsec) {
          setNsec(revealed.nsec);
          setUnavailableReason(null);
        } else {
          setNsec(null);
          setUnavailableReason(
            revealed.unavailableReason ?? VPS_SECRET_UNAVAILABLE,
          );
        }
      } catch (error) {
        if (fetchCancelledRef.current) return;
        setNsec(null);
        setLoadError(
          error instanceof Error
            ? error.message
            : "Could not request the VPS Buzz private key.",
        );
      } finally {
        if (!fetchCancelledRef.current) setIsLoading(false);
      }
      return;
    }

    fetchCancelledRef.current = true;
    setNsec(null);
    setUnavailableReason(null);
    setIsOpen(false);
  }

  return (
    <div className="px-4 py-3" data-testid="community-bot-private-key-row">
      <div className="flex items-center justify-between gap-4">
        <p className="text-sm font-medium">Private key</p>
        <Button
          aria-label={isOpen ? "Hide private key" : "Reveal private key"}
          className="rounded-full"
          data-testid="community-bot-private-key-toggle"
          onClick={() => void handleReveal()}
          type="button"
          variant="secondary"
        >
          {isOpen ? (
            <>
              <EyeOff className="h-4 w-4 shrink-0" />
              Hide
            </>
          ) : (
            <>
              <Eye className="h-4 w-4 shrink-0" />
              Reveal
            </>
          )}
        </Button>
      </div>
      {isOpen ? (
        <div className="mt-2">
          {isLoading ? (
            <p className="text-sm text-muted-foreground">Loading…</p>
          ) : loadError ? (
            <p
              className="text-sm text-destructive"
              data-testid="community-bot-private-key-error"
            >
              {loadError}
            </p>
          ) : nsec ? (
            <NsecMaskedDisplay nsec={nsec} />
          ) : (
            <p
              className="text-sm text-muted-foreground"
              data-testid="community-bot-private-key-unavailable"
            >
              {unavailableReason ?? VPS_SECRET_UNAVAILABLE}
            </p>
          )}
        </div>
      ) : null}
    </div>
  );
}

/** Owner/admin identity chrome for an installed community bot. */
export function CommunityBotIdentitySection({
  pubkey,
}: CommunityBotIdentitySectionProps) {
  const membership = useMyRelayMembershipLookupQuery();
  const botsQuery = useCommunityBotsQuery();
  const normalized = pubkey ? normalizePubkey(pubkey) : "";
  const isInstalledBot = (botsQuery.data ?? []).some(
    (bot) => normalizePubkey(bot.pubkey) === normalized,
  );
  const canView = canViewCommunityBotSecret({
    canManageCommunity: canManageCommunityMembers(membership.data),
    isInstalledBot,
  });

  if (!canView || !normalized) {
    return null;
  }

  return (
    <ProfileSectionGroup testId="community-bot-identity" title="Identity">
      <div className="flex min-h-16 items-center justify-between gap-4 px-4 py-3">
        <div className="min-w-0 space-y-1">
          <p className="text-sm font-medium">Public key</p>
          <PubKey pubkey={normalized} testId="community-bot-pubkey" />
        </div>
      </div>
      <CommunityBotPrivateKeyRow pubkey={normalized} />
    </ProfileSectionGroup>
  );
}
