import * as React from "react";
import { useFeatureEnabled } from "@/shared/features";
import { hasPrimaryShortcutModifier } from "@/shared/lib/platform";

export function useVoiceDictationShortcut(disabled: boolean) {
  const enabled = useFeatureEnabled("voiceDictation");

  React.useLayoutEffect(() => {
    if (!enabled || disabled) {
      return;
    }

    let keyHeld = false;
    const release = () => {
      if (!keyHeld) return;
      keyHeld = false;
      window.dispatchEvent(new CustomEvent("buzz:dictation-key-up"));
    };
    const handleKeyDown = (event: KeyboardEvent) => {
      if (
        event.defaultPrevented ||
        event.repeat ||
        event.altKey ||
        event.shiftKey ||
        event.key.toLowerCase() !== "d" ||
        !hasPrimaryShortcutModifier(event)
      ) {
        return;
      }
      event.preventDefault();
      keyHeld = true;
      window.dispatchEvent(new CustomEvent("buzz:dictation-key-down"));
    };
    const handleKeyUp = (event: KeyboardEvent) => {
      if (event.key.toLowerCase() === "d") release();
    };
    const handleVisibilityChange = () => {
      if (document.hidden) release();
    };

    window.addEventListener("keydown", handleKeyDown);
    window.addEventListener("keyup", handleKeyUp);
    window.addEventListener("blur", release);
    document.addEventListener("visibilitychange", handleVisibilityChange);
    return () => {
      release();
      window.removeEventListener("keydown", handleKeyDown);
      window.removeEventListener("keyup", handleKeyUp);
      window.removeEventListener("blur", release);
      document.removeEventListener("visibilitychange", handleVisibilityChange);
    };
  }, [disabled, enabled]);
}
