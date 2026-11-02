import * as React from "react";
import { useQuery } from "@tanstack/react-query";

import { useCommunities } from "@/features/communities/useCommunities";
import { useMyRelayMembershipQuery } from "@/features/community-members/hooks";
import { usePresenceQuery } from "@/features/presence/hooks";
import {
  useProfileQuery,
  useUpdateProfileMutation,
} from "@/features/profile/hooks";
import {
  useSetUserStatusMutation,
  useUserStatusQuery,
  visibleUserStatus,
} from "@/features/user-status/hooks";
import { SetStatusDialog } from "@/features/user-status/ui/SetStatusDialog";
import {
  DEFAULT_USER_STATUS_EMOJI,
  StatusEmoji,
} from "@/features/user-status/ui/StatusEmoji";
import { getAccountAuthClient } from "@/features/onboarding/accountAuthAdapter";
import { WorkspaceMark } from "@/features/communities/ui/WorkspaceMark";
import { ProfileAvatar } from "@/features/profile/ui/ProfileAvatar";
import { useIdentityQuery } from "@/shared/api/hooks";
import {
  detectedTimeZone,
  readTimeZonePreference,
  timeZoneLabel,
  timeZoneOptions,
  writeTimeZonePreference,
} from "../lib/timezonePreference";
import type { SettingsSection } from "./SettingsPanels";
import { membershipRoleLabel } from "./membershipRoleLabel";
import { StatusFaceIcon } from "./settingsIcons";

type AccountProfileSettingsPanelProps = {
  avatarSaved?: boolean;
  fallbackDisplayName?: string;
  onEditAvatar?: () => void;
  onSectionChange: (section: SettingsSection) => void;
};

const accountQueryKey = ["settings-account-auth"] as const;

