import * as DialogPrimitive from "@radix-ui/react-dialog";
import { openUrl } from "@tauri-apps/plugin-opener";
import { ChevronDown } from "lucide-react";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import * as React from "react";
import { toast } from "sonner";

import { getIdentity } from "@/shared/api/tauriIdentity";
import type { Identity } from "@/shared/api/types";
import type { NostrBindDeepLinkPayload } from "@/shared/deep-link";
import { listenForNostrBindDeepLinks } from "@/shared/deep-link";
import { OnboardingSlideTransition } from "@/features/onboarding/ui/OnboardingSlideTransition";
import { buildNostrBindCallbackUrl } from "@/features/profile/lib/nostrBindCallback";
import { signNostrIdentityBinding } from "@/features/profile/lib/nostrIdentityBinding";
import { cn } from "@/shared/lib/cn";
import { useSystemColorScheme } from "@/shared/theme/useSystemColorScheme";
import { Button } from "@/shared/ui/button";
import { StartupWindowDragRegion } from "@/shared/ui/StartupWindowDragRegion";

const COPY_SUCCESS_MESSAGE =
  "Signed response copied. Paste it into the Buzz admin console.";
const PREVIEW_COPY_SUCCESS_MESSAGE = "Preview response copied.";
const COPY_FAILURE_MESSAGE = "Buzz couldn't access the clipboard. Try again.";
const EXPIRED_LINK_MESSAGE =
  "This binding link has expired. Request a new one from the requesting app.";
const VERIFICATION_CODE_LENGTH = 6;
const VERIFICATION_CODE_DIGIT_KEYS = ["1", "2", "3", "4", "5", "6"] as const;
const VERIFICATION_CODE_MISMATCH_MESSAGE =
  "That code doesn't match. Check the code and try again.";
const COPY_BUTTON_LABEL_CLASS =
  "col-start-1 row-start-1 transition-[opacity,transform] duration-150 ease-[cubic-bezier(0.23,1,0.32,1)] motion-reduce:translate-y-0 motion-reduce:duration-0";
const NOSTR_BIND_PREVIEW_PAYLOAD: NostrBindDeepLinkPayload = {
  challengeId: "550e8400-e29b-41d4-a716-446655440000",
  nonce: "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghi01234567",
  verificationCode: "123456",
  audience: "buzz:nostr-identity",
  action: "bind_nostr_identity",
  protocol: "buzz-nostr-identity",
  version: "1",
  origin: "https://example.com",
  expiresAt: "2099-01-01T00:00:00Z",
  returnMode: "clipboard",
};
const NOSTR_BIND_PREVIEW_IDENTITY: Identity = {
  pubkey: "deadbeef".repeat(8),
  displayName: "Preview identity",
};
const NOSTR_BIND_PREVIEW_SIGNED_RESPONSE = JSON.stringify({
  id: "preview-only-not-a-real-signature",
  pubkey: NOSTR_BIND_PREVIEW_IDENTITY.pubkey,
  created_at: 0,
  kind: 24243,
  tags: [
    ["challenge_id", NOSTR_BIND_PREVIEW_PAYLOAD.challengeId],
    ["nonce", NOSTR_BIND_PREVIEW_PAYLOAD.nonce],
    ["verification_code", NOSTR_BIND_PREVIEW_PAYLOAD.verificationCode],
    ["audience", NOSTR_BIND_PREVIEW_PAYLOAD.audience],
    ["action", NOSTR_BIND_PREVIEW_PAYLOAD.action],
    ["protocol", NOSTR_BIND_PREVIEW_PAYLOAD.protocol],
    ["version", NOSTR_BIND_PREVIEW_PAYLOAD.version],
    ["origin", NOSTR_BIND_PREVIEW_PAYLOAD.origin],
    ["expires_at", NOSTR_BIND_PREVIEW_PAYLOAD.expiresAt],
  ],
  content: "",
  sig: "preview-only-not-a-real-signature",
});

