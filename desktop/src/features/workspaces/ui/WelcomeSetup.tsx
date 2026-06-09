import * as React from "react";
import { Check, KeyRound, Sprout } from "lucide-react";
import { flushSync } from "react-dom";

import {
  getIdentity,
  importIdentity as tauriImportIdentity,
} from "@/shared/api/tauri";
import {
  type OnboardingTransitionDirection,
  OnboardingSlideTransition,
} from "@/features/onboarding/ui/OnboardingSlideTransition";
import { cn } from "@/shared/lib/cn";
import { nsecToNpub, shortenNpub } from "@/shared/lib/nostrUtils";
import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";
import { Spinner } from "@/shared/ui/spinner";
import { StepProgress } from "@/shared/ui/step-progress";
import { useSystemColorScheme } from "@/shared/theme/useSystemColorScheme";

import type { Workspace } from "../types";
import { initFirstWorkspace } from "../workspaceStorage";

type WelcomeSetupPage = "welcome" | "create-workspace" | "nostr-key";
type WelcomeTransitionMode = "initial" | OnboardingTransitionDirection;

type WelcomeSetupProps = {
  defaultRelayUrl: string;
  initialTransitionMode?: WelcomeTransitionMode;
  onComplete: (workspace: Workspace) => void;
};

const DEFAULT_WORKSPACE_HANDOFF_MIN_MS = 200;
const NOSTR_KEY_FILE_MAX_BYTES = 1024;

function wait(ms: number) {
  return new Promise<void>((resolve) => {
    window.setTimeout(resolve, ms);
  });
}

