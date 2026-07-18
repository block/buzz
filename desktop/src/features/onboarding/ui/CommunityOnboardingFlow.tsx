import * as React from "react";
import { useQueryClient } from "@tanstack/react-query";
import { Plus, Users, X } from "lucide-react";

import {
  markCommunityOnboardingComplete,
  useCommunityOnboarding,
} from "@/features/onboarding/communityOnboarding";
import { initializeStarterChannels } from "@/features/onboarding/hooks";
import { useClaimInvite } from "@/features/onboarding/useClaimInvite";
import { CommunityChangeOverlay } from "@/features/communities/ui/CommunityChangeOverlay";
import {
  takePendingWelcomeChannelForDirectEntry,
  WELCOME_SURFACE_READY_EVENT,
} from "@/features/onboarding/welcome";
import { profileQueryKey } from "@/features/profile/hooks";
import { ProfileAvatar } from "@/features/profile/ui/ProfileAvatar";
import {
  parseEmojiAvatarDataUrl,
  ProfileAvatarEditor,
} from "@/features/profile/ui/ProfileAvatarEditor";
import { updateProfile } from "@/shared/api/tauriProfiles";
import { getIdentity, importIdentity } from "@/shared/api/tauriIdentity";
import { listPersonas } from "@/shared/api/tauriPersonas";
import { relayClient } from "@/shared/api/relayClient";
import type { AgentPersona } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import { MembershipDenied } from "./MembershipDenied";
import { StartupWindowDragRegion } from "@/shared/ui/StartupWindowDragRegion";
import {
  ONBOARDING_PRIMARY_CTA_CLASS,
  OnboardingChrome,
} from "./OnboardingChrome";
import { OnboardingFooter, OnboardingFooterProvider } from "./OnboardingFooter";
import { OnboardingStepHeader } from "./OnboardingStepHeader";
import {
  ONBOARDING_KEY_FRAME_CLASS,
  ONBOARDING_KEY_ROW_CLASS,
  ONBOARDING_KEY_TEXT_CLASS,
} from "./NsecMaskedDisplay";

function isRelayMembershipDeniedError(error: unknown): boolean {
  if (!(error instanceof Error)) return false;
  return (
    error.message.includes("You must be a relay member") ||
    error.message.includes("relay_membership_required") ||
    error.message.includes("restricted: not a relay member") ||
    error.message.includes("invalid: you are not a relay member")
  );
}

const STARTER_PERSONA_ANIMATIONS: Record<string, string> = {
  Fizz: "/onboarding/starter-team/fizz.png",
  Honey: "/onboarding/starter-team/honey.png",
  Bumble: "/onboarding/starter-team/bumble.png",
};

/** Fade duration for the "entering" curtain over the mounting app. */
const ENTERING_CURTAIN_FADE_MS = 500;
/**
 * Safety valve: if Welcome never reports ready (slow relay, failed query),
 * fade anyway rather than stranding the user on the onboarding screen.
 */
const ENTERING_CURTAIN_MAX_WAIT_MS = 8_000;

const NEUTRAL_EMOJI_PICKER_THEME_VARS = {
  "--buzz-emoji-picker-rgb-background":
    "var(--buzz-onboarding-emoji-picker-background)",
  "--buzz-emoji-picker-rgb-color": "var(--buzz-onboarding-emoji-picker-color)",
  "--buzz-emoji-picker-rgb-input": "var(--buzz-onboarding-emoji-picker-input)",
} as React.CSSProperties;