export function AccountProfileSettingsPanel({
  avatarSaved = false,
  fallbackDisplayName,
  onEditAvatar,
  onSectionChange,
}: AccountProfileSettingsPanelProps) {
  const profileQuery = useProfileQuery();
  const updateProfileMutation = useUpdateProfileMutation();
  const identityQuery = useIdentityQuery();
  const membershipQuery = useMyRelayMembershipQuery();
  const pubkey = identityQuery.data?.pubkey?.toLowerCase() ?? "";
  const statusQuery = useUserStatusQuery(pubkey ? [pubkey] : []);
  const presenceQuery = usePresenceQuery(pubkey ? [pubkey] : [], {
    enabled: pubkey.length > 0,
  });
  const setUserStatus = useSetUserStatusMutation(pubkey);
  const accountQuery = useQuery({
    queryKey: accountQueryKey,
    queryFn: () => getAccountAuthClient().getAccount(),
    retry: false,
    staleTime: 60_000,
  });
  const { activeCommunity } = useCommunities();
  const profileName =
    profileQuery.data?.displayName ?? fallbackDisplayName ?? "";
  const [nameDraft, setNameDraft] = React.useState(profileName);
  const [statusDialogOpen, setStatusDialogOpen] = React.useState(false);
  const dirtyRef = React.useRef(false);
  const savedZone = readTimeZonePreference(pubkey || undefined);
  const [zoneDraft, setZoneDraft] = React.useState(
    savedZone ?? detectedTimeZone(),
  );
  const [zoneSaveFailed, setZoneSaveFailed] = React.useState(false);
  React.useEffect(() => {
    setZoneDraft(savedZone ?? detectedTimeZone());
  }, [savedZone]);
  const zones = React.useMemo(() => timeZoneOptions(zoneDraft), [zoneDraft]);
  const status = visibleUserStatus(statusQuery.data?.[pubkey]);
  const presence = presenceQuery.data?.[pubkey] ?? "online";
  const membershipRole = membershipRoleLabel(membershipQuery.data?.role);
  const isLoading =
    profileQuery.isLoading ||
    identityQuery.isLoading ||
    membershipQuery.isLoading ||
    statusQuery.isLoading ||
    accountQuery.isLoading;

  React.useEffect(() => {
    if (!dirtyRef.current) setNameDraft(profileName);
  }, [profileName]);

  function saveProfile(event: React.FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const displayName = nameDraft.trim();
    if (!displayName) return;
    const zoneChanged = zoneDraft !== (savedZone ?? detectedTimeZone());
    if (zoneChanged) {
      setZoneSaveFailed(
        !writeTimeZonePreference(pubkey || undefined, zoneDraft),
      );
    }

    updateProfileMutation.mutate(
      { displayName },
      {
        onSuccess: () => {
          dirtyRef.current = false;
        },
      },
    );
  }

  const initials = profileName
    .split(/\s+/)
    .filter(Boolean)
    .slice(0, 2)
    .map((word) => word[0]?.toUpperCase())
    .join("");

  return (
    <div
      aria-busy={isLoading}
      className="cs-stack"
      data-ready={!isLoading ? "true" : "false"}
      data-testid="settings-profile"
    >
      <section
        aria-labelledby="account-profile-title"
        className="cs-card"
        data-testid="settings-account-profile-card"
      >
        <form
          aria-label="Your profile"
          className="cs-card-body"
          data-testid="account-profile-form"
          onSubmit={saveProfile}
        >
          <h2 className="cs-card-title" id="account-profile-title">
            Profile
          </h2>
          <div className="cs-photo-row">
            <span className="cs-avatar-large">
              <ProfileAvatar
                avatarUrl={profileQuery.data?.avatarUrl ?? null}
                className="cs-avatar-large-image"
                label={profileName || "Your profile"}
                pubkey={profileQuery.data?.pubkey}
                testId="account-profile-avatar"
              />
              <span
                aria-label={presence}
                className="cs-presence-dot"
                data-presence={presence}
                role="img"
              />
              <span className="sr-only">{initials}</span>
            </span>
            <div className="cs-photo-copy">
              <button
                className="cs-button"
                data-testid="profile-avatar-edit"
                onClick={onEditAvatar}
                type="button"
              >
                Change photo
              </button>
              <span className="cs-help">
                Shown beside your messages and on your profile.
              </span>
            </div>
          </div>

          <div className="cs-form-grid">
            <div className="cs-field">
              <label className="cs-label" htmlFor="account-profile-name">
                Name
              </label>
              <input
                autoComplete="name"
                className="cs-input"
                data-testid="profile-display-name"
                id="account-profile-name"
                onChange={(event) => {
                  dirtyRef.current = event.target.value.trim() !== profileName;
                  setNameDraft(event.target.value);
                }}
                onKeyDown={(event) => {
                  if (event.key === "Enter") {
                    event.preventDefault();
                    event.currentTarget.form?.requestSubmit();
                  }
                }}
                value={nameDraft}
              />
            </div>

            <div className="cs-field">
              <label className="cs-label" htmlFor="account-profile-email">
                Email
              </label>
              <input
                autoComplete="email"
                className="cs-input"
                data-testid="account-profile-email"
                id="account-profile-email"
                readOnly
                value={accountQuery.data?.email ?? ""}
              />
              <button
                className="cs-link"
                data-testid="account-profile-change-email"
                onClick={() => onSectionChange("security")}
                type="button"
              >
                Change your sign-in email
              </button>
            </div>

            <div className="cs-field">
              <span className="cs-label" id="account-profile-status-label">
                Status
              </span>
              <button
                aria-labelledby="account-profile-status-label account-profile-status"
                className="cs-status-field"
                data-has-status={status ? "true" : undefined}
                data-testid="account-profile-status"
                id="account-profile-status"
                onClick={() => setStatusDialogOpen(true)}
                type="button"
              >
                {status ? (
                  <StatusEmoji
                    className="size-4.5 shrink-0 text-base"
                    showTitle={false}
                    value={status.emoji || DEFAULT_USER_STATUS_EMOJI}
                  />
                ) : (
                  <StatusFaceIcon size={18} />
                )}
                <span>{status?.text || "Set a status"}</span>
              </button>
            </div>

            <div className="cs-field">
              <label className="cs-label" htmlFor="account-profile-timezone">
                Timezone
              </label>
              <select
                className="cs-select"
                data-testid="account-profile-timezone"
                id="account-profile-timezone"
                onChange={(event) => setZoneDraft(event.target.value)}
                value={zoneDraft}
              >
                {zones.map((zone) => (
                  <option key={zone} value={zone}>
                    {zone === zoneDraft ? timeZoneLabel(zone) : zone}
                  </option>
                ))}
              </select>
            </div>
          </div>

          <div className="cs-save-row">
            <button
              className="cs-primary"
              data-testid="account-profile-save"
              disabled={updateProfileMutation.isPending}
              type="submit"
            >
              Save
            </button>
            <span className="cs-help">
              Changes are visible to everyone in the business.
            </span>
          </div>
          {profileQuery.error instanceof Error ||
          updateProfileMutation.error instanceof Error ? (
            <p className="cs-notice" data-tone="bad" role="alert">
              {updateProfileMutation.error instanceof Error
                ? updateProfileMutation.error.message
                : profileQuery.error instanceof Error
                  ? profileQuery.error.message
                  : ""}
            </p>
          ) : null}
          {zoneSaveFailed ? (
            <p className="cs-notice" data-tone="bad" role="alert">
              Your time zone could not be saved on this computer.
            </p>
          ) : null}
          {accountQuery.error instanceof Error ? (
            <p className="cs-notice" data-tone="bad" role="alert">
              We couldn't load your account details. Try again.{" "}
              <button
                className="cs-link"
                disabled={accountQuery.isFetching}
                onClick={() => void accountQuery.refetch()}
                type="button"
              >
                Retry
              </button>
            </p>
          ) : null}
          {avatarSaved ? (
            <p
              className="cs-notice"
              data-testid="profile-avatar-saved"
              data-tone="ok"
              role="status"
            >
              Profile photo updated
            </p>
          ) : null}
        </form>
      </section>

      <section
        aria-labelledby="account-business-title"
        className="cs-card cs-business-card"
        data-testid="settings-account-business-card"
      >
        <WorkspaceMark
          className="cs-business-mark"
          communityId={activeCommunity?.id}
          fallbackText={activeCommunity ? undefined : ""}
          name={activeCommunity?.name ?? ""}
          relayUrl={activeCommunity?.relayUrl}
        />
        <div className="cs-business-copy">
          <h2 className="cs-eyebrow" id="account-business-title">
            This business
          </h2>
          <strong>{activeCommunity?.name ?? ""}</strong>
          <span>{membershipRole ? `Your role · ${membershipRole}` : ""}</span>
        </div>
        <div className="cs-business-actions">
          <button
            className="cs-button"
            data-testid="settings-account-business-settings"
            onClick={() => onSectionChange("business-profile")}
            type="button"
          >
            Business settings
          </button>
          <button
            className="cs-button"
            data-testid="settings-account-members-roles"
            onClick={() => onSectionChange("people")}
            type="button"
          >
            Members &amp; roles
          </button>
        </div>
      </section>

      <SetStatusDialog
        hasExistingStatus={Boolean(status)}
        initialEmoji={status?.emoji}
        initialExpiresAt={status?.expiresAt}
        initialText={status?.text}
        initialUpdatedAt={status?.updatedAt}
        onClear={() =>
          setUserStatus.mutate({ text: "", emoji: "", expiresAt: undefined })
        }
        onOpenChange={setStatusDialogOpen}
        onSave={(next) => setUserStatus.mutate(next)}
        open={statusDialogOpen}
      />
    </div>
  );
}