function NostrKeyImportPage({
  connectionError,
  disabled,
  onBack,
  onImport,
}: {
  connectionError: string | null;
  disabled: boolean;
  onBack: () => void;
  onImport: (nsec: string) => Promise<void>;
}) {
  const [nsecInput, setNsecInput] = React.useState("");
  const [isImporting, setIsImporting] = React.useState(false);
  const [importError, setImportError] = React.useState<string | null>(null);
  const [isDragging, setIsDragging] = React.useState(false);
  const inputRef = React.useRef<HTMLInputElement | null>(null);
  const fileInputRef = React.useRef<HTMLInputElement | null>(null);
  const previewNpub = React.useMemo(() => nsecToNpub(nsecInput), [nsecInput]);
  const trimmedInput = nsecInput.trim();
  const hasInput = trimmedInput.length > 0;
  const isValid = previewNpub !== null;
  const isBusy = disabled || isImporting;
  const showInvalidHint = hasInput && !isValid && trimmedInput.length >= 5;
  const errorMessage = importError ?? connectionError;

  React.useLayoutEffect(() => {
    inputRef.current?.focus();
  }, []);

  const openFilePicker = React.useCallback(() => {
    if (isBusy) {
      return;
    }

    fileInputRef.current?.click();
  }, [isBusy]);

  const handleFiles = React.useCallback(async (files: FileList | null) => {
    const file = files?.[0];
    if (!file) {
      return;
    }

    if (file.size > NOSTR_KEY_FILE_MAX_BYTES) {
      setImportError(
        "That file is too large to be a key. Drop a .key file or paste your nsec.",
      );
      return;
    }

    try {
      const text = await file.text();
      const firstLine =
        text.split(/\r?\n/).find((line) => line.trim().length > 0) ?? "";
      setNsecInput(firstLine.trim());
      setImportError(null);
    } catch (error) {
      setImportError(
        error instanceof Error ? error.message : "Couldn't read that file.",
      );
    }
  }, []);

  const handleSubmit = React.useCallback(async () => {
    if (!previewNpub) {
      setImportError(
        "That doesn't look like a valid nsec. Paste an nsec1 key.",
      );
      return;
    }

    setIsImporting(true);
    setImportError(null);

    try {
      await onImport(trimmedInput);
    } catch (error) {
      setImportError(
        error instanceof Error ? error.message : "Failed to import key.",
      );
    } finally {
      setIsImporting(false);
    }
  }, [onImport, previewNpub, trimmedInput]);

  return (
    <OnboardingSlideTransition
      className="flex w-full flex-col items-center text-center"
      direction="forward"
      transitionKey="nostr-key-forward"
    >
      <div className="w-full max-w-[440px]">
        <h1 className="text-3xl font-semibold tracking-tight">
          Continue using Nostr
        </h1>
        <p className="mt-3 text-sm leading-6 text-muted-foreground">
          Import an existing Nostr private key to use that identity with Sprout.
        </p>
      </div>

      <form
        className="mt-8 flex w-full flex-col gap-4"
        onSubmit={(event) => {
          event.preventDefault();
          void handleSubmit();
        }}
      >
        <div className="space-y-1.5 text-left">
          <label
            className="text-sm font-medium text-foreground"
            htmlFor="nostr-private-key"
          >
            Private key
          </label>
          <Input
            autoComplete="off"
            autoCorrect="off"
            className="h-10 bg-background"
            data-testid="welcome-nostr-nsec-input"
            id="nostr-private-key"
            onChange={(event) => {
              setNsecInput(event.target.value);
              setImportError(null);
            }}
            placeholder="nsec1..."
            ref={inputRef}
            spellCheck={false}
            type="password"
            value={nsecInput}
          />
        </div>

        <input
          accept=".key,text/plain"
          className="sr-only"
          disabled={isBusy}
          onChange={(event) => {
            void handleFiles(event.currentTarget.files);
            event.currentTarget.value = "";
          }}
          ref={fileInputRef}
          tabIndex={-1}
          type="file"
        />

        <button
          className={cn(
            "relative flex h-[120px] flex-col items-center justify-center gap-3 overflow-hidden rounded-xl border border-transparent bg-muted text-foreground transition-[background-color,border-color,box-shadow,color] duration-[250ms] ease-out hover:bg-muted/80 disabled:opacity-60",
            isDragging &&
              "border-primary bg-primary/10 text-primary ring-1 ring-primary/35 hover:bg-primary/10",
          )}
          data-dragging={isDragging ? "true" : undefined}
          data-testid="welcome-nostr-drop"
          disabled={isBusy}
          onClick={openFilePicker}
          onDragEnter={(event) => {
            event.preventDefault();
            event.stopPropagation();
            if (!isBusy) {
              setIsDragging(true);
            }
          }}
          onDragLeave={(event) => {
            event.preventDefault();
            event.stopPropagation();
            if (
              event.currentTarget.contains(event.relatedTarget as Node | null)
            ) {
              return;
            }
            setIsDragging(false);
          }}
          onDragOver={(event) => {
            event.preventDefault();
            event.stopPropagation();
            if (!isBusy) {
              event.dataTransfer.dropEffect = "copy";
            }
          }}
          onDrop={(event) => {
            event.preventDefault();
            event.stopPropagation();
            setIsDragging(false);
            if (isBusy) {
              return;
            }
            void handleFiles(event.dataTransfer.files);
          }}
          type="button"
        >
          <span
            aria-hidden="true"
            className={cn(
              "pointer-events-none absolute inset-0 rounded-[inherit] bg-primary/10 opacity-0 transition-opacity duration-[250ms] ease-out",
              isDragging && "opacity-100",
            )}
          />
          <KeyRound
            className={cn(
              "relative h-8 w-8 text-muted-foreground transition-colors duration-[250ms] ease-out",
              isDragging && "text-primary",
            )}
          />
          <span
            className={cn(
              "relative text-sm font-medium text-muted-foreground transition-colors duration-[250ms] ease-out",
              isDragging && "text-primary",
            )}
          >
            Drop a key here
          </span>
        </button>

        {previewNpub ? (
          <div
            className="flex items-start gap-2 rounded-md border border-primary/30 bg-primary/5 px-3 py-2 text-xs"
            data-testid="welcome-nostr-npub-preview"
          >
            <Check className="mt-0.5 h-3.5 w-3.5 shrink-0 text-primary" />
            <div className="min-w-0 space-y-0.5">
              <p className="font-medium text-foreground">
                This will use this Nostr identity:
              </p>
              <p className="break-all font-mono text-[11px] text-muted-foreground">
                {shortenNpub(previewNpub)}
              </p>
            </div>
          </div>
        ) : null}

        {showInvalidHint && !errorMessage ? (
          <p className="text-xs text-muted-foreground">
            Waiting for a valid nsec1 key.
          </p>
        ) : null}

        {errorMessage ? (
          <p className="text-center text-sm text-destructive">{errorMessage}</p>
        ) : null}

        <div className="flex w-full flex-col gap-3 pt-1">
          <Button
            className="h-10 w-full"
            data-testid="welcome-nostr-submit"
            disabled={!isValid || isBusy}
            type="submit"
          >
            {isBusy ? (
              <Spinner aria-label="Importing key" className="h-4 w-4" />
            ) : (
              "Continue using Nostr"
            )}
          </Button>

          <Button
            className="h-10 w-full text-muted-foreground hover:text-accent-foreground"
            disabled={isBusy}
            onClick={onBack}
            type="button"
            variant="ghost"
          >
            Back
          </Button>
        </div>
      </form>
    </OnboardingSlideTransition>
  );
}

