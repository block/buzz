import * as React from "react";
import type { QueryClient } from "@tanstack/react-query";
import { motion, useReducedMotion } from "motion/react";

import {
  getIdentity,
  importIdentity,
  persistCurrentIdentity,
} from "@/shared/api/tauriIdentity";
import type { IdentityStorage } from "@/shared/api/types";
import { Button } from "@/shared/ui/button";
import { StartupWindowDragRegion } from "@/shared/ui/StartupWindowDragRegion";
import { BackupStep } from "./BackupStep";
import { DefaultConfigStep } from "./DefaultConfigStep";
import { DownloadKeyStep } from "./DownloadKeyStep";
import {
  backupSessionToPasswordEntry,
  resetEncryptedBackupSession,
  useEncryptedBackupSession,
} from "./EncryptedBackupCreator";
import {
  IdentityKeyHelpContent,
  IdentityKeyHelpDialog,
} from "./IdentityKeyHelpDialog";
import { IdentityKeyIntroduction } from "./IdentityKeyIntroduction";
import { IdentityRecoveryPairing } from "./IdentityRecoveryPairing";
import { LandingBees } from "./LandingBees";
import {
  NostrKeyImportForm,
  type NostrKeyImportStage,
} from "./NostrKeyImportForm";
import {
  ONBOARDING_LANDING_CTA_CLASS,
  ONBOARDING_SECONDARY_CTA_CLASS,
} from "./OnboardingChrome";
import { OnboardingCard } from "./OnboardingCard";
import { OnboardingFooterProvider } from "./OnboardingFooter";
import {
  type OnboardingTransitionDirection,
  OnboardingSlideTransition,
} from "./OnboardingSlideTransition";
import { SetupStep } from "./SetupStep";
import type { HarnessConnectionMethod } from "./harnessConnectionOptions";
import type { DefaultConfigDraft } from "./types";

export type MachineOnboardingPage =
  | "identity"
  | "identity-key-intro"
  | "identity-key-help"
  | "key-import"
  | "backup"
  | "setup"
  | "config";

type BackupSubview = "created" | "password";

/** A pending navigation the parent should execute after RouterProvider mounts. */
export type PostOnboardingNavigation = {
  to: string;
  search?: Record<string, string>;
};

