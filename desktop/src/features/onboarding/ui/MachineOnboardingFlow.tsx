import * as React from "react";
import type { QueryClient } from "@tanstack/react-query";
import { ArrowUp } from "lucide-react";
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
import { IdentityKeyHelpDialog } from "./IdentityKeyHelpDialog";
import { LandingBees } from "./LandingBees";
import {
  NostrKeyImportForm,
  type NostrKeyImportStage,
} from "./NostrKeyImportForm";
import {
  ONBOARDING_LANDING_CTA_CLASS,
  ONBOARDING_SECONDARY_CTA_CLASS,
  OnboardingChrome,
} from "./OnboardingChrome";
import { OnboardingFooterProvider } from "./OnboardingFooter";
import { OnboardingSlideTransition } from "./OnboardingSlideTransition";
import { SetupStep } from "./SetupStep";

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
  identityLost,
  initialPage,
  queryClient,
  navigateAfterComplete,
}: {
  complete: (pubkey?: string) => void;
  continueWithIdentity: (pubkey: string) => void;
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
  const [error, setError] = React.useState<string | null>(null);
  const [isPending, setIsPending] = React.useState(false);
  const [identityWasImported, setIdentityWasImported] = React.useState(false);
  const [keyImportStage, setKeyImportStage] =
    React.useState<NostrKeyImportStage>("key-entry");
  const [selectedPubkey, setSelectedPubkey] = React.useState<string | null>(
    null,
  );
  const [identityStorage, setIdentityStorage] = React.useState<
    IdentityStorage | undefined
  >();
  const [readyRuntimeIds, setReadyRuntimeIds] = React.useState<string[]>([]);
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

  // If boot already resolved a durable identity (e.g. legacy-file → keyring
  // migration), surface "Continue setup" instead of the misleading
  // "Create a new identity key" primary action. Ephemeral first-run keys
  // keep the create label until the user deliberately continues.
  // See https://github.com/block/buzz/issues/4472.
  React.useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const identity = await getIdentity();
        if (cancelled) return;
        const durable =
          identity.storage === "system-keyring" ||
          identity.storage === "local-file" ||
          identity.storage === "environment";
        if (!durable) return;
        setSelectedPubkey(identity.pubkey);
        setIdentityStorage(identity.storage);
        queryClient.setQueryData(["identity"], identity);
      } catch {
        // No readable identity yet — keep the create-new label.
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [queryClient]);

  const loadFreshIdentity = React.useCallback(async () => {
    setIsPending(true);
    setError(null);
    try {
      const identity = await getIdentity();
      queryClient.setQueryData(["identity"], identity);
      setSelectedPubkey(identity.pubkey);
      setIdentityStorage(identity.storage);
      setBackupDirection("forward");
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
      setPage("setup");
    },
    [continueWithIdentity, queryClient],
  );

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
      {isSecuritySubview ? (
        <div className="fixed inset-x-0 top-8 z-20 flex justify-center px-6">
          <Button
            className={`${ONBOARDING_SECONDARY_CTA_CLASS} gap-2 px-5`}
            data-testid="backup-return-to-onboarding"
            onClick={() => {
              setBackupDirection("backward");
              setReturningFromSecurity(true);
              setBackupSubview("created");
            }}
            type="button"
            variant="ghost"
          >
            <ArrowUp className="h-4 w-4" aria-hidden="true" />
            Return to onboarding
          </Button>
        </div>
      ) : page !== "identity" ? (
        <OnboardingChrome
          current={page === "config" ? 4 : page === "setup" ? 3 : 2}
        />
      ) : null}
      <OnboardingFooterProvider>
        <div
          className={`relative flex w-full max-w-[1040px] flex-col items-center text-center ${
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
                    setKeyImportStage("key-entry");
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
              direction="forward"
              effect="fade"
              transitionKey="machine-key-import"
            >
              <motion.div
                animate={{ opacity: 1, y: 0 }}
                className="shrink-0"
                initial={reduceMotion ? false : { opacity: 0, y: 10 }}
                key={keyImportStage}
                transition={reduceMotion ? { duration: 0 } : { duration: 0.22 }}
              >
                <h1 className="text-[40px] font-semibold leading-none tracking-tight text-foreground">
                  {keyImportStage === "password"
                    ? "Enter password"
                    : identityLost
                      ? "Recover your identity"
                      : "Use an existing key"}
                </h1>
                <p className="mt-3 text-base leading-6 text-muted-foreground">
                  {keyImportStage === "password"
                    ? "This key is encrypted. Enter the password you set when you backed it up."
                    : identityLost
                      ? "Your identity key is missing from this device. Paste a backup nsec or ncryptsec to recover it."
                      : "Paste an nsec or an encrypted ncryptsec to continue with that identity."}
                </p>
              </motion.div>
              <div className="mt-8 w-full max-w-[560px] flex-1">
                <NostrKeyImportForm
                  backLabel={identityLost ? "Start new identity" : "Back"}
                  onBack={
                    identityLost
                      ? () => void replaceLostIdentity()
                      : () => setPage("identity")
                  }
                  onImport={importExistingIdentity}
                  onStageChange={setKeyImportStage}
                  stage={keyImportStage}
                />
              </div>
            </OnboardingSlideTransition>
          ) : page === "backup" ? (
            <OnboardingSlideTransition
              className="flex min-h-[calc(100dvh-13.25rem)] w-full max-w-[837px] flex-col items-center text-center"
              direction={backupDirection}
              effect={backupSubview === "created" ? "fade" : "slide"}
              transitionKey={`machine-backup-${backupSubview}`}
            >
              {backupSubview === "created" ? (
                <DownloadKeyStep
                  identityStorage={identityStorage}
                  onBack={() => setPage("identity")}
                  onContinue={() => {
                    resetEncryptedBackupSession(backupSession);
                    setPage("setup");
                  }}
                  onOpenBackupOptions={() => {
                    setBackupDirection("forward");
                    setReturningFromSecurity(false);
                    setBackupSubview("options");
                  }}
                  returningFromSecurity={returningFromSecurity}
                />
              ) : (
                <BackupStep
                  initialView={
                    backupSubview === "password" ? "password" : "options"
                  }
                  onBack={() => {
                    setBackupDirection("backward");
                    setReturningFromSecurity(true);
                    setBackupSubview("created");
                  }}
                  onContinue={() => {
                    resetEncryptedBackupSession(backupSession);
                    setPage("setup");
                  }}
                  onViewChange={(view) =>
                    setBackupSubview(view === "password" ? "password" : "options")
                  }
                  session={backupSessionToPasswordEntry(backupSession)}
                />
              )}
            </OnboardingSlideTransition>
          ) : page === "setup" ? (
            <SetupStep
              onBack={() => {
                if (identityWasImported) {
                  setPage("key-import");
                  return;
                }
                setBackupDirection("backward");
                setReturningFromSecurity(true);
                setBackupSubview("created");
                setPage("backup");
              }}
              onContinue={() => setPage("config")}
              onReadyRuntimeIdsChange={handleReadyRuntimeIdsChange}
              onSkip={() => {
                complete(selectedPubkey ?? undefined);
              }}
              readyRuntimeIds={readyRuntimeIds}
            />
          ) : (
            <DefaultConfigStep
              onBack={() => setPage("setup")}
              onComplete={() => {
                complete(selectedPubkey ?? undefined);
              }}
              onNavigateToAgents={() => {
                complete(selectedPubkey ?? undefined);
                navigateAfterComplete?.({ to: "/settings", search: { tab: "agents" } });
              }}
              session={{
                complete: () => complete(selectedPubkey ?? undefined),
                queryClient,
              }}
            />
          )}
        </div>
      </OnboardingFooterProvider>
    </div>
  );
}
