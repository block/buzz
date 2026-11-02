import * as React from "react";
import { toast } from "sonner";

import { useMyRelayMembershipLookupQuery } from "@/features/community-members/hooks";
import { useCommunities } from "@/features/communities/useCommunities";
import { useChannelsQuery, useOpenDmMutation } from "@/features/channels/hooks";
import {
  useBriefQuery,
  useOnboardingQuery,
} from "@/features/cofounder/initiativeHooks";
import { useUsersBatchQuery } from "@/features/profile/hooks";
import { ProfileAvatar } from "@/features/profile/ui/ProfileAvatar";
import { SettingsSwitchRow } from "@/features/settings/ui/SettingsKit";
import { useAppNavigation } from "@/app/navigation/useAppNavigation";

import { useCofounderSettingsQuery, useSetCofounderSettings } from "../hooks";
import { useCofounderLeadStatusQuery } from "../leadStatusHooks";
import "./cofounderPhase2.css";

const SCOUT_STALL_AFTER_MS = 3 * 60 * 1000;

export function CofounderSettingRow() {
  const { activeCommunity } = useCommunities();
  const membershipQuery = useMyRelayMembershipLookupQuery();
  const settingsQuery = useCofounderSettingsQuery(activeCommunity?.id ?? null);
  const leadStatusQuery = useCofounderLeadStatusQuery(
    settingsQuery.data?.settings.enabled === true,
  );
  const setSettings = useSetCofounderSettings();
  const cofounderEnabled = settingsQuery.data?.settings.enabled === true;
  const onboardingQuery = useOnboardingQuery(cofounderEnabled);
  const briefQuery = useBriefQuery(cofounderEnabled);
  const onboardingHead = onboardingQuery.data?.head;
  const channelsQuery = useChannelsQuery({
    enabled: cofounderEnabled,
  });
  const openDm = useOpenDmMutation();
  const { goChannel } = useAppNavigation();
  const [now, setNow] = React.useState(() => Date.now());
  const role = membershipQuery.data?.membership?.role;
  const canChange = role === "owner" || role === "admin";
  const settings = settingsQuery.data?.settings;
  const companyName = activeCommunity?.name ?? "This company";
  const scoutPubkey = onboardingQuery.data?.head?.leadPubkey ?? null;
  const scoutProfileQuery = useUsersBatchQuery(
    scoutPubkey ? [scoutPubkey] : [],
    { enabled: cofounderEnabled && scoutPubkey !== null },
  );
  const scoutProfile = scoutPubkey
    ? scoutProfileQuery.data?.profiles[scoutPubkey.toLowerCase()]
    : undefined;
  const scoutChannel = channelsQuery.data?.find(
    (channel) =>
      channel.channelType === "dm" &&
      channel.participantPubkeys.some(
        (pubkey) => pubkey.toLowerCase() === scoutPubkey?.toLowerCase(),
      ),
  );
  const briefHead = briefQuery.data?.head;
  const onboardingStartedAt =
    onboardingHead?.resumedAt ?? onboardingHead?.startedAt ?? null;
  const showGettingToKnowCard =
    cofounderEnabled &&
    onboardingHead?.status === "in_progress" &&
    briefHead?.pending == null &&
    briefHead?.confirmed == null &&
    !onboardingQuery.isError &&
    !briefQuery.isError;
  React.useEffect(() => {
    if (!showGettingToKnowCard) return;
    setNow(Date.now());
    const timer = window.setInterval(() => setNow(Date.now()), 5_000);
    return () => window.clearInterval(timer);
  }, [showGettingToKnowCard]);
  const hasDurableWakeFailure =
    showGettingToKnowCard && leadStatusQuery.data?.state === "unreachable";
  const isScoutStalled =
    showGettingToKnowCard &&
    (hasDurableWakeFailure ||
      (onboardingStartedAt !== null &&
        now - onboardingStartedAt * 1_000 >= SCOUT_STALL_AFTER_MS));
  const disabled =
    !activeCommunity ||
    !canChange ||
    settingsQuery.isLoading ||
    settingsQuery.isError ||
    setSettings.isPending;

  async function changeEnabled(enabled: boolean) {
    if (!settings || !settingsQuery.data || !canChange) return;
    try {
      await setSettings.mutateAsync({
        settings: { ...settings, enabled },
        expectedHeadEventId: settingsQuery.data.eventId,
      });
    } catch {
      toast.error("Could not update co-founder mode.");
    }
  }

  async function openScout() {
    if (!scoutPubkey) return;
    try {
      const channel =
        scoutChannel ?? (await openDm.mutateAsync({ pubkeys: [scoutPubkey] }));
      await goChannel(channel.id);
    } catch {
      toast.error("Could not open Scout's direct message.");
    }
  }

  return (
    <>
      <SettingsSwitchRow
        checked={settings?.enabled ?? false}
        disabled={disabled}
        help={
          <>
            <span>
              Scout runs {companyName} with you as Chief of Staff. It plans
              work, hires and briefs agents, and asks before anything goes out,
              costs money or carries your brand.
            </span>
            <span className="mt-1 block">
              {companyName} only · Owner and admins can switch it
            </span>
          </>
        }
        id="cofounder-mode-toggle"
        label="Co-founder mode"
        onCheckedChange={(enabled) => void changeEnabled(enabled)}
        testId="cofounder-mode-toggle"
      />
      {showGettingToKnowCard ? (
        <div
          className="cf-setting-cofounder-scout"
          data-testid="cofounder-scout-entry"
        >
          <ProfileAvatar
            avatarUrl={scoutProfile?.avatarUrl ?? null}
            className="size-8 text-xs shadow-none"
            iconClassName="h-4 w-4"
            label="Scout"
            shape="squircle"
            testId="cofounder-scout-avatar"
          />
          <p>
            <strong>
              {isScoutStalled
                ? hasDurableWakeFailure
                  ? "Scout could not be reached"
                  : "Scout has not started yet"
                : `Scout is getting to know ${companyName}`}
            </strong>
            <span>
              {isScoutStalled
                ? hasDurableWakeFailure && leadStatusQuery.data?.detail
                  ? `${leadStatusQuery.data.detail} Try again to wake Scout and continue setup.`
                  : `It has been a few minutes since ${companyName} was switched on. Try again to wake Scout and continue setup.`
                : "Reading your website, leads, chats and connected tools. Scout will message you a short brief to check."}
            </span>
          </p>
          <button
            className="cf-setting-cofounder-scout-button"
            data-testid={
              isScoutStalled ? "cofounder-retry-scout" : "cofounder-open-scout"
            }
            disabled={
              isScoutStalled ? disabled : !scoutPubkey || openDm.isPending
            }
            onClick={() =>
              void (isScoutStalled ? changeEnabled(true) : openScout())
            }
            type="button"
          >
            {isScoutStalled ? "Try again" : "Open Scout"}
          </button>
        </div>
      ) : null}
    </>
  );
}