function isNostrBindPreviewEnabled(): boolean {
  if (!import.meta.env.DEV) {
    return false;
  }

  return (
    import.meta.env.VITE_NOSTR_BIND_PREVIEW === "1" ||
    new URLSearchParams(window.location.search).get("preview") === "nostr-bind"
  );
}

function createEmptyVerificationCode(): string[] {
  return Array.from({ length: VERIFICATION_CODE_LENGTH }, () => "");
}

function normalizeVerificationCode(value: string): string[] {
  return value
    .replace(/\D/g, "")
    .slice(0, VERIFICATION_CODE_LENGTH)
    .padEnd(VERIFICATION_CODE_LENGTH, " ")
    .split("")
    .map((character) => character.trim());
}

function isNostrBindRequestExpired(payload: NostrBindDeepLinkPayload): boolean {
  const expiry = new Date(payload.expiresAt).getTime();
  return Number.isNaN(expiry) || expiry <= Date.now();
}

function formatError(error: unknown): string {
  if (error instanceof Error) {
    return error.message;
  }
  return String(error);
}

async function copyToClipboard(text: string): Promise<boolean> {
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch (error) {
    console.warn("copy signed nostr binding response failed:", error);
    return false;
  }
}

function appendCallbackStatus(callbackUrl: string): string {
  const url = new URL(callbackUrl);
  url.searchParams.set("buzz_bind", "signed");
  return url.toString();
}

async function notifySignedResponseReady(callbackUrl: string | undefined) {
  if (!callbackUrl) {
    return;
  }
  try {
    await openUrl(appendCallbackStatus(callbackUrl));
  } catch (error) {
    console.warn("open nostr bind callback failed:", error);
  }
}

async function returnSignedResponseToBrowser(
  callbackUrl: string,
  signedResponse: string,
): Promise<string | null> {
  try {
    await openUrl(buildNostrBindCallbackUrl(callbackUrl, signedResponse));
    return null;
  } catch (error) {
    console.warn("return signed nostr binding response failed:", error);
    return "Could not open the browser. Copy the response below to finish manually.";
  }
}

function SignedResponseControls({
  copyFailed,
  copyLabel,
  onCopy,
  signedResponse,
}: {
  copyFailed: boolean;
  copyLabel: string;
  onCopy: () => void;
  signedResponse: string;
}) {
  return (
    <div className="space-y-4" data-testid="nostr-bind-manual-fallback-content">
      <pre
        className="max-h-56 w-full overflow-auto rounded-2xl border border-border/70 bg-muted/60 p-4 text-left shadow-xs"
        data-testid="nostr-bind-signed-response"
      >
        <code className="whitespace-pre-wrap break-all font-mono text-xs leading-5 text-foreground">
          {signedResponse}
        </code>
      </pre>

      {copyFailed ? (
        <p className="w-full rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-left text-sm text-destructive">
          {COPY_FAILURE_MESSAGE}
        </p>
      ) : null}

      <Button
        aria-label={copyLabel}
        className="h-10 w-full"
        data-testid="nostr-bind-copy-response"
        onClick={onCopy}
        type="button"
      >
        <span aria-live="polite" className="sr-only">
          {copyLabel}
        </span>
        <span
          aria-hidden="true"
          className="inline-grid h-5 place-items-center overflow-hidden"
        >
          <span
            className={cn(
              COPY_BUTTON_LABEL_CLASS,
              copyLabel === "Copy response"
                ? "translate-y-0 opacity-100"
                : "-translate-y-0.5 opacity-0",
            )}
          >
            Copy response
          </span>
          <span
            className={cn(
              COPY_BUTTON_LABEL_CLASS,
              copyLabel === "Copied"
                ? "translate-y-0 opacity-100"
                : "translate-y-0.5 opacity-0",
            )}
          >
            Copied
          </span>
        </span>
      </Button>
    </div>
  );
}

