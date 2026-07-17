import * as React from "react";
import { useQueryClient } from "@tanstack/react-query";
import { Bot, Users } from "lucide-react";

import {
  markCommunityOnboardingComplete,
  useCommunityOnboarding,
} from "@/features/onboarding/communityOnboarding";
import { initializeStarterChannels } from "@/features/onboarding/hooks";
import { useClaimInvite } from "@/features/onboarding/useClaimInvite";
import { AvatarUpload } from "@/features/profile/ui/AvatarUpload";
import { ProfileAvatar } from "@/features/profile/ui/ProfileAvatar";
import { updateProfile } from "@/shared/api/tauriProfiles";
import { getIdentity } from "@/shared/api/tauriIdentity";
import { listPersonas } from "@/shared/api/tauriPersonas";
import type { AgentPersona } from "@/shared/api/types";
import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";
import { StartupWindowDragRegion } from "@/shared/ui/StartupWindowDragRegion";
import {
  ONBOARDING_PRIMARY_CTA_CLASS,
  OnboardingChrome,
  TOTAL_ONBOARDING_PAGES,
} from "./OnboardingChrome";
import { OnboardingFooter, OnboardingFooterProvider } from "./OnboardingFooter";

export function CommunityOnboardingFlow({
  onConnect,
}: {
  onConnect: () => void;
}) {
  const { transaction, update, clear } = useCommunityOnboarding();
  const queryClient = useQueryClient();
  const [displayName, setDisplayName] = React.useState("");
  const [avatarUrl, setAvatarUrl] = React.useState("");
  const [isUploadingAvatar, setIsUploadingAvatar] = React.useState(false);
  const [starterPersonas, setStarterPersonas] = React.useState<AgentPersona[]>(
    [],
  );
  const [isPending, setIsPending] = React.useState(false);

  React.useEffect(() => {
    if (transaction?.stage !== "team-intro") return;
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
  }, [transaction?.stage]);

  useClaimInvite();

  React.useEffect(() => {
    if (transaction?.stage === "connecting") onConnect();
  }, [onConnect, transaction?.stage]);

  const retryClaim = () => update({ stage: "claiming", error: undefined });
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
      await finish();
    } catch (error) {
      update({
        error: error instanceof Error ? error.message : String(error),
      });
      setIsPending(false);
    }
  }, [finish, isPending, queryClient, relayUrl, update]);

  if (!transaction) return null;

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
      update({ error: error instanceof Error ? error.message : String(error) });
    } finally {
      setIsPending(false);
    }
  };

  // Community join is the tail of the 6-page journey: profile is page 5, the
  // starter-team page is page 6. That page renders for both "team-intro" and
  // the "finalizing" transient, so mirror the render condition here — only the
  // claiming/connecting/profile states sit on page 5. (Keying off "team-intro"
  // alone made the dots regress 6→5 while "Preparing Welcome…" was shown.)
  const communityStep =
    transaction.stage === "claiming" ||
    transaction.stage === "connecting" ||
    transaction.stage === "profile"
      ? 5
      : 6;

  return (
    <div
      className="buzz-onboarding-neutral-theme buzz-startup-shell flex items-start justify-center overflow-y-auto bg-background px-4 pb-28 pt-[106px] text-foreground"
      data-testid="community-onboarding-flow"
    >
      <StartupWindowDragRegion />
      <OnboardingChrome
        current={communityStep}
        total={TOTAL_ONBOARDING_PAGES}
      />
      <OnboardingFooterProvider>
        <div className="w-full max-w-[440px] text-center">
          {transaction.stage === "claiming" ||
          transaction.stage === "connecting" ? (
            <>
              <Users className="mx-auto h-10 w-10" />
              <h1 className="mt-5 text-title font-normal">
                Joining {transaction.communityName}
              </h1>
              <p className="mt-5 text-sm text-muted-foreground">
                {transaction.error ??
                  (transaction.stage === "claiming"
                    ? "Accepting your invite…"
                    : "Connecting securely…")}
              </p>
              <div className="mt-6 flex justify-center gap-3">
                {transaction.error ? (
                  <Button onClick={retryClaim}>Retry</Button>
                ) : null}
                <Button onClick={clear} variant="secondary">
                  Cancel
                </Button>
              </div>
            </>
          ) : transaction.stage === "profile" ? (
            <>
              <h1 className="text-title font-normal">
                How should you appear here?
              </h1>
              <p className="mt-5 text-sm text-muted-foreground">
                Your name and avatar are specific to {transaction.communityName}
                .
              </p>
              <div className="mt-8 space-y-3 text-left">
                <Input
                  aria-label="Community display name"
                  autoFocus
                  onChange={(event) => setDisplayName(event.target.value)}
                  placeholder="Your name"
                  value={displayName}
                />
                <AvatarUpload
                  avatarUrl={avatarUrl}
                  disabled={isPending}
                  onClear={() => setAvatarUrl("")}
                  onUploadingChange={setIsUploadingAvatar}
                  onUrlChange={setAvatarUrl}
                  previewName={displayName.trim() || "Your profile"}
                  showClear={avatarUrl.length > 0}
                  testIdPrefix="community-avatar"
                />
                {transaction.error ? (
                  <p className="text-sm text-destructive">
                    {transaction.error}
                  </p>
                ) : null}
              </div>
              <OnboardingFooter>
                <Button
                  className={ONBOARDING_PRIMARY_CTA_CLASS}
                  disabled={
                    !displayName.trim() || isPending || isUploadingAvatar
                  }
                  onClick={() => void saveProfile()}
                >
                  Continue
                </Button>
              </OnboardingFooter>
            </>
          ) : (
            <>
              <Bot className="mx-auto h-10 w-10" />
              <h1 className="mt-5 text-title font-normal">
                Meet your starter team
              </h1>
              <p className="mt-5 text-sm leading-6 text-muted-foreground">
                Fizz helps you build, Honey helps you communicate, and Bumble
                helps you research. They’ll be ready when you need them.
              </p>
              {starterPersonas.length > 0 ? (
                <div className="mt-7 flex justify-center gap-5">
                  {starterPersonas.map((persona) => (
                    <div
                      className="flex w-20 flex-col items-center gap-2"
                      key={persona.id}
                    >
                      <ProfileAvatar
                        avatarUrl={persona.avatarUrl}
                        className="h-14 w-14"
                        label={persona.displayName}
                      />
                      <span className="text-sm font-medium">
                        {persona.displayName}
                      </span>
                    </div>
                  ))}
                </div>
              ) : null}
              {transaction.error ? (
                <p className="mt-4 text-sm text-destructive">
                  {transaction.error}
                </p>
              ) : null}
              <OnboardingFooter>
                <Button
                  className={ONBOARDING_PRIMARY_CTA_CLASS}
                  disabled={isPending}
                  onClick={() => void finalize()}
                >
                  {transaction.stage === "finalizing"
                    ? "Preparing Welcome…"
                    : `Enter ${transaction.communityName}`}
                </Button>
                {transaction.error ? (
                  <Button
                    className="h-10 w-full text-muted-foreground hover:text-accent-foreground"
                    disabled={isPending}
                    onClick={() => void finish()}
                    variant="ghost"
                  >
                    Skip for now
                  </Button>
                ) : null}
              </OnboardingFooter>
            </>
          )}
        </div>
      </OnboardingFooterProvider>
    </div>
  );
}
