import * as React from "react";
import type { QueryClient } from "@tanstack/react-query";

import {
  getGlobalAgentConfig,
  setGlobalAgentConfig,
} from "@/shared/api/tauriGlobalAgentConfig";
import {
  getIdentity,
  importIdentity,
  persistCurrentIdentity,
} from "@/shared/api/tauriIdentity";
import { Button } from "@/shared/ui/button";
import { StartupWindowDragRegion } from "@/shared/ui/StartupWindowDragRegion";
import { BackupStep } from "./BackupStep";
import { DefaultConfigStep } from "./DefaultConfigStep";
import { IdentityKeyHelpDialog } from "./IdentityKeyHelpDialog";
import { LandingBees } from "./LandingBees";
import { NostrKeyImportForm } from "./NostrKeyImportForm";
import {
  ONBOARDING_LANDING_CTA_CLASS,
  OnboardingChrome,
} from "./OnboardingChrome";
import { OnboardingFooterProvider } from "./OnboardingFooter";
import {
  getPreferredRuntimeIdForSelection,
  runtimeSelectionNeedsDefaultsStep,
} from "./onboardingRuntimeSelection";
import { OnboardingSlideTransition } from "./OnboardingSlideTransition";
import { SetupStep } from "./SetupStep";

export type MachineOnboardingPage =
  | "identity"
  | "key-import"
  | "backup"
  | "setup"
  | "config";

