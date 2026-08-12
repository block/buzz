import * as React from "react";

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

export function IdentityKeyHelpDialog() {
  const [isVisible, setIsVisible] = React.useState(hasSeenIdentityKeyHelp);

  React.useEffect(() => {
    if (isVisible) return;

    const timeout = window.setTimeout(() => {
      rememberIdentityKeyHelpSeen();
      setIsVisible(true);
    }, IDENTITY_KEY_HELP_DELAY_MS);

    return () => window.clearTimeout(timeout);
  }, [isVisible]);

  return (
    <Dialog>
      <OnboardingFooter className="max-w-none">
        <DialogTrigger asChild>
          <Button
            className={`text-foreground/70 transition-opacity duration-300 hover:text-foreground motion-reduce:transition-none ${
              isVisible ? "opacity-100" : "pointer-events-none opacity-0"
            }`}
            data-testid="identity-key-help-trigger"
            tabIndex={isVisible ? 0 : -1}
            type="button"
            variant="link"
          >
            How do Buzz accounts work?
          </Button>
        </DialogTrigger>
      </OnboardingFooter>
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
            How Buzz accounts work
          </DialogTitle>
          <DialogDescription
            asChild
            className="mt-6 space-y-4 text-pretty text-base leading-7 text-[color:var(--buzz-onboarding-backup-ink)]"
          >
            <div>
              <p>
                A Buzz account is backed by a private identity key created on
                your device. The key signs you in to communities, Desktop,
                agents, and command-line tools as the same person.
              </p>
              <p>
                Your backup password encrypts a recovery file on your computer;
                it is never sent to a Buzz relay. Keep both the file and its
                password safe. Buzz cannot reset that password or recover a lost
                identity key.
              </p>
              <p>
                Never share your private key. Anyone with it can act as your
                account. Administrators manage community access through member
                roles and invitation links, without receiving your key.
              </p>
            </div>
          </DialogDescription>
        </div>
      </DialogContent>
    </Dialog>
  );
}