function AvatarCircle({
  avatarUrl,
  onClick,
  previewName,
}: {
  avatarUrl: string;
  onClick: () => void;
  previewName: string;
}) {
  const emojiAvatar = parseEmojiAvatarDataUrl(avatarUrl);
  const hasAvatar = avatarUrl.trim().length > 0;

  return (
    <button
      aria-label={hasAvatar ? "Change your avatar" : "Add an avatar"}
      className="group block shrink-0 rounded-full"
      data-testid="community-avatar-open"
      onClick={onClick}
      type="button"
    >
      {emojiAvatar ? (
        <span
          className="flex h-28 w-28 items-center justify-center overflow-hidden rounded-full text-5xl shadow-xs"
          style={{ backgroundColor: emojiAvatar.color }}
        >
          {emojiAvatar.emoji}
        </span>
      ) : hasAvatar ? (
        <ProfileAvatar
          avatarUrl={avatarUrl}
          className="h-28 w-28 rounded-full text-3xl"
          label={previewName}
        />
      ) : (
        <span className="flex h-28 w-28 items-center justify-center rounded-full text-[var(--buzz-onboarding-backup-ink)] transition-colors group-hover:bg-white/25">
          <Plus className="h-8 w-8" aria-hidden="true" />
        </span>
      )}
    </button>
  );
}

