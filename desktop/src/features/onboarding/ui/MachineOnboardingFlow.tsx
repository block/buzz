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
import { NostrKeyImportForm } from "./NostrKeyImportForm";
import { OnboardingSlideTransition } from "./OnboardingSlideTransition";
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
      className="buzz-onboarding-neutral-theme buzz-startup-shell flex items-center justify-center bg-background px-4 py-8 text-foreground"
      data-testid="machine-onboarding-gate"
    >
      <StartupWindowDragRegion />
      <div className="relative flex w-full max-w-[920px] flex-col items-center text-center">
        {page === "identity" ? (
          <OnboardingSlideTransition
            className="flex w-full max-w-[500px] flex-col items-center text-center"
            direction="forward"
            effect="mask-reveal-up"
            transitionKey="machine-identity"
          >
            <img
              alt="Buzz"
              className="h-14 w-14 rounded-xl shadow-xs"
              src="/app-icon@2x.png"
              srcSet="/app-icon@2x.png 1x, /app-icon@3x.png 2x"
            />
            <h1 className="mt-6 text-3xl font-semibold tracking-tight">
              Welcome to Buzz
            </h1>
            <p className="mt-3 max-w-[440px] text-sm leading-6 text-muted-foreground">
              Start with a new Nostr identity or bring the key you already use.
              Your identity works across every community you join.
            </p>
            {error ? (
              <p className="mt-4 text-sm text-destructive">{error}</p>
            ) : null}
            <div className="mt-8 flex w-full flex-col gap-3">
              <Button
                className="h-10 w-full"
                disabled={isPending}
                onClick={() => void loadFreshIdentity()}
                type="button"
              >
                {isPending ? "Saving identity…" : "Get started"}
              </Button>
              <Button
                className="h-10 w-full"
                disabled={isPending}
                onClick={() => setPage("key-import")}
                type="button"
                variant="ghost"
              >
                I already have a key
              </Button>
            </div>
          </OnboardingSlideTransition>
        ) : page === "key-import" ? (
          <OnboardingSlideTransition
            className="flex w-full max-w-[500px] flex-col items-center text-center"
            direction="forward"
            transitionKey="machine-key-import"
          >
            <h1 className="text-3xl font-semibold tracking-tight">
              {identityLost ? "Re-import your key" : "Use your existing key"}
            </h1>
            <p className="mt-3 text-sm leading-6 text-muted-foreground">
              {identityLost
                ? "Your identity is no longer in the system keyring. Re-import your nsec to restore it."
                : "Import your Nostr private key to use that identity with Buzz."}
            </p>
            <NostrKeyImportForm
              backLabel={identityLost ? "Start new identity" : "Back"}
              onBack={
                identityLost
                  ? () => void replaceLostIdentity()
                  : () => setPage("identity")
              }
              onImport={importExistingIdentity}
            />
          </OnboardingSlideTransition>
        ) : page === "backup" ? (
          <BackupStep
            currentStep={2}
            direction="forward"
            onBack={() => setPage("identity")}
            onNext={() => setPage("setup")}
            totalSteps={3}
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
