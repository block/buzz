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
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogTitle,
} from "@/shared/ui/dialog";
import { StartupWindowDragRegion } from "@/shared/ui/StartupWindowDragRegion";
import { BackupStep } from "./BackupStep";
import { DefaultConfigStep } from "./DefaultConfigStep";
import { DownloadKeyStep } from "./DownloadKeyStep";
import {
  backupSessionToPasswordEntry,
  resetEncryptedBackupSession,
  useEncryptedBackupSession,
} from "./EncryptedBackupCreator";
import { IdentityKeyHelpDialog } from "./IdentityKeyHelpDialog";
import { IdentityRecoveryPairing } from "./IdentityRecoveryPairing";
import { LandingBees } from "./LandingBees";
import {
  NostrKeyImportForm,
  type NostrKeyImportStage,
} from "./NostrKeyImportForm";
import {
  ONBOARDING_INK_ICON_CLASS,
  ONBOARDING_LANDING_CTA_CLASS,
  ONBOARDING_SECONDARY_CTA_CLASS,
  OnboardingChrome,
} from "./OnboardingChrome";
import { OnboardingFooterProvider } from "./OnboardingFooter";
import {
  type OnboardingTransitionDirection,
  OnboardingSlideTransition,
} from "./OnboardingSlideTransition";
import { SetupStep } from "./SetupStep";
import type { DefaultConfigDraft } from "./types";

export type MachineOnboardingPage =
  | "identity"
  | "key-import"
  | "backup"
  | "setup"
  | "config";

