import * as React from "react";

import {
  getPersistentAgentAudienceRevision,
  promotePersistentAgentAudienceIfUnchanged,
  removePersistentAgentAudienceMembersIfUnchanged,
  usePersistentAgentAudience,
} from "@/features/messages/lib/persistentAgentAudience";
import { normalizePubkey } from "@/shared/lib/pubkey";

const CONFIRMATION_DURATION_MS = 4_000;

type Confirmation = {
  expectedRevision: number;
  pubkeys: readonly string[];
  scope: string;
  title: string;
};

type PendingPreferenceChange = {
  confirmation?: Confirmation;
  enabled: boolean;
  request: number;
};

type Options = {
  audienceScope: string | null;
  enabled: boolean;
  getDisplayName: (pubkey: string) => string | null | undefined;
  onPulse: (pubkey: string) => void;
  onTurnOff: () => void;
  onTurnOn: () => void;
};

export function useAutoPinMentionedAgents({
  audienceScope,
  enabled,
  getDisplayName,
  onPulse,
  onTurnOff,
  onTurnOn,
}: Options) {
  const { pubkeys: currentAudiencePubkeys } =
    usePersistentAgentAudience(audienceScope);
  const [confirmation, setConfirmation] = React.useState<Confirmation | null>(
    null,
  );
  const [confirmationHovered, setConfirmationHovered] = React.useState(false);
  const [openOptionsRequest, setOpenOptionsRequest] = React.useState(0);
  const nextOptionsRequestRef = React.useRef(0);
  const pendingPreferenceChangeRef =
    React.useRef<PendingPreferenceChange | null>(null);
  const onTurnOffRef = React.useRef(onTurnOff);
  const onTurnOnRef = React.useRef(onTurnOn);
  onTurnOffRef.current = onTurnOff;
  onTurnOnRef.current = onTurnOn;

  React.useEffect(() => {
    if (pendingPreferenceChangeRef.current?.enabled === enabled) {
      pendingPreferenceChangeRef.current = null;
    }
  }, [enabled]);

  React.useEffect(
    () => () => {
      const pending = pendingPreferenceChangeRef.current;
      pendingPreferenceChangeRef.current = null;
      if (!pending) return;
      if (pending.enabled) {
        onTurnOnRef.current();
      } else {
        onTurnOffRef.current();
      }
    },
    [],
  );

  const requestPreferenceChange = React.useCallback(
    (preferenceEnabled: boolean, pendingConfirmation?: Confirmation) => {
      const request = nextOptionsRequestRef.current + 1;
      nextOptionsRequestRef.current = request;
      pendingPreferenceChangeRef.current = {
        confirmation: pendingConfirmation,
        enabled: preferenceEnabled,
        request,
      };
      setOpenOptionsRequest(request);
    },
    [],
  );

  const completeOptionsReveal = React.useCallback((request: number) => {
    const pending = pendingPreferenceChangeRef.current;
    if (!pending || pending.request !== request) return;
    pendingPreferenceChangeRef.current = null;
    if (pending.enabled) {
      onTurnOnRef.current();
      return;
    }
    if (pending.confirmation) {
      removePersistentAgentAudienceMembersIfUnchanged({
        expectedRevision: pending.confirmation.expectedRevision,
        pubkeys: pending.confirmation.pubkeys,
        scope: pending.confirmation.scope,
      });
    }
    onTurnOffRef.current();
  }, []);

  const clearConfirmation = React.useCallback(() => {
    setConfirmationHovered(false);
    setConfirmation(null);
  }, []);

  React.useEffect(() => {
    if (!confirmation || confirmationHovered) return;
    const timeout = window.setTimeout(
      clearConfirmation,
      CONFIRMATION_DURATION_MS,
    );
    return () => window.clearTimeout(timeout);
  }, [clearConfirmation, confirmation, confirmationHovered]);

  const currentAudiencePubkeySet = React.useMemo(
    () => new Set(currentAudiencePubkeys.map(normalizePubkey).filter(Boolean)),
    [currentAudiencePubkeys],
  );
  const confirmationIsCurrent =
    confirmation?.scope === audienceScope &&
    confirmation.pubkeys.every((pubkey) =>
      currentAudiencePubkeySet.has(pubkey),
    );
  React.useEffect(() => {
    if (confirmation && !confirmationIsCurrent) clearConfirmation();
  }, [clearConfirmation, confirmation, confirmationIsCurrent]);

  const promoteAgents = React.useCallback(
    ({
      expectedRevision = audienceScope
        ? getPersistentAgentAudienceRevision(audienceScope)
        : 0,
      pubkeys,
      reinstateExcluded,
      requirePreference,
    }: {
      expectedRevision?: number;
      pubkeys: readonly string[];
      reinstateExcluded: boolean;
      requirePreference: boolean;
    }) => {
      if (!audienceScope || (requirePreference && !enabled)) return;
      const normalizedPubkeys = [
        ...new Set(pubkeys.map(normalizePubkey)),
      ].filter(Boolean);
      const promotion = promotePersistentAgentAudienceIfUnchanged({
        expectedRevision,
        reinstateExcluded,
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
      setConfirmationHovered(false);
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
    (promotion: {
      expectedRevision?: number;
      pubkeys: readonly string[];
      reinstateExcluded?: boolean;
    }) =>
      promoteAgents({
        ...promotion,
        reinstateExcluded: promotion.reinstateExcluded ?? false,
        requirePreference: true,
      }),
    [promoteAgents],
  );
  const promoteExplicitlyAddressedAgents = React.useCallback(
    (promotion: { expectedRevision?: number; pubkeys: readonly string[] }) => {
      promoteAgents({
        ...promotion,
        reinstateExcluded: true,
        requirePreference: false,
      });
      requestPreferenceChange(true);
    },
    [promoteAgents, requestPreferenceChange],
  );

  const dismissConfirmation = clearConfirmation;
  const turnOffConfirmation = React.useCallback(() => {
    if (!confirmation) return;
    clearConfirmation();
    requestPreferenceChange(false, confirmation);
  }, [clearConfirmation, confirmation, requestPreferenceChange]);

  return {
    confirmationTitle: confirmationIsCurrent ? confirmation.title : null,
    completeOptionsReveal,
    dismissConfirmation,
    openOptionsRequest,
    promoteExplicitlyAddressedAgents,
    promoteMentionedAgents,
    setConfirmationHovered,
    turnOffConfirmation,
  };
}
