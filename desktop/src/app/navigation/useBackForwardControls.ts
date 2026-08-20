import * as React from "react";
import {
  useCanGoBack,
  useRouter,
  useRouterState,
} from "@tanstack/react-router";
import { isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

import { matchBackForwardChord } from "@/app/navigation/backForwardChords";
import { isMacPlatform } from "@/shared/lib/platform";
import { trimMapToSize } from "@/shared/lib/trimMapToSize";

type RouterHistoryState = {
  __TSR_index?: number;
  __TSR_key?: string;
  key?: string;
};

export function useBackForwardControls() {
  const router = useRouter();
  const canGoBack = useCanGoBack();
  const locationState = useRouterState({
    select: (state) => state.location.state,
  }) as RouterHistoryState;
  const locationIndex = locationState.__TSR_index ?? 0;
  const locationKey =
    locationState.__TSR_key ?? locationState.key ?? String(locationIndex);
  const keysByIndexRef = React.useRef(new Map<number, string>());
  const [maxIndex, setMaxIndex] = React.useState(locationIndex);

  React.useEffect(() => {
    const keysByIndex = keysByIndexRef.current;
    const currentKey = keysByIndex.get(locationIndex);

    if (currentKey && currentKey !== locationKey) {
      for (const storedIndex of [...keysByIndex.keys()]) {
        if (storedIndex >= locationIndex) {
          keysByIndex.delete(storedIndex);
        }
      }
    }

    keysByIndex.set(locationIndex, locationKey);
    trimMapToSize(keysByIndex, 200);
    setMaxIndex((current: number) => {
      if (currentKey && currentKey !== locationKey) {
        return locationIndex;
      }

      return Math.max(current, locationIndex);
    });
  }, [locationIndex, locationKey]);

  const canGoForward = locationIndex < maxIndex;

  const goBack = React.useCallback(() => {
    if (!canGoBack) {
      return;
    }

    router.history.back();
  }, [canGoBack, router.history]);

  const goForward = React.useCallback(() => {
    if (!canGoForward) {
      return;
    }

    router.history.forward();
  }, [canGoForward, router.history]);

  const handleKeyDown = React.useEffectEvent((event: KeyboardEvent) => {
    // Note: the chords deliberately fire even when focus is inside an
    // editable element. The composer autofocuses on every channel switch
    // (`useComposerAutofocus`), so in steady state focus almost always
    // lives in a contenteditable — an editable-target guard here made the
    // shortcuts effectively dead (#3775). Safe because neither ⌘[ / ⌘]
    // (macOS) nor Alt+←/→ (Windows/Linux) carry text-editing semantics,
    // and the TipTap editor binds no conflicting shortcuts.
    const direction = matchBackForwardChord(event, isMacPlatform());

    if (direction === "back") {
      event.preventDefault();
      goBack();
      return;
    }

    if (direction === "forward") {
      event.preventDefault();
      goForward();
    }
  });

  const handleMouseNav = React.useEffectEvent((direction: string) => {
    if (direction === "back") {
      goBack();
      return;
    }

    if (direction === "forward") {
      goForward();
    }
  });

  React.useEffect(() => {
    window.addEventListener("keydown", handleKeyDown);
    return () => {
      window.removeEventListener("keydown", handleKeyDown);
    };
  }, []);

  // macOS: WKWebView never delivers X1/X2 button events or horizontal
  // swipe gestures to the DOM, so the native layer catches them
  // (`mouse_nav.rs`) and forwards them as a Tauri event.
  React.useEffect(() => {
    if (!isTauri()) {
      return;
    }

    const unlistenPromise = listen<string>("mouse-nav", (event) => {
      handleMouseNav(event.payload);
    });

    return () => {
      void unlistenPromise.then((unlisten) => unlisten());
    };
  }, []);

  return {
    canGoBack,
    canGoForward,
    goBack,
    goForward,
  };
}