export function MachineOnboardingFlow({
  complete,
  continueWithIdentity,
  continueWithRecoveredIdentity,
  identityLost,
  initialPage,
  queryClient,
  navigateAfterComplete,
}: {
  complete: (pubkey?: string) => void;
  continueWithIdentity: (pubkey: string) => void;
  continueWithRecoveredIdentity: (pubkey: string) => void;
  identityLost: boolean;
  initialPage?: MachineOnboardingPage;
  queryClient: QueryClient;
  /**
   * Called when the user finishes onboarding and requests navigation to a
   * specific route (e.g. Settings → Agents). The parent owns the RouterProvider,
   * so navigation must be deferred to it — calling router.navigate() here races
   * with RouterProvider mounting.
   */
  navigateAfterComplete?: (nav: PostOnboardingNavigation) => void;
}) {
  const [page, setPage] = React.useState<MachineOnboardingPage>(
    identityLost ? "key-import" : (initialPage ?? "identity"),
  );
  const [transitionDirection, setTransitionDirection] =
    React.useState<OnboardingTransitionDirection>("forward");
  const [error, setError] = React.useState<string | null>(null);
  const [isPending, setIsPending] = React.useState(false);
  const [identityWasImported, setIdentityWasImported] = React.useState(false);
  const [keyImportStage, setKeyImportStage] =
    React.useState<NostrKeyImportStage>("key-entry");
  const [isKeyImporting, setIsKeyImporting] = React.useState(false);
  const [keyImportFormKey, setKeyImportFormKey] = React.useState(0);
  const [keyImportDialog, setKeyImportDialog] = React.useState<
    "backup" | "phone" | null
  >(null);
  const [identityKeyHelpReturnPage, setIdentityKeyHelpReturnPage] =
    React.useState<"identity" | "identity-key-intro">("identity");
  const [phoneRecoveryStep, setPhoneRecoveryStep] = React.useState("loading");
  const [selectedPubkey, setSelectedPubkey] = React.useState<string | null>(
    null,
  );
  const [identityStorage, setIdentityStorage] = React.useState<
    IdentityStorage | undefined
  >();
  const [readyRuntimeIds, setReadyRuntimeIds] = React.useState<string[]>([]);
  const [setupBackAction, setSetupBackAction] = React.useState<
    (() => void) | null
  >(null);
  const [harnessConnectionMethod, setHarnessConnectionMethod] =
    React.useState<HarnessConnectionMethod | null>(null);
  const [configBackTarget, setConfigBackTarget] = React.useState<
    "method" | "list"
  >("method");
  const [isChoosingDifferentHarness, setIsChoosingDifferentHarness] =
    React.useState(false);
  const [defaultConfigDraft, setDefaultConfigDraft] =
    React.useState<DefaultConfigDraft | null>(null);
  const [isDefaultConfigSaving, setIsDefaultConfigSaving] =
    React.useState(false);
  const [backupSubview, setBackupSubview] =
    React.useState<BackupSubview>("created");
  const [backupDirection, setBackupDirection] = React.useState<
    "forward" | "backward"
  >("forward");
  const [returningFromSecurity, setReturningFromSecurity] =
    React.useState(false);
  // Owned here so switching between the onboarding card and the security
  // subview keeps the created backup, password, and test progress.
  const backupSession = useEncryptedBackupSession();
  const reduceMotion = useReducedMotion() ?? false;
  const handleReadyRuntimeIdsChange = React.useCallback(
    (runtimeIds: readonly string[]) => {
      setReadyRuntimeIds(Array.from(new Set(runtimeIds)));
    },
    [],
  );
  const handleSetupBackActionChange = React.useCallback(
    (backAction: () => void) => setSetupBackAction(() => backAction),
    [],
  );

  const loadFreshIdentity = React.useCallback(async () => {
    setIsPending(true);
    setError(null);
    try {
      const identity = await getIdentity();
      queryClient.setQueryData(["identity"], identity);
      setSelectedPubkey(identity.pubkey);
      setIdentityStorage(identity.storage);
      setBackupDirection("forward");
      setTransitionDirection("forward");
      setReturningFromSecurity(false);
      setBackupSubview("created");
      setPage("backup");
    } catch (cause) {
      setError(
        cause instanceof Error ? cause.message : "Failed to load identity",
      );
    } finally {
      setIsPending(false);
    }
  }, [queryClient]);

  const loadRecoveredIdentity = React.useCallback(async () => {
    setIsPending(true);
    setError(null);
    try {
      const identity = await getIdentity();
      continueWithRecoveredIdentity(identity.pubkey);
      queryClient.setQueryData(["identity"], identity);
      setIdentityWasImported(true);
      setSelectedPubkey(identity.pubkey);
      setIdentityStorage(identity.storage);
      setTransitionDirection("forward");
      setPage("setup");
    } catch (cause) {
      setError(
        cause instanceof Error ? cause.message : "Failed to load identity",
      );
    } finally {
      setIsPending(false);
    }
  }, [continueWithRecoveredIdentity, queryClient]);

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
      setIdentityStorage(identity.storage);
      setBackupDirection("forward");
      setTransitionDirection("forward");
      setReturningFromSecurity(false);
      setBackupSubview("created");
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
    async (nsec: string, password?: string) => {
      const identity = await importIdentity(nsec, password);
      continueWithIdentity(identity.pubkey);
      queryClient.setQueryData(["identity"], identity);
      setIdentityWasImported(true);
      setSelectedPubkey(identity.pubkey);
      setTransitionDirection("forward");
      setPage("setup");
    },
    [continueWithIdentity, queryClient],
  );

  const backFromKeyImport = React.useCallback(() => {
    if (keyImportStage === "backup-password") {
      setKeyImportFormKey((current) => current + 1);
      setKeyImportStage("key-entry");
      return;
    }
    if (keyImportDialog) {
      setKeyImportDialog(null);
      setPhoneRecoveryStep("loading");
      return;
    }
    setTransitionDirection("backward");
    setPage("identity");
  }, [keyImportDialog, keyImportStage]);

  const returnToCreatedKey = React.useCallback(() => {
    setBackupDirection("backward");
    setReturningFromSecurity(true);
    setBackupSubview("created");
  }, []);

  const backFromPasswordBackup = React.useCallback(() => {
    resetEncryptedBackupSession(backupSession);
    setBackupDirection("backward");
    setReturningFromSecurity(true);
    setBackupSubview("created");
  }, [backupSession]);

  const backFromSetup = React.useCallback(() => {
    if (identityWasImported) {
      setKeyImportFormKey((current) => current + 1);
      setKeyImportStage("key-entry");
      setTransitionDirection("backward");
      setPage("key-import");
      return;
    }
    if (backupSubview === "password") {
      backupSessionToPasswordEntry(backupSession);
    }
    setBackupDirection("backward");
    setTransitionDirection("backward");
    setReturningFromSecurity(false);
    setPage("backup");
  }, [backupSession, backupSubview, identityWasImported]);

  const backFromConfig = React.useCallback(() => {
    setTransitionDirection("backward");
    setIsChoosingDifferentHarness(false);
    if (configBackTarget === "method") {
      setHarnessConnectionMethod(null);
    }
    setPage("setup");
  }, [configBackTarget]);

  const chromeBackAction =
    page === "identity-key-help"
      ? {
          onClick: () => {
            setTransitionDirection("backward");
            setPage(identityKeyHelpReturnPage);
          },
        }
      : page === "identity-key-intro"
        ? {
            disabled: isPending,
            onClick: () => {
              setTransitionDirection("backward");
              setPage("identity");
            },
          }
        : page === "key-import" &&
            (keyImportDialog !== null ||
              !identityLost ||
              keyImportStage === "backup-password")
          ? { disabled: isKeyImporting, onClick: backFromKeyImport }
          : page === "backup" && backupSubview !== "created"
            ? {
                label: "Return to onboarding",
                onClick: returnToCreatedKey,
                testId: "backup-return-to-onboarding",
              }
            : page === "backup"
              ? {
                  onClick: () => {
                    setTransitionDirection("backward");
                    setPage("identity-key-intro");
                  },
                }
              : page === "setup"
                ? { onClick: setupBackAction ?? backFromSetup }
                : page === "config"
                  ? {
                      disabled: isDefaultConfigSaving,
                      onClick: backFromConfig,
                    }
                  : undefined;

  if (page === "identity") {
    return (
      <div
        className="buzz-onboarding-neutral-theme buzz-startup-shell buzz-onboarding-welcome flex max-h-dvh items-start justify-center overflow-x-hidden overflow-y-auto px-4 py-8 text-foreground"
        data-testid="machine-onboarding-gate"
      >
        <StartupWindowDragRegion />
        <LandingBees />
        <OnboardingFooterProvider>
          <div className="relative my-auto flex w-full max-w-[1040px] flex-col items-center text-center">
            <OnboardingSlideTransition
              className="flex w-full max-w-[720px] flex-col items-center text-center"
              direction={transitionDirection}
              transitionKey={`machine-identity-${transitionDirection}`}
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
                  onClick={() => {
                    if (selectedPubkey) {
                      void loadFreshIdentity();
                      return;
                    }
                    setTransitionDirection("forward");
                    setPage("identity-key-intro");
                  }}
                  type="button"
                >
                  {isPending
                    ? "Loading identity…"
                    : selectedPubkey
                      ? "Continue setup"
                      : "Create a new identity key"}
                </Button>
                <Button
                  className={`${ONBOARDING_SECONDARY_CTA_CLASS} px-5`}
                  disabled={isPending}
                  onClick={() => {
                    setKeyImportDialog(null);
                    setKeyImportStage("key-entry");
                    setTransitionDirection("forward");
                    setPage("key-import");
                  }}
                  type="button"
                  variant="ghost"
                >
                  {selectedPubkey
                    ? "Use a different key instead"
                    : "Use an existing key"}
                </Button>
              </div>
              <IdentityKeyHelpDialog
                onOpen={() => {
                  setIdentityKeyHelpReturnPage("identity");
                  setTransitionDirection("forward");
                  setPage("identity-key-help");
                }}
              />
            </OnboardingSlideTransition>
          </div>
        </OnboardingFooterProvider>
      </div>
    );
  }

  return (
    <OnboardingCard
      backAction={chromeBackAction}
      current={page === "config" ? 4 : page === "setup" ? 3 : 2}
      testId="machine-onboarding-gate"
    >
      {page === "identity-key-intro" ? (
        <IdentityKeyIntroduction
          direction={transitionDirection}
          disabled={isPending}
          onCreate={() => void loadFreshIdentity()}
          onOpenHelp={() => {
            setIdentityKeyHelpReturnPage("identity-key-intro");
            setTransitionDirection("forward");
            setPage("identity-key-help");
          }}
        />
      ) : page === "identity-key-help" ? (
        <OnboardingSlideTransition
          className="flex min-h-0 w-full flex-col items-stretch justify-start text-left"
          direction={transitionDirection}
          transitionKey={`identity-key-help-${transitionDirection}`}
        >
          <IdentityKeyHelpContent />
        </OnboardingSlideTransition>
      ) : page === "key-import" ? (
        <OnboardingSlideTransition
          className="flex min-h-0 w-full flex-col items-stretch text-left"
          direction={transitionDirection}
          transitionKey={`machine-key-import-${keyImportDialog ?? "key"}-${transitionDirection}`}
        >
          {keyImportDialog === "backup" ? (
            <div className="w-full" data-testid="backup-recovery-dialog">
              <h1 className="text-title font-normal text-foreground">
                Restore from a backup file
              </h1>
              <p className="mt-2 w-full text-base leading-6 text-foreground/80">
                Choose the encrypted backup file you saved from Buzz.
              </p>
              <NostrKeyImportForm
                key={keyImportFormKey}
                mode="backup"
                onBack={backFromKeyImport}
                onImport={importExistingIdentity}
                onImportingChange={setIsKeyImporting}
                onStageChange={setKeyImportStage}
                showBack={false}
                showPasswordStageBack={false}
                variant="spotlight"
              />
            </div>
          ) : keyImportDialog === "phone" ? (
            <div
              className="flex min-h-0 w-full flex-1 flex-col"
              data-testid="phone-recovery-dialog"
            >
              <h1 className="text-title font-normal text-foreground">
                {identityLost ? "Recover from your phone" : "Scan to sign in"}
              </h1>
              <p className="mt-2 w-full text-base leading-6 text-foreground/80">
                {phoneRecoveryStep === "loading" || phoneRecoveryStep === "qr"
                  ? "On your phone, open Settings → Send identity to desktop. This code expires shortly and works once."
                  : "Confirm the code before sharing your identity."}
              </p>
              <div
                className="flex min-h-0 flex-1 items-center justify-center"
                data-testid="identity-recovery-stage"
              >
                <IdentityRecoveryPairing
                  onRecovered={loadRecoveredIdentity}
                  onStepChange={setPhoneRecoveryStep}
                />
              </div>
            </div>
          ) : (
            <>
              <motion.div
                animate={{ opacity: 1 }}
                className="relative z-10 shrink-0 text-left"
                initial={reduceMotion ? false : { opacity: 0 }}
                key={keyImportStage}
                transition={{
                  duration: reduceMotion ? 0 : 0.3,
                  ease: "easeOut",
                }}
              >
                <h1 className="text-title font-normal text-foreground">
                  {keyImportStage === "backup-password"
                    ? "Unlock your account"
                    : "Enter your private key"}
                </h1>
                <div className="mt-2 w-full text-base leading-6 text-foreground/80">
                  {keyImportStage === "backup-password" ? (
                    "Enter your backup password to restore your identity."
                  ) : (
                    <p>
                      Paste your private key to sign in to Buzz. You can also
                      use a{" "}
                      <button
                        className="rounded-sm font-medium underline decoration-foreground/40 underline-offset-4 transition-colors hover:decoration-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 disabled:opacity-60"
                        data-testid="nostr-import-file-button"
                        disabled={isPending}
                        onClick={() => {
                          setKeyImportStage("key-entry");
                          setKeyImportDialog("backup");
                        }}
                        type="button"
                      >
                        backup file
                      </button>
                      , or{" "}
                      <button
                        className="rounded-sm font-medium underline decoration-foreground/40 underline-offset-4 transition-colors hover:decoration-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 disabled:opacity-60"
                        data-testid="nostr-import-phone-link"
                        disabled={isPending}
                        onClick={() => {
                          setPhoneRecoveryStep("loading");
                          setKeyImportDialog("phone");
                        }}
                        type="button"
                      >
                        recover from your phone
                      </button>
                      .
                    </p>
                  )}
                </div>
              </motion.div>
              <div className="mt-8 w-full">
                <div className="flex flex-col items-stretch">
                  <NostrKeyImportForm
                    key={keyImportFormKey}
                    onBack={backFromKeyImport}
                    onImport={importExistingIdentity}
                    onImportingChange={setIsKeyImporting}
                    onStageChange={setKeyImportStage}
                    showBack={false}
                    showPasswordStageBack={false}
                    variant="spotlight"
                  />
                  {identityLost && keyImportStage === "key-entry" ? (
                    <Button
                      className={`${ONBOARDING_SECONDARY_CTA_CLASS} mt-2 px-5`}
                      disabled={isPending || isKeyImporting}
                      onClick={() => void replaceLostIdentity()}
                      type="button"
                      variant="ghost"
                    >
                      Start new identity
                    </Button>
                  ) : null}
                </div>
              </div>
            </>
          )}
        </OnboardingSlideTransition>
      ) : page === "backup" ? (
        backupSubview === "password" ? (
          <DownloadKeyStep
            direction={backupDirection}
            onBack={backFromPasswordBackup}
            session={backupSession}
          />
        ) : (
          <BackupStep
            direction={backupDirection}
            identityStorage={identityStorage}
            onNext={() => {
              setTransitionDirection("forward");
              setPage("setup");
            }}
            onOpenPasswordBackup={() => {
              resetEncryptedBackupSession(backupSession);
              setBackupDirection("forward");
              setReturningFromSecurity(false);
              setBackupSubview("password");
            }}
            optionsExpanded={false}
            returningFromSecurity={returningFromSecurity}
          />
        )
      ) : page === "setup" ? (
        <SetupStep
          actions={{
            // Fresh-key users return to whichever identity backup subview
            // they used to reach setup; imported keys skip backup entirely.
            back: () => {
              backFromSetup();
            },
            next: (runtimeIds, nextConfigBackTarget = "list") => {
              const ids = Array.from(runtimeIds);
              setReadyRuntimeIds(ids);
              // Harness install can fail (Windows/PATH/network). Don't soft-lock
              // onboarding — users can finish setup later in Settings → Agents.
              if (ids.length === 0) {
                complete(selectedPubkey ?? undefined);
                return;
              }
              setConfigBackTarget(nextConfigBackTarget);
              setIsChoosingDifferentHarness(false);
              setTransitionDirection("forward");
              setPage("config");
            },
            navigateToAgentSettings: () => {
              // Complete onboarding first, then delegate the Settings → Agents
              // navigation to the parent.  The parent owns RouterProvider, so
              // navigation from within the onboarding flow races with the
              // router mounting — calling router.navigate() here is unsafe.
              complete(selectedPubkey ?? undefined);
              navigateAfterComplete?.({
                to: "/settings",
                search: { section: "agents" },
              });
            },
          }}
          direction={transitionDirection}
          initialMethod={harnessConnectionMethod}
          onInitialListBack={
            isChoosingDifferentHarness
              ? () => {
                  setIsChoosingDifferentHarness(false);
                  setTransitionDirection("backward");
                  setPage("config");
                }
              : undefined
          }
          onBackActionChange={handleSetupBackActionChange}
          onMethodChange={setHarnessConnectionMethod}
          onReadyRuntimeIdsChange={handleReadyRuntimeIdsChange}
        />
      ) : (
        <DefaultConfigStep
          actions={{
            back: () => {
              backFromConfig();
            },
            complete: () => complete(selectedPubkey ?? undefined),
            discardDraft: () => setDefaultConfigDraft(null),
            updateDraft: setDefaultConfigDraft,
            useDifferentHarness:
              harnessConnectionMethod === "api"
                ? () => {
                    setIsChoosingDifferentHarness(true);
                    setTransitionDirection("forward");
                    setPage("setup");
                  }
                : undefined,
          }}
          direction={transitionDirection}
          draft={defaultConfigDraft}
          onSavingChange={setIsDefaultConfigSaving}
          readyRuntimeIds={readyRuntimeIds}
        />
      )}
    </OnboardingCard>
  );
}
