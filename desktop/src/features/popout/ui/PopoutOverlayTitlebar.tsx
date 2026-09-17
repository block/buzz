import {
  PLAYGROUND_OPAQUE_FILL_STYLE,
  playgroundFullscreenTitlebarGapClass,
} from "@/features/playground/lib/overlayLayout";
import { ConversationPlaygroundPinsMenu } from "@/features/playground/ui/ConversationPlaygroundPinsMenu";
import {
  isPopoutThreadOnlyLayout,
  type PopoutPayload,
} from "@/features/popout/lib/popoutWindow";
import { cn } from "@/shared/lib/cn";
import { isMacPlatform } from "@/shared/lib/platform";

export const POPOUT_OVERLAY_TITLEBAR_TEST_ID = "popout-overlay-titlebar";
export const POPOUT_TITLEBAR_GAP_TEST_ID = "popout-titlebar-gap";

/**
 * Whether the OS pop-out overlay strip should host the thread-scoped playground
 * pin control. Channel-only / playground-only companions keep an empty drag
 * gap; thread/split pop-outs hide MessageThreadPanelHeader, so pins live here.
 *
 * No app-level close control: macOS traffic lights already close the window,
 * and a trailing X would read as a duplicate window chrome.
 */
export function popoutOverlayTitlebarShowsPins(
  payload: PopoutPayload | null,
): boolean {
  if (!isPopoutThreadOnlyLayout(payload)) return false;
  const channelId = payload?.channelId?.trim() ?? "";
  const threadId = payload?.threadId?.trim() ?? "";
  return channelId.length > 0 && threadId.length > 0;
}

/**
 * Slim overlay titlebar for OS companion windows. Keeps the ~40px drag gap so
 * traffic lights stay clickable; when thread-only, pins sit on the trailing
 * edge with macOS left inset clearance (same 80px as AppTopChrome).
 */
export function PopoutOverlayTitlebar({
  payload,
}: {
  payload: PopoutPayload | null;
}) {
  const showPins = popoutOverlayTitlebarShowsPins(payload);
  const macChrome = isMacPlatform();
  const channelId = payload?.channelId?.trim() ?? "";
  const threadId = payload?.threadId?.trim() ?? "";

  return (
    <div
      aria-hidden={showPins ? undefined : true}
      className={cn(
        "relative z-20 flex shrink-0 items-center",
        playgroundFullscreenTitlebarGapClass,
      )}
      data-tauri-drag-region
      data-testid={POPOUT_TITLEBAR_GAP_TEST_ID}
      style={PLAYGROUND_OPAQUE_FILL_STYLE}
    >
      {showPins ? (
        <div
          className={cn(
            "flex h-full w-full items-center justify-end",
            // Fixed px: native traffic lights ignore Cmd+/- rem zoom (AppTopChrome).
            macChrome ? "translate-y-[3px] pl-[80px] pr-2" : "px-2",
          )}
          data-testid={POPOUT_OVERLAY_TITLEBAR_TEST_ID}
        >
          <ConversationPlaygroundPinsMenu
            channelId={channelId}
            threadId={threadId}
          />
        </div>
      ) : null}
    </div>
  );
}
