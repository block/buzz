import * as React from "react";
import type { QueryClient } from "@tanstack/react-query";

import {
  getIdentity,
  importIdentity,
  persistCurrentIdentity,
} from "@/shared/api/tauriIdentity";
import { Button } from "@/shared/ui/button";
import { StartupWindowDragRegion } from "@/shared/ui/StartupWindowDragRegion";
import { BackupStep } from "./BackupStep";
import { LandingBees } from "./LandingBees";
import { NostrKeyImportForm } from "./NostrKeyImportForm";
import { OnboardingSlideTransition } from "./OnboardingSlideTransition";
import { OnboardingStepDots } from "./OnboardingStepDots";
import { SetupStep } from "./SetupStep";

type MachinePage = "identity" | "key-import" | "backup" | "setup";

export function MachineOnboardingFlow({
  complete,
  identityLost,
  queryClient,
}: {
  complete: (pubkey?: string) => void;
  identityLost: boolean;
  queryClient: QueryClient;
}) {
  const [page, setPage] = React.useState<MachinePage>(
    identityLost ? "key-import" : "identity",
  );
  const [error, setError] = React.useState<string | null>(null);
  const [isPending, setIsPending] = React.useState(false);
  const [identityWasImported, setIdentityWasImported] = React.useState(false);
  const [selectedPubkey, setSelectedPubkey] = React.useState<string | null>(
    null,
  );

  const loadFreshIdentity = React.useCallback(async () => {
    setIsPending(true);
    setError(null);
    try {
      const identity = await getIdentity();
      queryClient.setQueryData(["identity"], identity);
      setSelectedPubkey(identity.pubkey);
      setPage("backup");
    } catch (cause) {
      setError(
        cause instanceof Error ? cause.message : "Failed to load identity",
      );
    } finally {
      setIsPending(false);
    }
  }, [queryClient]);

  const replaceLostIdentity = React.useCallback(async () => {
    const confirmed = window.confirm(
      "This will create a new identity and abandon your previous key. This cannot be undone. Continue?",
    );
    if (!confirmed) return;

    setIsPending(true);
    setError(null);
    try {
      const identity = await persistCurrentIdentity();
      queryClient.setQueryData(["identity"], identity);
      setSelectedPubkey(identity.pubkey);
      setPage("backup");
    } catch (cause) {
      setError(
        cause instanceof Error ? cause.message : "Failed to save identity",
      );
    } finally {
      setIsPending(false);
    }
  }, [queryClient]);

  const importExistingIdentity = React.useCallback(
    async (nsec: string) => {
      const identity = await importIdentity(nsec);
      queryClient.setQueryData(["identity"], identity);
      setIdentityWasImported(true);
      setSelectedPubkey(identity.pubkey);
      setPage("setup");
    },
    [queryClient],
  );

  return (
    <div
      className={`buzz-onboarding-neutral-theme buzz-startup-shell flex max-h-dvh items-start justify-center overflow-y-auto px-4 text-foreground ${
        page === "setup" ? "py-24" : "py-8"
      } ${page === "identity" ? "buzz-onboarding-welcome" : ""}`}
      data-testid="machine-onboarding-gate"
    >
      <StartupWindowDragRegion />
      {page === "identity" ? <LandingBees /> : null}
      {page !== "identity" ? (
        <OnboardingStepDots current={page === "setup" ? 3 : 2} />
      ) : null}
      <div className="relative my-auto flex w-full max-w-[920px] flex-col items-center text-center">
        {page === "identity" ? (
          <OnboardingSlideTransition
            className="flex w-full max-w-[720px] flex-col items-center text-center"
            direction="forward"
            effect="mask-reveal-up"
            transitionKey="machine-identity"
          >
            <img
              alt="Buzz"
              className="w-full max-w-[560px]"
              src="/landing/buzz-wordmark.png"
            />
            <p className="mt-2 max-w-[560px] text-center text-2xl font-normal leading-none text-foreground">
              Your people, your agents, your projects — all in one place.
            </p>
            {error ? (
              <p className="mt-4 text-sm text-destructive">{error}</p>
            ) : null}
            <div className="mt-10 flex flex-col items-center gap-3">
              <Button
                className="h-10 rounded-full px-6"
                disabled={isPending}
                onClick={() => void loadFreshIdentity()}
                type="button"
              >
                {isPending ? "Saving identity…" : "Get started"}
              </Button>
              <Button
                className="h-9 rounded-full bg-foreground/10 px-5 hover:bg-foreground/15"
                disabled={isPending}
                onClick={() => setPage("key-import")}
                type="button"
                variant="ghost"
              >
                Enter a key
              </Button>
            </div>
          </OnboardingSlideTransition>
        ) : page === "key-import" ? (
          <OnboardingSlideTransition
            className="flex w-full max-w-[640px] flex-col items-center text-center"
            direction="forward"
            transitionKey="machine-key-import"
          >
            <h1 className="text-3xl font-semibold tracking-tight">
              {identityLost ? "Re-import your key" : "Enter your private key"}
            </h1>
            <p className="mt-3 max-w-[440px] text-sm leading-6 text-foreground/80">
              {identityLost
                ? "Your identity is no longer in the system keyring. Re-import your nsec to restore it."
                : "If you already have a Nostr account, enter your private key below to get started."}
            </p>
            <NostrKeyImportForm
              backLabel={identityLost ? "Start new identity" : "Back"}
              onBack={
                identityLost
                  ? () => void replaceLostIdentity()
                  : () => setPage("identity")
              }
              onImport={importExistingIdentity}
              variant="spotlight"
            />
          </OnboardingSlideTransition>
        ) : page === "backup" ? (
          <BackupStep
            direction="forward"
            onBack={() => setPage("identity")}
            onNext={() => setPage("setup")}
          />
        ) : (
          <SetupStep
            actions={{
              back: () =>
                setPage(identityWasImported ? "key-import" : "backup"),
              complete: () => complete(selectedPubkey ?? undefined),
            }}
            direction="forward"
          />
        )}
      </div>
    </div>
  );
}