export function WelcomeSetup({
  defaultRelayUrl,
  initialTransitionMode = "initial",
  onComplete,
}: WelcomeSetupProps) {
  const [page, setPage] = React.useState<WelcomeSetupPage>("welcome");
  const [transitionMode, setTransitionMode] =
    React.useState<WelcomeTransitionMode>(initialTransitionMode);
  const [customWorkspaceName, setCustomWorkspaceName] = React.useState("");
  const [customRelayUrl, setCustomRelayUrl] = React.useState("");
  const [isConnecting, setIsConnecting] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  const systemColorScheme = useSystemColorScheme();

  const handleConnect = React.useCallback(
    async (relayUrl: string, workspaceName?: string, pubkey?: string) => {
      const trimmedUrl = relayUrl.trim();
      if (!trimmedUrl) {
        setError("Please enter a workspace URL.");
        return;
      }

      const handoffStartedAt = performance.now();
      flushSync(() => {
        setIsConnecting(true);
        setError(null);
      });

      try {
        // We snapshot only the pubkey for display purposes (workspace switcher
        // labels, etc.). The private key lives on disk in `identity.key` and
        // is the single source of truth — never copied into localStorage.
        const identityPubkey = pubkey ?? (await getIdentity()).pubkey;
        const workspace = initFirstWorkspace(
          trimmedUrl,
          identityPubkey,
          workspaceName,
        );

        if (!workspaceName) {
          const elapsedMs = performance.now() - handoffStartedAt;
          if (elapsedMs < DEFAULT_WORKSPACE_HANDOFF_MIN_MS) {
            await wait(DEFAULT_WORKSPACE_HANDOFF_MIN_MS - elapsedMs);
          }
        }

        // The parent moves this workspace into React state so first-run setup
        // can continue without a full page reload.
        onComplete(workspace);
      } catch (err) {
        setError(
          err instanceof Error ? err.message : "Failed to connect. Try again.",
        );
        setIsConnecting(false);
      }
    },
    [onComplete],
  );

  const handleNostrImport = React.useCallback(
    async (nsec: string) => {
      const identity = await tauriImportIdentity(nsec);
      await handleConnect(defaultRelayUrl, undefined, identity.pubkey);
    },
    [defaultRelayUrl, handleConnect],
  );

  const handleCustomWorkspaceSubmit = React.useCallback(
    (event: React.FormEvent<HTMLFormElement>) => {
      event.preventDefault();
      const trimmedName = customWorkspaceName.trim();
      const trimmedUrl = customRelayUrl.trim();
      if (!trimmedName) {
        setError("Please enter a workspace name.");
        return;
      }
      if (!trimmedUrl) {
        setError("Please enter a workspace URL.");
        return;
      }
      void handleConnect(trimmedUrl, trimmedName);
    },
    [customRelayUrl, customWorkspaceName, handleConnect],
  );

  const showCreateWorkspacePage = React.useCallback(() => {
    setError(null);
    setTransitionMode("forward");
    setPage("create-workspace");
  }, []);

  const showNostrKeyPage = React.useCallback(() => {
    setError(null);
    setTransitionMode("forward");
    setPage("nostr-key");
  }, []);

  const showWelcomePage = React.useCallback(() => {
    setError(null);
    setTransitionMode("backward");
    setPage("welcome");
  }, []);

  const currentStep =
    page === "welcome" ? (isConnecting ? 2 : 1) : page === "nostr-key" ? 1 : 2;
  const transitionDirection =
    transitionMode === "backward" ? "backward" : "forward";
  const welcomeEffect =
    transitionMode === "backward" ? "line-slide" : "mask-reveal-up";

  return (
    <div
      className="sprout-onboarding-neutral-theme sprout-startup-shell flex items-center justify-center bg-background px-4 py-8 text-foreground"
      data-system-color-scheme={systemColorScheme}
    >
      <div className="relative flex w-full max-w-[500px] flex-col items-center text-center">
        <StepProgress
          activeSegmentClassName="bg-primary"
          className="fixed bottom-12 left-1/2 z-40 -translate-x-1/2"
          completeSegmentClassName="bg-primary/35"
          currentStep={currentStep}
          inactiveSegmentClassName="bg-muted-foreground/25"
        />

        {page === "welcome" ? (
          <OnboardingSlideTransition
            className="flex w-full flex-col items-center text-center"
            direction={transitionDirection}
            effect={welcomeEffect}
            transitionKey={`welcome-${welcomeEffect}-${transitionDirection}`}
          >
            <div className="flex h-14 w-14 items-center justify-center rounded-lg border border-border bg-background text-foreground shadow-xs">
              <Sprout className="h-7 w-7" aria-hidden="true" />
            </div>

            <h1 className="mt-6 text-3xl font-semibold tracking-tight">
              Welcome to Sprout
            </h1>
            <p className="mt-3 max-w-[440px] text-sm leading-6 text-muted-foreground">
              Choose your first workspace to get started.
            </p>

            <div className="mt-8 flex w-full flex-col gap-3">
              <Button
                className="h-10 w-full"
                aria-disabled={isConnecting}
                onClick={() => {
                  if (isConnecting) {
                    return;
                  }
                  setError(null);
                  void handleConnect(defaultRelayUrl);
                }}
                type="button"
              >
                Continue with Block Inc. workspace
              </Button>

              <Button
                className="h-10 w-full"
                aria-disabled={isConnecting}
                onClick={() => {
                  if (isConnecting) {
                    return;
                  }
                  showCreateWorkspacePage();
                }}
                type="button"
                variant="secondary"
              >
                Create a new workspace
              </Button>

              <Button
                className="h-10 w-full"
                aria-disabled={isConnecting}
                data-testid="welcome-continue-nostr"
                onClick={() => {
                  if (isConnecting) {
                    return;
                  }
                  showNostrKeyPage();
                }}
                type="button"
                variant="ghost"
              >
                Continue using Nostr
              </Button>
            </div>

            {error ? (
              <div className="mt-4 w-full">
                <p className="text-sm text-destructive">{error}</p>
              </div>
            ) : null}
          </OnboardingSlideTransition>
        ) : page === "create-workspace" ? (
          <OnboardingSlideTransition
            className="flex w-full flex-col items-center text-center"
            direction={transitionDirection}
            transitionKey={`create-workspace-${transitionDirection}`}
          >
            <div className="w-full max-w-[440px]">
              <h1 className="text-3xl font-semibold tracking-tight">
                Create a new workspace
              </h1>
              <p className="mt-3 text-sm leading-6 text-muted-foreground">
                Workspaces are where teammates and agents collaborate across
                channels, DMs, and shared projects.
              </p>
            </div>

            <form
              className="mt-8 flex w-full flex-col gap-4"
              onSubmit={handleCustomWorkspaceSubmit}
            >
              <div className="space-y-1.5 text-left">
                <label
                  className="text-sm font-medium text-foreground"
                  htmlFor="workspace-name"
                >
                  Workspace name
                </label>
                <Input
                  autoFocus
                  className="h-10 bg-background"
                  id="workspace-name"
                  onChange={(event) => {
                    setCustomWorkspaceName(event.target.value);
                    setError(null);
                  }}
                  placeholder="Design team"
                  type="text"
                  value={customWorkspaceName}
                />
              </div>

              <div className="space-y-1.5 text-left">
                <label
                  className="text-sm font-medium text-foreground"
                  htmlFor="workspace-url"
                >
                  Workspace URL
                </label>
                <Input
                  className="h-10 bg-background"
                  id="workspace-url"
                  onChange={(event) => {
                    setCustomRelayUrl(event.target.value);
                    setError(null);
                  }}
                  placeholder="wss://relay.example.com"
                  type="text"
                  value={customRelayUrl}
                />
              </div>

              <div className="flex w-full flex-col gap-3 pt-1">
                <Button
                  className="h-10 w-full"
                  disabled={
                    isConnecting ||
                    !customWorkspaceName.trim() ||
                    !customRelayUrl.trim()
                  }
                  type="submit"
                >
                  {isConnecting ? (
                    <Spinner
                      aria-label="Creating workspace"
                      className="h-4 w-4"
                    />
                  ) : (
                    "Create workspace"
                  )}
                </Button>

                <Button
                  className="h-10 w-full text-muted-foreground hover:text-accent-foreground"
                  disabled={isConnecting}
                  onClick={showWelcomePage}
                  type="button"
                  variant="ghost"
                >
                  Back
                </Button>

                {error ? (
                  <p className="text-center text-sm text-destructive">
                    {error}
                  </p>
                ) : null}
              </div>
            </form>
          </OnboardingSlideTransition>
        ) : (
          <NostrKeyImportPage
            connectionError={error}
            disabled={isConnecting}
            onBack={showWelcomePage}
            onImport={handleNostrImport}
          />
        )}
      </div>
    </div>
  );
}