export function NostrBindConsentDialog() {
  const isPreview = isNostrBindPreviewEnabled();
  const [payload, setPayload] = React.useState<NostrBindDeepLinkPayload | null>(
    isPreview ? NOSTR_BIND_PREVIEW_PAYLOAD : null,
  );
  const [identity, setIdentity] = React.useState<Identity | null>(
    isPreview ? NOSTR_BIND_PREVIEW_IDENTITY : null,
  );
  const [isSigning, setIsSigning] = React.useState(false);
  const [signedResponse, setSignedResponse] = React.useState<string | null>(
    null,
  );
  const [isCopied, setIsCopied] = React.useState(false);
  const [verificationCode, setVerificationCode] = React.useState<string[]>(
    createEmptyVerificationCode,
  );
  const [hasCodeMismatch, setHasCodeMismatch] = React.useState(false);
  const [copyFailed, setCopyFailed] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  const [isManualFallbackOpen, setIsManualFallbackOpen] = React.useState(false);
  const codeInputRefs = React.useRef<Array<HTMLInputElement | null>>([]);
  const codeShakeRef = React.useRef<HTMLDivElement | null>(null);
  const codeShakeAnimationRef = React.useRef<Animation | null>(null);
  const copiedTimerRef = React.useRef<ReturnType<typeof setTimeout> | null>(
    null,
  );
  const autoSignAttemptRef = React.useRef<string | null>(null);
  const activeSignAttemptRef = React.useRef<symbol | null>(null);
  const systemColorScheme = useSystemColorScheme();
  const shouldReduceMotion = useReducedMotion();
  const enteredVerificationCode = verificationCode.join("");
  const isVerificationCodeComplete =
    enteredVerificationCode.length === VERIFICATION_CODE_LENGTH;
  const isVerificationCodeValid =
    payload !== null && enteredVerificationCode === payload.verificationCode;
  const copyButtonLabel = isSigning ? "Signing…" : "Continue";
  const finishCopyButtonLabel = isCopied ? "Copied" : "Copy response";

  const clearCopiedState = React.useCallback(() => {
    if (copiedTimerRef.current) {
      clearTimeout(copiedTimerRef.current);
      copiedTimerRef.current = null;
    }
    setIsCopied(false);
  }, []);

  const showCopiedState = React.useCallback(() => {
    clearCopiedState();
    setIsCopied(true);
    copiedTimerRef.current = setTimeout(() => {
      setIsCopied(false);
      copiedTimerRef.current = null;
    }, 2_000);
  }, [clearCopiedState]);

  React.useEffect(
    () => () => {
      codeShakeAnimationRef.current?.cancel();
      if (copiedTimerRef.current) {
        clearTimeout(copiedTimerRef.current);
      }
    },
    [],
  );

  React.useEffect(() => {
    if (isPreview) {
      return;
    }

    const unlistenPromise = listenForNostrBindDeepLinks((nextPayload) => {
      clearCopiedState();
      autoSignAttemptRef.current = null;
      activeSignAttemptRef.current = null;
      setPayload(nextPayload);
      setIdentity(null);
      setIsSigning(false);
      setSignedResponse(null);
      setVerificationCode(createEmptyVerificationCode());
      setHasCodeMismatch(false);
      setCopyFailed(false);
      setError(null);
      setIsManualFallbackOpen(false);
      getIdentity()
        .then(setIdentity)
        .catch((error) => {
          console.warn("get_identity for nostr bind failed:", error);
          setIdentity(null);
          setError("Could not load the current Buzz identity.");
        });
    });

    return () => {
      void unlistenPromise.then((unlisten) => unlisten());
    };
  }, [clearCopiedState, isPreview]);

  const isExpired = payload !== null && isNostrBindRequestExpired(payload);

  const resetDialog = React.useCallback(() => {
    clearCopiedState();
    autoSignAttemptRef.current = null;
    activeSignAttemptRef.current = null;
    setPayload(null);
    setSignedResponse(null);
    setVerificationCode(createEmptyVerificationCode());
    setHasCodeMismatch(false);
    setCopyFailed(false);
    setError(null);
    setIsManualFallbackOpen(false);
    setIdentity(null);
    setIsSigning(false);
  }, [clearCopiedState]);

  const handleOpenChange = React.useCallback(
    (open: boolean) => {
      if (!open && !isPreview) {
        resetDialog();
      }
    },
    [isPreview, resetDialog],
  );

  const shakeVerificationCode = React.useCallback(() => {
    if (shouldReduceMotion || !codeShakeRef.current) {
      return;
    }

    codeShakeAnimationRef.current?.cancel();
    codeShakeAnimationRef.current = codeShakeRef.current.animate(
      [
        {
          easing: "cubic-bezier(0.22, 1, 0.36, 1)",
          offset: 0,
          transform: "translateX(0px)",
        },
        {
          easing: "cubic-bezier(0.22, 1, 0.36, 1)",
          offset: 0.2857,
          transform: "translateX(6px)",
        },
        {
          easing: "cubic-bezier(0.22, 1, 0.36, 1)",
          offset: 0.5714,
          transform: "translateX(-6px)",
        },
        {
          easing: "cubic-bezier(0.22, 1, 0.36, 1)",
          offset: 0.7857,
          transform: "translateX(4px)",
        },
        { offset: 1, transform: "translateX(0px)" },
      ],
      { duration: 280, easing: "linear" },
    );
  }, [shouldReduceMotion]);

  const showVerificationCodeMismatch = React.useCallback(() => {
    setHasCodeMismatch(true);
    shakeVerificationCode();
  }, [shakeVerificationCode]);

  const handleVerificationCodeChange = React.useCallback(
    (index: number, value: string) => {
      const nextDigits = value.replace(/\D/g, "");
      const next = [...verificationCode];
      autoSignAttemptRef.current = null;

      if (!nextDigits) {
        next[index] = "";
        setVerificationCode(next);
        setHasCodeMismatch(false);
        return;
      }

      if (index === VERIFICATION_CODE_LENGTH - 1 && verificationCode[index]) {
        shakeVerificationCode();
        return;
      }

      for (
        let offset = 0;
        offset < nextDigits.length && index + offset < next.length;
        offset += 1
      ) {
        next[index + offset] = nextDigits[offset] ?? "";
      }
      setVerificationCode(next);

      const completedCode = next.join("");
      if (
        completedCode.length === VERIFICATION_CODE_LENGTH &&
        completedCode !== payload?.verificationCode
      ) {
        showVerificationCodeMismatch();
      } else {
        setHasCodeMismatch(false);
      }

      const nextIndex = Math.min(
        index + nextDigits.length,
        VERIFICATION_CODE_LENGTH - 1,
      );
      codeInputRefs.current[nextIndex]?.focus();
      codeInputRefs.current[nextIndex]?.select();
    },
    [
      payload?.verificationCode,
      shakeVerificationCode,
      showVerificationCodeMismatch,
      verificationCode,
    ],
  );

  const handleVerificationCodePaste = React.useCallback(
    (index: number, event: React.ClipboardEvent<HTMLInputElement>) => {
      if (index === VERIFICATION_CODE_LENGTH - 1 && verificationCode[index]) {
        event.preventDefault();
        if (/\d/.test(event.clipboardData.getData("text"))) {
          shakeVerificationCode();
        }
        return;
      }

      const pastedCode = event.clipboardData
        .getData("text")
        .replace(/\D/g, "")
        .slice(0, VERIFICATION_CODE_LENGTH);
      if (!pastedCode) {
        return;
      }

      event.preventDefault();
      autoSignAttemptRef.current = null;
      const next = normalizeVerificationCode(pastedCode);
      setVerificationCode(next);
      if (
        pastedCode.length === VERIFICATION_CODE_LENGTH &&
        pastedCode !== payload?.verificationCode
      ) {
        showVerificationCodeMismatch();
      } else {
        setHasCodeMismatch(false);
      }
      const nextIndex = Math.min(
        pastedCode.length,
        VERIFICATION_CODE_LENGTH - 1,
      );
      codeInputRefs.current[nextIndex]?.focus();
      codeInputRefs.current[nextIndex]?.select();
    },
    [
      payload?.verificationCode,
      shakeVerificationCode,
      showVerificationCodeMismatch,
      verificationCode,
    ],
  );

  const handleVerificationCodeKeyDown = React.useCallback(
    (index: number, event: React.KeyboardEvent<HTMLInputElement>) => {
      if (
        index === VERIFICATION_CODE_LENGTH - 1 &&
        verificationCode[index] &&
        /^\d$/.test(event.key)
      ) {
        event.preventDefault();
        shakeVerificationCode();
        return;
      }

      if (event.key === "Backspace") {
        event.preventDefault();
        autoSignAttemptRef.current = null;
        const targetIndex = verificationCode[index]
          ? index
          : Math.max(index - 1, 0);
        setVerificationCode((current) => {
          const next = [...current];
          next[targetIndex] = "";
          return next;
        });
        setHasCodeMismatch(false);
        codeInputRefs.current[targetIndex]?.focus();
        return;
      }

      if (event.key === "ArrowLeft") {
        event.preventDefault();
        codeInputRefs.current[Math.max(index - 1, 0)]?.focus();
      } else if (event.key === "ArrowRight") {
        event.preventDefault();
        codeInputRefs.current[
          Math.min(index + 1, VERIFICATION_CODE_LENGTH - 1)
        ]?.focus();
      }
    },
    [shakeVerificationCode, verificationCode],
  );

  const handleSign = React.useCallback(async () => {
    if (activeSignAttemptRef.current || !payload) {
      return;
    }
    if (isNostrBindRequestExpired(payload)) {
      setError(EXPIRED_LINK_MESSAGE);
      return;
    }
    if (!isVerificationCodeValid) {
      if (isVerificationCodeComplete) {
        showVerificationCodeMismatch();
      }
      const firstEmptyIndex = verificationCode.findIndex((digit) => !digit);
      codeInputRefs.current[
        firstEmptyIndex === -1 ? 0 : firstEmptyIndex
      ]?.focus();
      return;
    }

    const attempt = Symbol("nostr-bind-sign");
    activeSignAttemptRef.current = attempt;
    setIsSigning(true);
    clearCopiedState();
    setError(null);
    setCopyFailed(false);
    try {
      const signed = isPreview
        ? NOSTR_BIND_PREVIEW_SIGNED_RESPONSE
        : await signNostrIdentityBinding({
            challengeId: payload.challengeId,
            nonce: payload.nonce,
            verificationCode: enteredVerificationCode,
            origin: payload.origin,
            expiresAt: payload.expiresAt,
          });
      if (activeSignAttemptRef.current !== attempt) {
        return;
      }

      setSignedResponse(signed);
      if (payload.returnMode === "browser_fragment_v1" && payload.callbackUrl) {
        const callbackError = await returnSignedResponseToBrowser(
          payload.callbackUrl,
          signed,
        );
        if (activeSignAttemptRef.current !== attempt) {
          return;
        }
        setError(callbackError);
        setIsManualFallbackOpen(callbackError !== null);
      }
    } catch (error) {
      if (activeSignAttemptRef.current === attempt) {
        setError(formatError(error) || "Failed to sign binding response.");
      }
    } finally {
      if (activeSignAttemptRef.current === attempt) {
        activeSignAttemptRef.current = null;
        setIsSigning(false);
      }
    }
  }, [
    clearCopiedState,
    enteredVerificationCode,
    isPreview,
    isVerificationCodeComplete,
    isVerificationCodeValid,
    payload,
    showVerificationCodeMismatch,
    verificationCode,
  ]);

  React.useEffect(() => {
    if (
      !payload ||
      identity === null ||
      isExpired ||
      isSigning ||
      signedResponse !== null ||
      !isVerificationCodeValid
    ) {
      return;
    }

    const attemptKey = `${payload.challengeId}:${enteredVerificationCode}`;
    if (autoSignAttemptRef.current === attemptKey) {
      return;
    }
    autoSignAttemptRef.current = attemptKey;
    void handleSign();
  }, [
    enteredVerificationCode,
    handleSign,
    identity,
    isExpired,
    isSigning,
    isVerificationCodeValid,
    payload,
    signedResponse,
  ]);

  const handleCopyAgain = React.useCallback(async () => {
    if (!signedResponse) {
      return;
    }
    const copied = await copyToClipboard(signedResponse);
    setCopyFailed(!copied);
    if (copied) {
      showCopiedState();
      if (payload?.returnMode === "clipboard") {
        await notifySignedResponseReady(payload.callbackUrl);
      }
      toast.success(
        isPreview ? PREVIEW_COPY_SUCCESS_MESSAGE : COPY_SUCCESS_MESSAGE,
      );
    } else {
      toast.warning(COPY_FAILURE_MESSAGE);
    }
  }, [
    isPreview,
    payload?.callbackUrl,
    payload?.returnMode,
    showCopiedState,
    signedResponse,
  ]);

  return (
    <DialogPrimitive.Root
      onOpenChange={handleOpenChange}
      open={payload !== null}
    >
      <DialogPrimitive.Portal>
        {payload ? (
          <DialogPrimitive.Content
            aria-describedby="nostr-bind-description"
            className="buzz-onboarding-neutral-theme buzz-startup-shell fixed inset-0 z-50 flex overflow-y-auto bg-background px-4 py-12 text-foreground outline-hidden"
            data-system-color-scheme={systemColorScheme}
            data-testid="nostr-bind-page"
          >
            <StartupWindowDragRegion />
            <div className="m-auto flex w-full max-w-[500px] flex-col items-center text-center">
              <img
                alt="Buzz"
                className="h-14 w-14 rounded-xl shadow-xs"
                src="/app-icon@2x.png"
                srcSet="/app-icon@2x.png 1x, /app-icon@3x.png 2x"
              />

              {signedResponse ? (
                <OnboardingSlideTransition
                  className="flex w-full flex-col items-center text-center"
                  data-testid="nostr-bind-finish-step"
                  direction="forward"
                  transitionKey="nostr-bind-finish"
                >
                  <DialogPrimitive.Title className="mt-6 text-3xl font-semibold tracking-tight">
                    {payload.returnMode === "browser_fragment_v1"
                      ? "Continue in your browser"
                      : "Finish on the Buzz website"}
                  </DialogPrimitive.Title>
                  <DialogPrimitive.Description
                    className="mt-3 max-w-[440px] text-sm leading-6 text-muted-foreground"
                    id="nostr-bind-description"
                  >
                    {payload.returnMode === "browser_fragment_v1"
                      ? "Buzz opened your browser to finish verification."
                      : "Copy the response below, then paste it into the Buzz website to finish verification."}
                  </DialogPrimitive.Description>

                  {error ? (
                    <p className="mt-4 w-full rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-left text-sm text-destructive">
                      {error}
                    </p>
                  ) : null}

                  {payload.returnMode === "browser_fragment_v1" ? (
                    <details
                      className="group mt-10 w-full overflow-hidden rounded-2xl border border-border/70 bg-muted/30 text-left shadow-xs"
                      data-testid="nostr-bind-manual-fallback"
                      onToggle={(event) =>
                        setIsManualFallbackOpen(event.currentTarget.open)
                      }
                      open={isManualFallbackOpen}
                    >
                      <summary className="flex cursor-pointer list-none items-center justify-between gap-4 px-4 py-3 text-sm font-medium transition-colors hover:bg-muted/40 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring [&::-webkit-details-marker]:hidden">
                        <span>Pairing didn’t finish automatically?</span>
                        <ChevronDown className="h-4 w-4 shrink-0 text-muted-foreground transition-transform duration-150 group-open:rotate-180" />
                      </summary>
                      <div className="space-y-4 border-t border-border/55 p-4">
                        <p className="text-sm leading-6 text-muted-foreground">
                          Copy this response and paste it into the pairing page.
                        </p>
                        <SignedResponseControls
                          copyFailed={copyFailed}
                          copyLabel={finishCopyButtonLabel}
                          onCopy={handleCopyAgain}
                          signedResponse={signedResponse}
                        />
                      </div>
                    </details>
                  ) : (
                    <div className="mt-10 w-full">
                      <SignedResponseControls
                        copyFailed={copyFailed}
                        copyLabel={finishCopyButtonLabel}
                        onCopy={handleCopyAgain}
                        signedResponse={signedResponse}
                      />
                    </div>
                  )}

                  <div className="mt-8 flex w-full flex-col gap-3">
                    <Button
                      className="h-10 w-full text-muted-foreground hover:text-accent-foreground"
                      onClick={() => handleOpenChange(false)}
                      type="button"
                      variant="ghost"
                    >
                      Continue
                    </Button>
                  </div>
                </OnboardingSlideTransition>
              ) : (
                <OnboardingSlideTransition
                  className="flex w-full flex-col items-center text-center"
                  data-testid="nostr-bind-code-step"
                  direction="forward"
                  transitionKey="nostr-bind-code"
                >
                  <DialogPrimitive.Title className="mt-6 text-3xl font-semibold tracking-tight">
                    Enter verification code
                  </DialogPrimitive.Title>
                  <DialogPrimitive.Description
                    className="mt-3 max-w-[440px] text-sm leading-6 text-muted-foreground"
                    id="nostr-bind-description"
                  >
                    Enter the six-digit code shown in your browser
                  </DialogPrimitive.Description>

                  <div className="mt-10 w-full space-y-4 text-sm">
                    <fieldset
                      aria-describedby={
                        hasCodeMismatch ? "nostr-bind-code-error" : undefined
                      }
                      aria-invalid={hasCodeMismatch}
                      className="w-full min-w-0 text-center"
                    >
                      <legend className="sr-only">Verification code</legend>
                      <div
                        className="flex justify-center gap-2"
                        data-testid="nostr-bind-verification-code"
                        ref={codeShakeRef}
                      >
                        {verificationCode.map((digit, index) => (
                          <div
                            className="relative h-16 w-14 shrink-0 overflow-hidden rounded-xl"
                            key={VERIFICATION_CODE_DIGIT_KEYS[index]}
                          >
                            <input
                              aria-label={`Verification code digit ${index + 1} of ${VERIFICATION_CODE_LENGTH}`}
                              autoComplete={
                                index === 0 ? "one-time-code" : "off"
                              }
                              className={cn(
                                "absolute inset-0 h-full w-full rounded-xl border text-center text-transparent shadow-xs caret-transparent selection:bg-transparent selection:text-transparent transition-[border-color,box-shadow] focus-visible:outline-hidden disabled:cursor-not-allowed disabled:opacity-50",
                                systemColorScheme === "light"
                                  ? "bg-[#fafafa]"
                                  : "bg-muted",
                                hasCodeMismatch
                                  ? "border-destructive focus-visible:border-destructive focus-visible:ring-2 focus-visible:ring-destructive/25"
                                  : "border-input/60 focus-visible:border-ring focus-visible:ring-2 focus-visible:ring-ring/30",
                              )}
                              data-testid={`nostr-bind-code-digit-${index + 1}`}
                              disabled={isSigning}
                              inputMode="numeric"
                              maxLength={1}
                              onChange={(event) =>
                                handleVerificationCodeChange(
                                  index,
                                  event.target.value,
                                )
                              }
                              onFocus={(event) => event.currentTarget.select()}
                              onKeyDown={(event) =>
                                handleVerificationCodeKeyDown(index, event)
                              }
                              onPaste={(event) =>
                                handleVerificationCodePaste(index, event)
                              }
                              pattern="[0-9]*"
                              ref={(element) => {
                                codeInputRefs.current[index] = element;
                              }}
                              type="text"
                              value={digit}
                            />
                            <AnimatePresence initial={false} mode="wait">
                              {digit ? (
                                <motion.span
                                  animate={{
                                    opacity: 1,
                                    transform: "translateY(0px)",
                                  }}
                                  aria-hidden="true"
                                  className="pointer-events-none absolute inset-0 flex items-center justify-center font-mono text-2xl font-semibold text-foreground"
                                  data-testid={`nostr-bind-code-digit-value-${index + 1}`}
                                  exit={{
                                    opacity: 0,
                                    transform: shouldReduceMotion
                                      ? "translateY(0px)"
                                      : "translateY(8px)",
                                  }}
                                  initial={{
                                    opacity: 0,
                                    transform: shouldReduceMotion
                                      ? "translateY(0px)"
                                      : "translateY(8px)",
                                  }}
                                  key={digit}
                                  transition={{
                                    duration: shouldReduceMotion ? 0 : 0.15,
                                    ease: "easeOut",
                                  }}
                                >
                                  {digit}
                                </motion.span>
                              ) : null}
                            </AnimatePresence>
                          </div>
                        ))}
                      </div>
                      <p
                        aria-live="polite"
                        className={cn(
                          "mt-2 min-h-5 text-destructive transition-opacity duration-150 ease-out",
                          hasCodeMismatch ? "opacity-100" : "opacity-0",
                        )}
                        id="nostr-bind-code-error"
                        role={hasCodeMismatch ? "alert" : undefined}
                      >
                        {hasCodeMismatch
                          ? VERIFICATION_CODE_MISMATCH_MESSAGE
                          : "\u00a0"}
                      </p>
                    </fieldset>

                    {isExpired ? (
                      <p className="rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-left text-destructive">
                        {EXPIRED_LINK_MESSAGE}
                      </p>
                    ) : null}

                    {error ? (
                      <p className="rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-left text-destructive">
                        {error}
                      </p>
                    ) : null}
                  </div>

                  <div className="mt-8 flex w-full flex-col gap-3">
                    <Button
                      aria-label={copyButtonLabel}
                      className="h-10 w-full"
                      data-testid="nostr-bind-sign-and-copy"
                      disabled={
                        isSigning ||
                        isExpired ||
                        identity === null ||
                        !isVerificationCodeValid
                      }
                      onClick={handleSign}
                      type="button"
                    >
                      <span aria-live="polite" className="sr-only">
                        {copyButtonLabel}
                      </span>
                      <span
                        aria-hidden="true"
                        className="inline-grid h-5 place-items-center overflow-hidden"
                      >
                        <span
                          className={cn(
                            COPY_BUTTON_LABEL_CLASS,
                            copyButtonLabel === "Continue"
                              ? "translate-y-0 opacity-100"
                              : "-translate-y-0.5 opacity-0",
                          )}
                        >
                          Continue
                        </span>
                        <span
                          className={cn(
                            COPY_BUTTON_LABEL_CLASS,
                            copyButtonLabel === "Signing…"
                              ? "translate-y-0 opacity-100"
                              : "translate-y-0.5 opacity-0",
                          )}
                        >
                          Signing…
                        </span>
                      </span>
                    </Button>
                    <Button
                      className="h-10 w-full text-muted-foreground hover:text-accent-foreground"
                      disabled={isSigning}
                      onClick={() => handleOpenChange(false)}
                      type="button"
                      variant="ghost"
                    >
                      Cancel
                    </Button>
                  </div>
                </OnboardingSlideTransition>
              )}
            </div>
          </DialogPrimitive.Content>
        ) : null}
      </DialogPrimitive.Portal>
    </DialogPrimitive.Root>
  );
}
