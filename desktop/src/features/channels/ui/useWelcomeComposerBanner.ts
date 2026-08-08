import * as React from "react";

import {
  WELCOME_COMPOSER_BANNER_DISMISS_DURATION_SECONDS,
  WELCOME_COMPOSER_BANNER_HIDE_BUFFER_MS,
  WELCOME_COMPOSER_BANNER_SUCCESS_SETTLE_MS,
  WELCOME_PERSONA_ROTATION_MS,
  type WelcomeComposerBannerState,
} from "@/features/channels/ui/WelcomeComposerBanner";

/**
 * Manages the Welcome-channel composer hint banner's state machine.
 *
 * Tracks which channels have been completed within the session so the banner
 * stays hidden on re-entry. Exposes three transitions:
 * - `completeBanner`: agent-mention path — plays the "Nice work." success
 *   animation before auto-dismissing.
 * - `dismissBanner`: manual X-button path — immediately begins the slide-down
 *   dismiss animation.
 */
export function useWelcomeComposerBanner(
  activeChannelId: string | null,
  isActiveWelcomeChannel: boolean,
): {
  bannerState: WelcomeComposerBannerState;
  completeBanner: () => void;
  dismissBanner: () => void;
} {
  const completedChannelIdsRef = React.useRef(new Set<string>());
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
      activeChannelId &&
      isActiveWelcomeChannel &&
      completedChannelIdsRef.current.has(activeChannelId)
    ) {
      setBannerState("hidden");
      return;
    }
    setBannerState("prompt");
  }, [activeChannelId, clearTimers, isActiveWelcomeChannel]);

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
    if (!activeChannelId || !isActiveWelcomeChannel) {
      return;
    }

    clearTimers();
    completedChannelIdsRef.current.add(activeChannelId);
    setBannerState("complete");
    dismissTimerRef.current = window.setTimeout(() => {
      setBannerState("dismissing");
      dismissTimerRef.current = null;
      scheduleHide();
    }, WELCOME_PERSONA_ROTATION_MS + WELCOME_COMPOSER_BANNER_SUCCESS_SETTLE_MS);
  }, [activeChannelId, clearTimers, isActiveWelcomeChannel, scheduleHide]);

  const dismissBanner = React.useCallback(() => {
    if (!activeChannelId || !isActiveWelcomeChannel) {
      return;
    }

    clearTimers();
    completedChannelIdsRef.current.add(activeChannelId);
    setBannerState("dismissing");
    scheduleHide();
  }, [activeChannelId, clearTimers, isActiveWelcomeChannel, scheduleHide]);

  return { bannerState, completeBanner, dismissBanner };
}