export function MachineOnboardingFlow({
  complete,
  continueWithIdentity,
  identityLost,
  initialPage,
  queryClient,
}: {
  complete: (pubkey?: string) => void;
  continueWithIdentity: (pubkey: string) => void;
  identityLost: boolean;
  initialPage?: MachineOnboardingPage;
  queryClient: QueryClient;
}) {
  const [page, setPage] = React.useState<MachineOnboardingPage>(
    identityLost ? "key-import" : (initialPage ?? "identity"),
  );
  const [error, setError] = React.useState<string | null>(null);
  const [isPending, setIsPending] = React.useState(false);
  const [identityWasImported, setIdentityWasImported] = React.useState(false);
  const [selectedPubkey, setSelectedPubkey] = React.useState<string | null>(
    null,
  );
  const [selectedRuntimeIds, setSelectedRuntimeIds] = React.useState<string[]>(
    [],
  );
  const [isRuntimeSelectionSaving, setIsRuntimeSelectionSaving] =
    React.useState(false);
  const [runtimeSelectionError, setRuntimeSelectionError] = React.useState<
    string | null
  >(null);
  const runtimeSaveSequence = React.useRef(0);
  const runtimeSaveChain = React.useRef<Promise<void>>(Promise.resolve());

  const persistHarnessSelection = React.useCallback(
    (runtimeIds: readonly string[]) => {
      const nextRuntimeIds = Array.from(new Set(runtimeIds));
      const preferredRuntimeId =
        getPreferredRuntimeIdForSelection(nextRuntimeIds);
      const sequence = runtimeSaveSequence.current + 1;
      runtimeSaveSequence.current = sequence;
      setSelectedRuntimeIds(nextRuntimeIds);
      setRuntimeSelectionError(null);
      setIsRuntimeSelectionSaving(true);

      const save = runtimeSaveChain.current.then(async () => {
        const current = await getGlobalAgentConfig();
        await setGlobalAgentConfig({
          ...current,
          preferred_runtime: preferredRuntimeId,
        });
      });
      runtimeSaveChain.current = save.then(
        () => undefined,
        () => undefined,
      );
      void save
        .catch((cause) => {
          if (runtimeSaveSequence.current === sequence) {
            setRuntimeSelectionError(
              cause instanceof Error
                ? cause.message
                : "Couldn’t save your harness selection.",
            );
          }
        })
        .finally(() => {
          if (runtimeSaveSequence.current === sequence) {
            setIsRuntimeSelectionSaving(false);
          }
        });
    },
    [],
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
      continueWithIdentity(identity.pubkey);
      queryClient.setQueryData(["identity"], identity);
      setIdentityWasImported(true);
      setSelectedPubkey(identity.pubkey);
      setPage("setup");
    },
    [continueWithIdentity, queryClient],
  );

  return (
    <div
      className={`buzz-onboarding-neutral-theme buzz-startup-shell flex max-h-dvh items-start justify-center overflow-x-hidden overflow-y-auto px-4 text-foreground ${
        page === "identity"
          ? "buzz-onboarding-welcome py-8"
          : "pb-28 pt-[106px]"
      }`}
      data-testid="machine-onboarding-gate"
    >
      <StartupWindowDragRegion />
      {page === "identity" ? <LandingBees /> : null}
      {page !== "identity" ? (
        <OnboardingChrome
          current={page === "config" ? 4 : page === "setup" ? 3 : 2}
        />
      ) : null}
      <OnboardingFooterProvider>
        <div
          className={`relative flex w-full max-w-[920px] flex-col items-center text-center ${
            page === "identity" ? "my-auto" : "buzz-onboarding-step-frame"
          }`}
        >
          {page === "identity" ? (
            <OnboardingSlideTransition
              className="flex w-full max-w-[720px] flex-col items-center text-center"
              direction="forward"
              effect="mask-reveal-up"
              transitionKey="machine-identity"
            >
              <img
                alt="Buzz"
                className="w-full max-w-[600px]"
                src="/landing/buzz-wordmark.png"
              />
              <p className="mt-2 max-w-[560px] text-center text-2xl font-normal leading-none text-foreground">
                Your people, your agents, your projects —<br />
                all in one place.
              </p>
              {error ? (
                <p className="mt-4 text-sm text-destructive">{error}</p>
              ) : null}
              <div className="mt-10 flex flex-col items-center gap-3">
                <Button
                  className={ONBOARDING_LANDING_CTA_CLASS}
                  disabled={isPending}
                  onClick={() => void loadFreshIdentity()}
                  type="button"
                >
                  {isPending ? "Saving identity…" : "Create a new identity key"}
                </Button>
                <Button
                  className="h-9 rounded-full bg-foreground/10 px-5 hover:bg-foreground/15"
                  disabled={isPending}
                  onClick={() => setPage("key-import")}
                  type="button"
                  variant="ghost"
                >
                  Use an existing key
                </Button>
              </div>
              <IdentityKeyHelpDialog />
            </OnboardingSlideTransition>
          ) : page === "key-import" ? (
            <OnboardingSlideTransition
              className="flex min-h-[calc(100dvh-13.25rem)] w-full max-w-[837px] flex-col items-center text-center"
              direction="forward"
              effect="fade"
              transitionKey="machine-key-import"
            >
              <div className="shrink-0">
                <h1 className="text-title font-normal text-foreground">
                  {identityLost
                    ? "Re-import your key"
                    : "Enter your private key"}
                </h1>
                <p className="mt-5 max-w-[440px] text-sm leading-6 text-foreground/80">
                  {identityLost
                    ? "Your identity is no longer in the system keyring. Re-import your nsec to restore it."
                    : "If you already have a Buzz account, enter your private key below to get started."}
                </p>
              </div>
              <div className="buzz-onboarding-key-import-position w-full">
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
              </div>
            </OnboardingSlideTransition>
          ) : page === "backup" ? (
            <BackupStep
              direction="forward"
              onBack={() => setPage("identity")}
              onNext={() => setPage("setup")}
            />
          ) : page === "setup" ? (
            <SetupStep
              actions={{
                back: () =>
                  setPage(identityWasImported ? "key-import" : "backup"),
                next: () => {
                  if (selectedRuntimeIds.length === 0) return;
                  if (!runtimeSelectionNeedsDefaultsStep(selectedRuntimeIds)) {
                    complete(selectedPubkey ?? undefined);
                    return;
                  }
                  setPage("config");
                },
              }}
              direction="forward"
              isSelectionSaving={isRuntimeSelectionSaving}
              onSelectedRuntimeIdsChange={(runtimeIds) => {
                void persistHarnessSelection(runtimeIds);
              }}
              selectionError={runtimeSelectionError}
              selectedRuntimeIds={selectedRuntimeIds}
            />
          ) : (
            <DefaultConfigStep
              actions={{
                back: () => setPage("setup"),
                complete: () => complete(selectedPubkey ?? undefined),
              }}
              direction="forward"
              selectedRuntimeIds={selectedRuntimeIds}
            />
          )}
        </div>
      </OnboardingFooterProvider>
    </div>
  );
}
