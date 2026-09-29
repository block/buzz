import * as React from "react";
import { AppHuddleBar } from "@/app/AppHuddleBar";
import * as BuzzTheme from "@/app/BuzzThemeSurfaces";
import {
  type HuddlePresentation,
  shouldShowHuddleDockChrome,
} from "@/app/huddlePresentation";
import { HuddleProvider, useHuddle } from "@/features/huddle";
import { HUDDLE_SHORTCUT_EVENT } from "@/shared/lib/keyboard-shortcuts";
import { RemindMeLaterProvider } from "@/features/reminders/ui/RemindMeLaterProvider";
import { cn } from "@/shared/lib/cn";

type AppHuddleShellProps = {
  children: React.ReactNode;
  currentPubkey?: string;
  isCompanionOpen: boolean;
  isDrawerOpen: boolean;
  isRoom: boolean;
  /** Main-app presentation enum; dock chrome uses isRoom XOR main mount gate. */
  presentation: HuddlePresentation;
  onCompanionOpen: () => void | Promise<void>;
  onHuddleStartPendingChange: (pending: boolean) => void;
  onHuddleStarted: (ephemeralChannelId: string) => void | Promise<void>;
  onShowHuddleInMainApp: (ephemeralChannelId: string) => void;
  onViewHuddleChannel: (ephemeralChannelId: string) => void;
  onVisibilityChange: (visible: boolean) => void;
};

type HuddleShortcutHandlerProps = {
  children: React.ReactNode;
};

function HuddleShortcutHandler({ children }: HuddleShortcutHandlerProps) {
  const { activeEphemeralChannelId, leaveHuddle } = useHuddle();

  React.useEffect(() => {
    if (!activeEphemeralChannelId) return;

    function handleHuddleShortcut() {
      void leaveHuddle();
    }

    window.addEventListener(HUDDLE_SHORTCUT_EVENT, handleHuddleShortcut);
    return () =>
      window.removeEventListener(HUDDLE_SHORTCUT_EVENT, handleHuddleShortcut);
  }, [activeEphemeralChannelId, leaveHuddle]);

  return children;
}

export function AppHuddleShell({
  children,
  currentPubkey,
  isCompanionOpen,
  isDrawerOpen,
  isRoom,
  presentation,
  onCompanionOpen,
  onHuddleStartPendingChange,
  onHuddleStarted,
  onShowHuddleInMainApp,
  onViewHuddleChannel,
  onVisibilityChange,
}: AppHuddleShellProps) {
  // Companion (`isRoom` from huddle-* window label) always mounts + reveals the
  // dock. Main never does — do not gate companion chrome on presentation/drawer.
  const showDockChrome = shouldShowHuddleDockChrome({ isRoom, presentation });

  // Companion-only: room webview owns mic/AudioWorklet. Main-owned capture goes
  // deaf when WKWebView suspends the background AudioContext under companion focus.
  return (
    <HuddleProvider
      ownsAudioSession={isRoom}
      onHuddleStartPendingChange={
        isRoom ? undefined : onHuddleStartPendingChange
      }
      onHuddleStarted={isRoom ? undefined : onHuddleStarted}
      onShowHuddleInMainApp={isRoom ? undefined : onShowHuddleInMainApp}
      onViewHuddleChannel={isRoom ? undefined : onViewHuddleChannel}
    >
      <HuddleShortcutHandler>
        <RemindMeLaterProvider pubkey={currentPubkey}>
          <div
            className="buzz-huddle-shell relative h-dvh overflow-hidden overscroll-none"
            data-huddle-open={showDockChrome || isDrawerOpen}
            data-huddle-companion={isCompanionOpen || presentation === "window"}
            data-huddle-window={isRoom}
          >
            <div
              aria-hidden="true"
              className={cn(
                "buzz-huddle-drawer-backdrop",
                showDockChrome && "buzz-huddle-drawer-backdrop-open",
              )}
            />
            <div
              className={cn(
                "buzz-huddle-app-surface z-10 flex min-h-0 flex-row overflow-hidden bg-background",
                showDockChrome &&
                  (isRoom
                    ? "buzz-huddle-app-surface-room-open"
                    : "buzz-huddle-app-surface-open"),
              )}
            >
              <BuzzTheme.GradientLayer />
              {children}
            </div>
            {/* Companion room label → dock. Main presentation never mounts. */}
            {showDockChrome ? (
              <div className="buzz-huddle-drawer-slot absolute inset-x-0 bottom-0 z-[2] min-h-(--buzz-huddle-drawer-height)">
                <AppHuddleBar
                  mode={isRoom ? "room" : "main"}
                  onOpenHuddleWindow={isRoom ? undefined : onCompanionOpen}
                  onVisibilityChange={onVisibilityChange}
                />
              </div>
            ) : null}
          </div>
        </RemindMeLaterProvider>
      </HuddleShortcutHandler>
    </HuddleProvider>
  );
}