type BackupSubview = "created" | "options" | "password";

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
  const [phoneRecoveryStep, setPhoneRecoveryStep] = React.useState("loading");
  const [selectedPubkey, setSelectedPubkey] = React.useState<string | null>(
    null,
  );
  const [identityStorage, setIdentityStorage] = React.useState<
    IdentityStorage | undefined
  >();
  const [readyRuntimeIds, setReadyRuntimeIds] = React.useState<string[]>([]);
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
  // Owned here so switching between the yellow onboarding view and the dark
  // security subview keeps the created backup, password, and test progress.
  const backupSession = useEncryptedBackupSession();
  const reduceMotion = useReducedMotion() ?? false;
  const isSecuritySubview = page === "backup" && backupSubview !== "created";
  const handleReadyRuntimeIdsChange = React.useCallback(
    (runtimeIds: readonly string[]) => {
      setReadyRuntimeIds(Array.from(new Set(runtimeIds)));
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
    setTransitionDirection("backward");
    setPage("identity");
  }, [keyImportStage]);

  const returnToCreatedKey = React.useCallback(() => {
    setBackupDirection("backward");
    setReturningFromSecurity(true);
    setBackupSubview("created");
  }, []);

  const backFromPasswordBackup = React.useCallback(() => {
    resetEncryptedBackupSession(backupSession);
    setBackupDirection("backward");
    setReturningFromSecurity(false);
    setBackupSubview("options");
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

  const chromeBackAction =
    page === "key-import" &&
    (!identityLost || keyImportStage === "backup-password")
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
                setPage("identity");
              },
            }
          : page === "setup"
            ? { onClick: backFromSetup }
            : page === "config"
              ? {
                  disabled: isDefaultConfigSaving,
                  onClick: () => {
                    setTransitionDirection("backward");
                    setPage("setup");
                  },
                }
              : undefined;

  return (
    <div
      className={`buzz-onboarding-neutral-theme buzz-startup-shell flex max-h-dvh items-start justify-center overflow-x-hidden overflow-y-auto px-4 text-foreground ${
        isSecuritySubview ? "buzz-onboarding-security-theme" : ""
      } ${
        page === "identity"
          ? "buzz-onboarding-welcome py-8"
          : "pb-28 pt-[106px]"
      }`}
      data-testid="machine-onboarding-gate"
    >
      <StartupWindowDragRegion />
      {page === "identity" ? <LandingBees /> : null}
      {page !== "identity" && !isSecuritySubview ? (
        <OnboardingChrome
          current={page === "config" ? 4 : page === "setup" ? 3 : 2}
        />
      ) : null}
      <OnboardingFooterProvider backAction={chromeBackAction}>
        <div
          className={`relative flex w-full max-w-[1040px] flex-col items-center text-center ${
            page === "identity" ? "my-auto" : "buzz-onboarding-step-frame"
          }`}
        >
          {page === "identity" ? (
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
                  onClick={() => void loadFreshIdentity()}
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
              <IdentityKeyHelpDialog />
            </OnboardingSlideTransition>
          ) : page === "key-import" ? (
            <OnboardingSlideTransition
              className="flex min-h-[calc(100dvh-13.25rem)] w-full max-w-[837px] flex-col items-center text-center"
              direction={transitionDirection}
              transitionKey={`machine-key-import-${transitionDirection}`}
            >
              <motion.div
                animate={{ opacity: 1, y: 0 }}
                className="relative z-10 shrink-0"
                initial={reduceMotion ? false : { opacity: 0, y: 10 }}
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
                <div className="mt-5 max-w-[440px] text-sm leading-6 text-foreground/80">
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
                        onClick={() => setKeyImportDialog("backup")}
                        type="button"
                      >
                        backup file
                      </button>
                      , or{" "}
                      <button
                        className="rounded-sm font-medium underline decoration-foreground/40 underline-offset-4 transition-colors hover:decoration-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 disabled:opacity-60"
                        data-testid="nostr-import-phone-link"
                        disabled={isPending}
                        onClick={() => setKeyImportDialog("phone")}
                        type="button"
                      >
                        recover from your phone
                      </button>
                      .
                    </p>
                  )}
                </div>
              </motion.div>
              <div className="buzz-onboarding-key-import-position w-full">
                <div className="flex flex-col items-center">
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
              <Dialog
                onOpenChange={(open) => {
                  if (!open) setKeyImportDialog(null);
                }}
                open={keyImportDialog === "backup"}
              >
                <DialogContent
                  className="buzz-onboarding-neutral-theme max-w-[47.5rem] -translate-y-5"
                  closeButtonClassName={ONBOARDING_INK_ICON_CLASS}
                  data-system-color-scheme="light"
                  data-testid="backup-recovery-dialog"
                  surface="textured"
                >
                  <div className="mx-auto w-full max-w-[35rem] pb-6 pt-10 text-center max-sm:pb-4 max-sm:pt-6">
                    <DialogTitle className="text-balance px-8 text-3xl font-normal text-foreground">
                      Restore from a backup file
                    </DialogTitle>
                    <DialogDescription className="mx-auto mt-4 max-w-[28rem] text-sm leading-6 text-foreground/80">
                      Choose the encrypted backup file you saved from Buzz.
                    </DialogDescription>
                    <NostrKeyImportForm
                      footerMode="inline"
                      mode="backup"
                      onBack={() => setKeyImportDialog(null)}
                      onImport={importExistingIdentity}
                      showBack={false}
                      variant="spotlight"
                    />
                  </div>
                </DialogContent>
              </Dialog>
              <Dialog
                onOpenChange={(open) => {
                  if (!open) setKeyImportDialog(null);
                }}
                open={keyImportDialog === "phone"}
              >
                <DialogContent
                  className="buzz-onboarding-neutral-theme max-h-[calc(100dvh-2rem)] max-w-[47.5rem] -translate-y-5 overflow-y-auto"
                  closeButtonClassName={ONBOARDING_INK_ICON_CLASS}
                  data-system-color-scheme="light"
                  data-testid="phone-recovery-dialog"
                  surface="textured"
                >
                  <div className="mx-auto flex w-full max-w-[35rem] flex-col items-center pb-6 pt-8 text-center max-sm:pb-4 max-sm:pt-4">
                    <DialogTitle className="text-balance px-8 text-3xl font-normal text-foreground">
                      {identityLost
                        ? "Recover from your phone"
                        : "Use your Buzz identity"}
                    </DialogTitle>
                    <DialogDescription className="mt-4 text-sm leading-6 text-foreground/80">
                      {phoneRecoveryStep === "loading" ||
                      phoneRecoveryStep === "qr"
                        ? "Scan this code with a signed-in Buzz phone."
                        : "Confirm the code before sharing your identity."}
                    </DialogDescription>
                    <div className="mt-5">
                      <IdentityRecoveryPairing
                        onRecovered={loadRecoveredIdentity}
                        onStepChange={setPhoneRecoveryStep}
                      />
                    </div>
                  </div>
                </DialogContent>
              </Dialog>
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
                onShowOptions={() => {
                  setBackupDirection("forward");
                  setReturningFromSecurity(false);
                  setBackupSubview("options");
                }}
                optionsExpanded={backupSubview === "options"}
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
                next: (runtimeIds) => {
                  const ids = Array.from(runtimeIds);
                  setReadyRuntimeIds(ids);
                  // Harness install can fail (Windows/PATH/network). Don't soft-lock
                  // onboarding — users can finish setup later in Settings → Agents.
                  if (ids.length === 0) {
                    complete(selectedPubkey ?? undefined);
                    return;
                  }
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
              onReadyRuntimeIdsChange={handleReadyRuntimeIdsChange}
            />
          ) : (
            <DefaultConfigStep
              actions={{
                back: () => {
                  setTransitionDirection("backward");
                  setPage("setup");
                },
                complete: () => complete(selectedPubkey ?? undefined),
                discardDraft: () => setDefaultConfigDraft(null),
                updateDraft: setDefaultConfigDraft,
              }}
              direction={transitionDirection}
              draft={defaultConfigDraft}
              onSavingChange={setIsDefaultConfigSaving}
              readyRuntimeIds={readyRuntimeIds}
            />
          )}
        </div>
      </OnboardingFooterProvider>
    </div>
  );
}
