import * as React from "react";

import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogTitle,
  DialogTrigger,
} from "@/shared/ui/dialog";
import { ONBOARDING_INK_ICON_CLASS } from "./OnboardingChrome";
import { OnboardingFooter } from "./OnboardingFooter";

const IDENTITY_KEY_HELP_SEEN_STORAGE_KEY =
  "buzz.machine-onboarding.identity-key-help-seen.v1";
const IDENTITY_KEY_HELP_DELAY_MS = 2_000;

function hasSeenIdentityKeyHelp(): boolean {
  try {
    return (
      window.localStorage.getItem(IDENTITY_KEY_HELP_SEEN_STORAGE_KEY) === "true"
    );
  } catch {
    return false;
  }
}

function rememberIdentityKeyHelpSeen() {
  try {
    window.localStorage.setItem(IDENTITY_KEY_HELP_SEEN_STORAGE_KEY, "true");
  } catch {
    // The help remains available for this visit if storage is unavailable.
  }
}

export function IdentityKeyHelpDialog({
  inline = false,
  onPreviewOpen,
  previewMode = false,
  v3Presentation = previewMode,
}: {
  inline?: boolean;
  onPreviewOpen?: () => void;
  previewMode?: boolean;
  /** Apply the experimental V3 label and link treatment in preview mode. */
  v3Presentation?: boolean;
}) {
  const [isVisible, setIsVisible] = React.useState(
    previewMode ? true : hasSeenIdentityKeyHelp,
  );

  React.useEffect(() => {
    if (previewMode || isVisible) return;

    const timeout = window.setTimeout(() => {
      rememberIdentityKeyHelpSeen();
      setIsVisible(true);
    }, IDENTITY_KEY_HELP_DELAY_MS);

    return () => window.clearTimeout(timeout);
  }, [isVisible, previewMode]);

  const triggerButton = (
    <Button
      className={cn(
        v3Presentation
          ? "text-foreground underline decoration-foreground/45 underline-offset-4 hover:decoration-foreground"
          : "text-foreground/70 hover:text-foreground",
        "transition-opacity duration-300 motion-reduce:transition-none",
        inline && "h-auto justify-start p-0 text-left",
        isVisible ? "opacity-100" : "pointer-events-none opacity-0",
      )}
      data-testid="identity-key-help-trigger"
      onClick={previewMode ? onPreviewOpen : undefined}
      tabIndex={isVisible ? 0 : -1}
      type="button"
      variant="link"
    >
      {v3Presentation
        ? "Learn how identity keys work"
        : "What’s an identity key?"}
    </Button>
  );

  if (previewMode && onPreviewOpen) {
    return inline ? (
      triggerButton
    ) : (
      <OnboardingFooter className="max-w-none">
        {triggerButton}
      </OnboardingFooter>
    );
  }

  const trigger = <DialogTrigger asChild>{triggerButton}</DialogTrigger>;

  return (
    <Dialog>
      {inline ? (
        trigger
      ) : (
        <OnboardingFooter className="max-w-none">{trigger}</OnboardingFooter>
      )}
      <DialogContent
        className="buzz-onboarding-neutral-theme max-w-[47.5rem] -translate-y-5"
        closeButtonClassName={ONBOARDING_INK_ICON_CLASS}
        data-system-color-scheme="light"
        data-testid="identity-key-help-dialog"
        overlayVariant="transparent"
        surface="textured"
      >
        <div className="mx-auto w-full max-w-[35rem] py-14 text-left max-sm:py-6">
          <DialogTitle className="text-balance pr-8 text-3xl font-normal text-foreground">
            What’s an identity key?
          </DialogTitle>
          <DialogDescription
            asChild
            className="mt-6 space-y-4 text-pretty text-base leading-7 text-[color:var(--buzz-onboarding-backup-ink)]"
          >
            <div>
              <IdentityKeyHelpBody />
            </div>
          </DialogDescription>
        </div>
      </DialogContent>
    </Dialog>
  );
}

function IdentityKeyHelpBody() {
  return (
    <>
      <p>
        Buzz will create a Nostr identity with two parts: a private key that
        signs you in and a public key you can safely share. You can find your
        public identity anytime in Buzz settings.
      </p>
      <p>
        This identity belongs to you, not Buzz, and can move with you to another
        device or compatible Nostr app. Because only you control the private
        key, Buzz can’t reset or recover it. Keep a backup somewhere safe, and
        never share it.
      </p>
    </>
  );
}

/** Production identity-key explainer content in the V3 onboarding card treatment. */
export function IdentityKeyHelpPreviewContent() {
  return (
    <>
      <h1 className="text-title font-normal text-foreground">
        What’s an identity key?
      </h1>
      <div className="mt-2 max-w-[500px] space-y-4 text-pretty text-base leading-7 text-foreground/80">
        <IdentityKeyHelpBody />
      </div>
    </>
  );
}
