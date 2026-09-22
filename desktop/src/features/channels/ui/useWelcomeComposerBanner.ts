import * as React from "react";

import {
  WELCOME_COMPOSER_BANNER_DISMISS_DURATION_SECONDS,
  WELCOME_COMPOSER_BANNER_HIDE_BUFFER_MS,
  WELCOME_COMPOSER_BANNER_SUCCESS_SETTLE_MS,
  WELCOME_PERSONA_ROTATION_MS,
  type WelcomeComposerBannerState,
} from "@/features/channels/ui/WelcomeComposerBanner";

const completedWelcomeComposerIdentityPubkeys = new Set<string>();

/**
 * Manages the Welcome-channel composer hint banner's state machine.
 *
 * Remembers completion across the Welcome experience per identity for this app
 * session, so the hint stays hidden while moving between the private and
 * starter Welcome channels without leaking dismissal to another identity.
 * - `completeBanner`: agent-mention path — plays the "Nice work." success
 *   animation before auto-dismissing.
 * - `dismissBanner`: manual X-button path — immediately begins the slide-down
 *   dismiss animation.
 */
export function useWelcomeComposerBanner(
  activeChannelId: string | null,
  isActiveWelcomeChannel: boolean,
  identityPubkey: string | null,
): {
  bannerState: WelcomeComposerBannerState;
  completeBanner: () => void;
  dismissBanner: () => void;
} {
  const dismissTimerRef = React.useRef<number | null>(null);
  const hideTimerRef = React.useRef<number | null>(null);
  const [bannerState, setBannerState] =
    React.useState<WelcomeComposerBannerState>("prompt");

  const clearTimers = React.useCallback(() => {
    if (dismissTimerRef.current !== null) {
      window.clearTimeout(dismissTimerRef.current);
      dismissTimerRef.current = null;
    }
    if (hideTimerRef.current !== null) {
      window.clearTimeout(hideTimerRef.current);
      hideTimerRef.current = null;
    }
  }, []);

  React.useEffect(() => () => clearTimers(), [clearTimers]);

  React.useEffect(() => {
    clearTimers();
    if (
      isActiveWelcomeChannel &&
      identityPubkey &&
      completedWelcomeComposerIdentityPubkeys.has(identityPubkey)
    ) {
      setBannerState("hidden");
      return;
    }
    setBannerState("prompt");
  }, [clearTimers, identityPubkey, isActiveWelcomeChannel]);

  const scheduleHide = React.useCallback(() => {
    hideTimerRef.current = window.setTimeout(
      () => {
        setBannerState("hidden");
        hideTimerRef.current = null;
      },
      WELCOME_COMPOSER_BANNER_DISMISS_DURATION_SECONDS * 1000 +
        WELCOME_COMPOSER_BANNER_HIDE_BUFFER_MS,
    );
  }, []);

  const completeBanner = React.useCallback(() => {
    if (!activeChannelId || !isActiveWelcomeChannel || !identityPubkey) {
      return;
    }

    clearTimers();
    completedWelcomeComposerIdentityPubkeys.add(identityPubkey);
    setBannerState("complete");
    dismissTimerRef.current = window.setTimeout(() => {
      setBannerState("dismissing");
      dismissTimerRef.current = null;
      scheduleHide();
    }, WELCOME_PERSONA_ROTATION_MS + WELCOME_COMPOSER_BANNER_SUCCESS_SETTLE_MS);
  }, [
    activeChannelId,
    clearTimers,
    identityPubkey,
    isActiveWelcomeChannel,
    scheduleHide,
  ]);

  const dismissBanner = React.useCallback(() => {
    if (!activeChannelId || !isActiveWelcomeChannel || !identityPubkey) {
      return;
    }

    clearTimers();
    completedWelcomeComposerIdentityPubkeys.add(identityPubkey);
    setBannerState("dismissing");
    scheduleHide();
  }, [
    activeChannelId,
    clearTimers,
    identityPubkey,
    isActiveWelcomeChannel,
    scheduleHide,
  ]);

  return { bannerState, completeBanner, dismissBanner };
}
