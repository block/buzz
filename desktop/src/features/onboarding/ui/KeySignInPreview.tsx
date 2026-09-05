import * as React from "react";

import { cn } from "@/shared/lib/cn";
import { StyledQrCode } from "@/shared/ui/styled-qr-code";
import { NostrKeyImportForm } from "./NostrKeyImportForm";
import {
  OnboardingPreviewStep,
  useOnboardingPreviewCardLayout,
} from "./OnboardingPreviewShell";
import { OnboardingSlideTransition } from "./OnboardingSlideTransition";

export function KeySignInPreview({
  onBack,
  onContinue,
  total,
}: {
  onBack: () => void;
  onContinue: () => void;
  total: number;
}) {
  const cardLayout = useOnboardingPreviewCardLayout();
  const [keyImportDialog, setKeyImportDialog] = React.useState<
    "backup" | "phone" | null
  >(null);

  return (
    <OnboardingPreviewStep
      onBack={keyImportDialog ? () => setKeyImportDialog(null) : onBack}
      testId="onboarding-preview-sign-in-key"
      total={total}
    >
      <OnboardingSlideTransition
        className={cn(
          "flex w-full flex-col",
          cardLayout
            ? "min-h-0 max-w-[560px] items-stretch justify-start text-left"
            : "min-h-[calc(100dvh-13.25rem)] max-w-[837px] items-center text-center",
        )}
        transitionKey={`preview-sign-in-key-${keyImportDialog ?? "key"}`}
      >
        {keyImportDialog === "backup" ? (
          <div className="w-full" data-testid="backup-recovery-dialog">
            <h1 className="text-title font-normal text-foreground">
              Restore from a backup file
            </h1>
            <p className="mt-2 max-w-[440px] text-base leading-6 text-foreground/80">
              Choose the encrypted backup file you saved from Buzz.
            </p>
            <NostrKeyImportForm
              mode="backup"
              onBack={() => setKeyImportDialog(null)}
              onImport={async () => {
                setKeyImportDialog(null);
                onContinue();
              }}
              previewMode
              showBack={false}
              showPasswordStageBack={false}
              variant="spotlight"
            />
          </div>
        ) : keyImportDialog === "phone" ? (
          <div className="w-full" data-testid="phone-recovery-dialog">
            <h1 className="text-title font-normal text-foreground">
              Scan to sign in
            </h1>
            <p className="mt-2 max-w-[440px] text-base leading-6 text-foreground/80">
              Scan this code with a device where you’re currently signed in to
              Buzz.
            </p>
            <div className="mx-auto mt-6 flex size-[290px] items-center justify-center rounded-xl border border-border/70 bg-white p-6">
              <StyledQrCode
                centerImageSrc="/app-icon@2x.png"
                size={240}
                title="Preview identity recovery QR code"
                value="buzz-pair://preview-identity-recovery"
              />
            </div>
          </div>
        ) : (
          <>
            <h1 className="text-title font-normal text-foreground">
              Enter your private key
            </h1>
            <div
              className={cn(
                "max-w-[440px] text-sm leading-6 text-foreground/80",
                cardLayout ? "mt-2" : "mt-5",
              )}
            >
              <p>
                Paste your private key to sign in to Buzz. You can also use a{" "}
                <button
                  className="rounded-sm font-medium underline decoration-foreground/40 underline-offset-4 transition-colors hover:decoration-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2"
                  data-testid="nostr-import-file-button"
                  onClick={() => setKeyImportDialog("backup")}
                  type="button"
                >
                  backup file
                </button>
                , or{" "}
                <button
                  className="rounded-sm font-medium underline decoration-foreground/40 underline-offset-4 transition-colors hover:decoration-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2"
                  data-testid="nostr-import-phone-link"
                  onClick={() => setKeyImportDialog("phone")}
                  type="button"
                >
                  recover from your phone
                </button>
                .
              </p>
            </div>
            <div
              className={cn(
                "w-full",
                !cardLayout && "buzz-onboarding-key-import-position",
              )}
            >
              <NostrKeyImportForm
                onBack={onBack}
                onImport={async () => onContinue()}
                previewMode
                showBack={false}
                variant="spotlight"
              />
            </div>
          </>
        )}
      </OnboardingSlideTransition>
    </OnboardingPreviewStep>
  );
}
