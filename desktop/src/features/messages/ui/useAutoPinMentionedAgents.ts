import * as React from "react";

import {
  getPersistentAgentAudienceRevision,
  promotePersistentAgentAudienceIfUnchanged,
  removePersistentAgentAudienceMembersIfUnchanged,
} from "@/features/messages/lib/persistentAgentAudience";
import { normalizePubkey } from "@/shared/lib/pubkey";

const CONFIRMATION_DURATION_MS = 4_000;

type Confirmation = {
  expectedRevision: number;
  pubkeys: readonly string[];
  scope: string;
  title: string;
};

type Options = {
  audienceScope: string | null;
  enabled: boolean;
  getDisplayName: (pubkey: string) => string | null | undefined;
  onPulse: (pubkey: string) => void;
  onTurnOff: () => void;
};

export function useAutoPinMentionedAgents({
  audienceScope,
  enabled,
  getDisplayName,
  onPulse,
  onTurnOff,
}: Options) {
  const [confirmation, setConfirmation] = React.useState<Confirmation | null>(
    null,
  );

  React.useEffect(() => {
    if (!confirmation) return;
    const timeout = window.setTimeout(
      () => setConfirmation(null),
      CONFIRMATION_DURATION_MS,
    );
    return () => window.clearTimeout(timeout);
  }, [confirmation]);

  const promoteAgents = React.useCallback(
    ({
      expectedRevision = audienceScope
        ? getPersistentAgentAudienceRevision(audienceScope)
        : 0,
      pubkeys,
      requirePreference,
    }: {
      expectedRevision?: number;
      pubkeys: readonly string[];
      requirePreference: boolean;
    }) => {
      if (!audienceScope || (requirePreference && !enabled)) return;
      const normalizedPubkeys = [
        ...new Set(pubkeys.map(normalizePubkey)),
      ].filter(Boolean);
      const promotion = promotePersistentAgentAudienceIfUnchanged({
        expectedRevision,
        pubkeys: normalizedPubkeys,
        scope: audienceScope,
      });
      if (promotion === null) return;
      const { promotedPubkeys, revision } = promotion;
      for (const pubkey of promotedPubkeys) onPulse(pubkey);

      const displayName =
        promotedPubkeys.length === 1
          ? getDisplayName(promotedPubkeys[0])?.trim()
          : null;
      const title = displayName
        ? `${displayName} will be mentioned automatically`
        : promotedPubkeys.length === 1
          ? "Agent will be mentioned automatically"
          : `${promotedPubkeys.length} agents will be mentioned automatically`;
      setConfirmation({
        expectedRevision: revision,
        pubkeys: promotedPubkeys,
        scope: audienceScope,
        title,
      });
    },
    [audienceScope, enabled, getDisplayName, onPulse],
  );
  const promoteMentionedAgents = React.useCallback(
    (promotion: { expectedRevision?: number; pubkeys: readonly string[] }) =>
      promoteAgents({ ...promotion, requirePreference: true }),
    [promoteAgents],
  );
  const promoteExplicitlyAddressedAgents = React.useCallback(
    (promotion: { expectedRevision?: number; pubkeys: readonly string[] }) =>
      promoteAgents({ ...promotion, requirePreference: false }),
    [promoteAgents],
  );

  const dismissConfirmation = React.useCallback(
    () => setConfirmation(null),
    [],
  );
  const turnOffConfirmation = React.useCallback(() => {
    if (!confirmation) return;
    setConfirmation(null);
    removePersistentAgentAudienceMembersIfUnchanged({
      expectedRevision: confirmation.expectedRevision,
      pubkeys: confirmation.pubkeys,
      scope: confirmation.scope,
    });
    onTurnOff();
  }, [confirmation, onTurnOff]);

  return {
    confirmationTitle:
      confirmation?.scope === audienceScope ? confirmation.title : null,
    dismissConfirmation,
    promoteExplicitlyAddressedAgents,
    promoteMentionedAgents,
    turnOffConfirmation,
  };
}