export function CommunityOnboardingFlow({
  onCancel,
  onConnect,
}: {
  onCancel: () => void;
  onConnect: () => void;
}) {
  const { transaction, update, clear } = useCommunityOnboarding();
  const queryClient = useQueryClient();
  const [displayName, setDisplayName] = React.useState("");
  const [avatarUrl, setAvatarUrl] = React.useState("");
  const [isUploadingAvatar, setIsUploadingAvatar] = React.useState(false);
  const [isAvatarEditorOpen, setIsAvatarEditorOpen] = React.useState(false);
  const [starterPersonas, setStarterPersonas] = React.useState<AgentPersona[]>(
    [],
  );
  const [isPending, setIsPending] = React.useState(false);
  const [deniedPubkey, setDeniedPubkey] = React.useState("");
  const [isMembershipDenied, setIsMembershipDenied] = React.useState(false);
  const [isCommunityChangeOpen, setIsCommunityChangeOpen] =
    React.useState(false);
  const [isCurtainFading, setIsCurtainFading] = React.useState(false);
  const nameInputRef = React.useRef<HTMLInputElement | null>(null);

  // Also fetch on "entering": the curtain is a fresh mount of this component,
  // so the team-intro fetch from the pre-curtain instance isn't in this state.
  const isTeamIntroVisible =
    transaction?.stage === "team-intro" ||
    transaction?.stage === "finalizing" ||
    transaction?.stage === "entering";
  React.useEffect(() => {
    if (!isTeamIntroVisible) return;
    void listPersonas()
      .then((personas) =>
        setStarterPersonas(
          ["Fizz", "Honey", "Bumble"].flatMap((name) => {
            const persona = personas.find(
              (candidate) => candidate.displayName === name,
            );
            return persona ? [persona] : [];
          }),
        ),
      )
      .catch(() => setStarterPersonas([]));
  }, [isTeamIntroVisible]);

  useClaimInvite();

  React.useEffect(() => {
    if (transaction?.stage === "connecting") onConnect();
  }, [onConnect, transaction?.stage]);

  // "Entering" curtain: the app is mounting on the Welcome route underneath.
  // Fade out when Welcome reports its first settled render — or after a
  // safety timeout so a slow load can never strand the user on this screen.
  const isEnteringStage = transaction?.stage === "entering";
  React.useEffect(() => {
    if (!isEnteringStage) return;

    let fadeTimer: number | null = null;
    const beginFade = () => {
      if (fadeTimer !== null) return;
      setIsCurtainFading(true);
      fadeTimer = window.setTimeout(() => {
        clear();
      }, ENTERING_CURTAIN_FADE_MS);
    };

    window.addEventListener(WELCOME_SURFACE_READY_EVENT, beginFade);
    const safetyTimer = window.setTimeout(
      beginFade,
      ENTERING_CURTAIN_MAX_WAIT_MS,
    );
    return () => {
      window.removeEventListener(WELCOME_SURFACE_READY_EVENT, beginFade);
      window.clearTimeout(safetyTimer);
      if (fadeTimer !== null) window.clearTimeout(fadeTimer);
    };
  }, [clear, isEnteringStage]);

  const retry = () =>
    update({
      stage: transaction?.inviteCode ? "claiming" : "connecting",
      error: undefined,
    });
  const relayUrl = transaction?.relayUrl;
  const finish = React.useCallback(async () => {
    if (!relayUrl) return;
    const identity = await getIdentity();
    markCommunityOnboardingComplete(identity.pubkey, relayUrl);
    clear();
  }, [clear, relayUrl]);
  const finalize = React.useCallback(async () => {
    if (isPending || !relayUrl) return;
    setIsPending(true);
    update({ stage: "finalizing", error: undefined });
    try {
      const identity = await getIdentity();
      const result = await initializeStarterChannels(queryClient, {
        focus: true,
        pubkey: identity.pubkey,
        communityScope: relayUrl,
      });
      if (!result.ok) throw new Error(result.reason);
      if (result.focusChannelId) {
        // Direct entry: point the router at the Welcome channel *before* the
        // app mounts, so it never lands on Home first. Consume the pending
        // entry — it exists for the Home-route fallback, and leaving it would
        // yank a later Home visit back to Welcome.
        takePendingWelcomeChannelForDirectEntry();
        window.location.hash = `/channels/${result.focusChannelId}`;
        markCommunityOnboardingComplete(identity.pubkey, relayUrl);
        // Keep this screen mounted as a curtain over the loading app; the
        // "entering" stage fades it out once Welcome reports ready.
        update({ stage: "entering", error: undefined });
        return;
      }
      await finish();
    } catch (error) {
      update({
        stage: "team-intro",
        error: error instanceof Error ? error.message : String(error),
      });
      setIsPending(false);
    }
  }, [finish, isPending, queryClient, relayUrl, update]);
  const backToProfile = React.useCallback(() => {
    if (isPending) return;
    update({ stage: "profile", error: undefined });
  }, [isPending, update]);

  const isProfileStage = transaction?.stage === "profile";
  const isTeamStage =
    transaction?.stage === "team-intro" ||
    transaction?.stage === "finalizing" ||
    transaction?.stage === "entering";

  React.useLayoutEffect(() => {
    if (isProfileStage && !isAvatarEditorOpen) {
      nameInputRef.current?.focus();
    }
  }, [isAvatarEditorOpen, isProfileStage]);

  if (!transaction) return null;

  if (isMembershipDenied) {
    return (
      <>
        <MembershipDenied
          activeRelayUrl={transaction.relayUrl}
          onBack={() => setIsMembershipDenied(false)}
          onChangeCommunity={() => setIsCommunityChangeOpen(true)}
          onImportKey={async (nsec) => {
            const identity = await importIdentity(nsec);
            relayClient.disconnect();
            queryClient.setQueryData(["identity"], identity);
            queryClient.removeQueries({ queryKey: profileQueryKey });
            setIsMembershipDenied(false);
            update({ stage: "connecting", error: undefined });
          }}
          onRetry={() => {
            setIsMembershipDenied(false);
            update({ stage: "connecting", error: undefined });
          }}
          pubkey={deniedPubkey}
        />
        {isCommunityChangeOpen ? (
          <CommunityChangeOverlay
            onClose={() => setIsCommunityChangeOpen(false)}
            onUpdated={(communityName, updatedRelayUrl) => {
              update({
                communityName,
                relayUrl: updatedRelayUrl,
                stage: "connecting",
                error: undefined,
              });
              setIsMembershipDenied(false);
            }}
          />
        ) : null}
      </>
    );
  }

  const saveProfile = async () => {
    if (!displayName.trim()) return;
    setIsPending(true);
    try {
      await updateProfile({
        displayName: displayName.trim(),
        avatarUrl: avatarUrl.trim() || undefined,
      });
      update({ stage: "team-intro", error: undefined });
    } catch (error) {
      if (isRelayMembershipDeniedError(error)) {
        try {
          const identity = await getIdentity();
          setDeniedPubkey(identity.pubkey);
        } catch {
          setDeniedPubkey("");
        }
        setIsMembershipDenied(true);
        return;
      }
      update({ error: error instanceof Error ? error.message : String(error) });
    } finally {
      setIsPending(false);
    }
  };

  return (
    <div
      className={cn(
        "buzz-onboarding-neutral-theme buzz-startup-shell flex h-dvh justify-center overflow-y-auto px-4 text-foreground",
        isProfileStage || isTeamStage
          ? "items-start pb-36 pt-[106px]"
          : "items-stretch",
        isCurtainFading &&
          "pointer-events-none opacity-0 transition-opacity ease-out motion-reduce:transition-none",
      )}
      data-testid="community-onboarding-flow"
      style={
        isCurtainFading
          ? { transitionDuration: `${ENTERING_CURTAIN_FADE_MS}ms` }
          : undefined
      }
    >
      <StartupWindowDragRegion />
      {isProfileStage || isTeamStage ? (
        <OnboardingChrome current={isTeamStage ? 7 : 6} />
      ) : null}
      <OnboardingFooterProvider>
        <div
          className={cn(
            "relative w-full text-center",
            isProfileStage || isTeamStage
              ? "buzz-onboarding-step-frame flex flex-col items-center"
              : "flex min-h-dvh flex-col justify-center py-8",
            isProfileStage
              ? "max-w-4xl"
              : isTeamStage
                ? "max-w-[760px]"
                : "max-w-[560px]",
          )}
          data-testid="community-onboarding-body"
        >
          {transaction.stage === "claiming" ||
          transaction.stage === "connecting" ? (
            <>
              <Users className="mx-auto h-10 w-10" />
              <h1 className="mt-5 text-title font-normal">
                Joining {transaction.communityName}
              </h1>
              <p className="mt-3 text-sm text-foreground/80">
                {transaction.error ??
                  (transaction.stage === "claiming"
                    ? "Accepting your invite…"
                    : "Connecting securely…")}
              </p>
              <div className="mt-6 flex justify-center gap-3">
                {transaction.error ? (
                  <Button className="rounded-full px-6" onClick={retry}>
                    Retry
                  </Button>
                ) : null}
                <Button
                  className="rounded-full bg-foreground/10 px-5 hover:bg-foreground/15"
                  onClick={onCancel}
                  variant="ghost"
                >
                  Cancel
                </Button>
              </div>
            </>
          ) : isProfileStage ? (
            isAvatarEditorOpen ? (
              <div
                className={cn("relative", ONBOARDING_KEY_FRAME_CLASS)}
                data-testid="community-avatar-editor-key-frame"
              >
                <Button
                  aria-label="Close avatar editor"
                  className="absolute -right-3 -top-3 h-9 w-9 rounded-full"
                  data-testid="community-avatar-close"
                  onClick={() => setIsAvatarEditorOpen(false)}
                  size="icon"
                  type="button"
                >
                  <X className="h-4 w-4" />
                </Button>
                <ProfileAvatarEditor
                  avatarUrl={avatarUrl}
                  disabled={isPending}
                  emojiPickerTheme="auto"
                  emojiPickerThemeVars={NEUTRAL_EMOJI_PICKER_THEME_VARS}
                  onDone={() => setIsAvatarEditorOpen(false)}
                  onUploadingChange={setIsUploadingAvatar}
                  onUrlChange={setAvatarUrl}
                  previewName={displayName.trim() || "Your profile"}
                  testIdPrefix="community-avatar"
                />
              </div>
            ) : (
              <>
                <div data-testid="community-profile-main">
                  <OnboardingStepHeader
                    description={
                      <>
                        Add a name and avatar. They’ll show up on your messages,
                        reactions, and agent handoffs.
                      </>
                    }
                    descriptionClassName="max-w-[380px]"
                    title="Build your profile"
                  />
                  <div className="mt-10 w-full max-w-4xl">
                    <div
                      className={ONBOARDING_KEY_FRAME_CLASS}
                      data-testid="community-profile-key-frame"
                    >
                      <div className={ONBOARDING_KEY_ROW_CLASS}>
                        <AvatarCircle
                          avatarUrl={avatarUrl}
                          onClick={() => setIsAvatarEditorOpen(true)}
                          previewName={displayName.trim() || "Your profile"}
                        />
                        <label
                          className="min-w-0 flex-1"
                          htmlFor="community-display-name"
                        >
                          <span className="sr-only">Your name</span>
                          <input
                            aria-label="Community display name"
                            autoCapitalize="words"
                            autoComplete="name"
                            autoCorrect="off"
                            className={cn(
                              ONBOARDING_KEY_TEXT_CLASS,
                              "border-0 bg-transparent p-0 shadow-none outline-none placeholder:text-[var(--buzz-onboarding-backup-ink)] placeholder:opacity-40 focus-visible:outline-none disabled:cursor-not-allowed disabled:opacity-50",
                            )}
                            data-testid="community-profile-name-key"
                            disabled={isPending || isUploadingAvatar}
                            id="community-display-name"
                            onChange={(event) =>
                              setDisplayName(event.target.value)
                            }
                            placeholder="First and last name"
                            ref={nameInputRef}
                            spellCheck={false}
                            type="text"
                            value={displayName}
                          />
                        </label>
                      </div>
                    </div>
                  </div>
                  {transaction.error ? (
                    <p className="mt-4 text-sm text-destructive">
                      {transaction.error}
                    </p>
                  ) : null}
                </div>
                <OnboardingFooter>
                  <Button
                    className={ONBOARDING_PRIMARY_CTA_CLASS}
                    data-testid="community-profile-next"
                    disabled={
                      !displayName.trim() || isPending || isUploadingAvatar
                    }
                    onClick={() => void saveProfile()}
                    type="button"
                  >
                    Next
                  </Button>
                </OnboardingFooter>
              </>
            )
          ) : (
            <>
              <OnboardingStepHeader
                description={
                  <>
                    Buzz lets you bring multiple agents into the same workspace.
                    Your team will help you get started using Buzz.
                  </>
                }
                descriptionClassName="max-w-[400px]"
                title="Meet your starter team"
              />
              <div className="flex w-full flex-1 items-center justify-center py-10">
                {starterPersonas.length > 0 ? (
                  <div className="flex flex-wrap justify-center gap-8">
                    {starterPersonas.map((persona) => {
                      const animationUrl =
                        STARTER_PERSONA_ANIMATIONS[persona.displayName];
                      return (
                        <div
                          className="flex w-40 flex-col items-center gap-3"
                          key={persona.id}
                        >
                          {animationUrl ? (
                            <img
                              alt={`${persona.displayName} animated character`}
                              className="h-40 w-40 object-contain"
                              data-testid={`starter-persona-${persona.displayName.toLowerCase()}`}
                              src={animationUrl}
                            />
                          ) : (
                            <ProfileAvatar
                              avatarUrl={persona.avatarUrl}
                              className="h-28 w-28 text-3xl"
                              label={persona.displayName}
                            />
                          )}
                          <span className="font-mono text-xs font-medium uppercase tracking-[0.15em]">
                            {persona.displayName}
                          </span>
                        </div>
                      );
                    })}
                  </div>
                ) : null}
              </div>
              {transaction.error ? (
                <p className="text-sm text-destructive">{transaction.error}</p>
              ) : null}
              <OnboardingFooter>
                <Button
                  className={ONBOARDING_PRIMARY_CTA_CLASS}
                  data-testid="community-team-intro-enter"
                  disabled={isPending || transaction.stage === "entering"}
                  onClick={() => void finalize()}
                >
                  {isPending || transaction.stage === "entering"
                    ? "Preparing Welcome…"
                    : "Take me to Buzz"}
                </Button>
                <Button
                  className="h-9 rounded-full bg-foreground/10 px-5 hover:bg-foreground/15"
                  data-testid="community-team-intro-back"
                  disabled={isPending || transaction.stage === "entering"}
                  onClick={backToProfile}
                  variant="ghost"
                >
                  Back
                </Button>
              </OnboardingFooter>
            </>
          )}
        </div>
      </OnboardingFooterProvider>
    </div>
  );
}
