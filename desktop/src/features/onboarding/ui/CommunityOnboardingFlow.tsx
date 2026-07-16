import * as React from "react";
import { useQueryClient } from "@tanstack/react-query";
import { Bot, Users } from "lucide-react";

import {
  markCommunityOnboardingComplete,
  useCommunityOnboarding,
} from "@/features/onboarding/communityOnboarding";
import { initializeWelcomeChannel } from "@/features/onboarding/hooks";
import { AvatarUpload } from "@/features/profile/ui/AvatarUpload";
import { ProfileAvatar } from "@/features/profile/ui/ProfileAvatar";
import { claimInvite } from "@/shared/api/invites";
import { inviteErrorMessage } from "@/shared/api/inviteHelpers";
import { updateProfile } from "@/shared/api/tauriProfiles";
import { getIdentity } from "@/shared/api/tauriIdentity";
import { listPersonas } from "@/shared/api/tauriPersonas";
import type { AgentPersona } from "@/shared/api/types";
import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";
import { StartupWindowDragRegion } from "@/shared/ui/StartupWindowDragRegion";

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

  React.useEffect(() => {
    if (transaction?.stage !== "claiming" || isPending) return;
    setIsPending(true);
    void claimInvite(transaction.relayUrl, transaction.inviteCode ?? "")
      .then(() => update({ stage: "connecting", error: undefined }))
      .catch((error: unknown) => update({ error: inviteErrorMessage(error) }))
      .finally(() => setIsPending(false));
  }, [isPending, transaction, update]);

  React.useEffect(() => {
    if (transaction?.stage !== "connecting") return;
    onConnect();
  }, [onConnect, transaction]);

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
      const result = await initializeWelcomeChannel(queryClient, {
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

  return (
    <div
      className="buzz-onboarding-neutral-theme buzz-startup-shell flex items-center justify-center bg-background px-4 py-8 text-foreground"
      data-testid="community-onboarding-flow"
    >
      <StartupWindowDragRegion />
      <div className="w-full max-w-[440px] text-center">
        {transaction.stage === "claiming" ||
        transaction.stage === "connecting" ? (
          <>
            <Users className="mx-auto h-10 w-10" />
            <h1 className="mt-5 text-3xl font-semibold">
              Joining {transaction.communityName}
            </h1>
            <p className="mt-3 text-sm text-muted-foreground">
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
            <h1 className="text-3xl font-semibold">
              How should you appear here?
            </h1>
            <p className="mt-3 text-sm text-muted-foreground">
              Your name and avatar are specific to {transaction.communityName}.
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
                <p className="text-sm text-destructive">{transaction.error}</p>
              ) : null}
              <Button
                className="w-full"
                disabled={!displayName.trim() || isPending || isUploadingAvatar}
                onClick={() => void saveProfile()}
              >
                Continue
              </Button>
            </div>
          </>
        ) : (
          <>
            <Bot className="mx-auto h-10 w-10" />
            <h1 className="mt-5 text-3xl font-semibold">
              Meet your starter team
            </h1>
            <p className="mt-3 text-sm leading-6 text-muted-foreground">
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
            <div className="mt-8 flex flex-col gap-3">
              <Button
                className="w-full"
                disabled={isPending}
                onClick={() => void finalize()}
              >
                {transaction.stage === "finalizing"
                  ? "Preparing Welcome…"
                  : `Enter ${transaction.communityName}`}
              </Button>
              {transaction.error ? (
                <Button
                  disabled={isPending}
                  onClick={() => void finish()}
                  variant="ghost"
                >
                  Enter without Welcome channel
                </Button>
              ) : null}
            </div>
          </>
        )}
      </div>
    </div>
  );
}
