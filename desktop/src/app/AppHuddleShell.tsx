import * as React from "react";
import { AppHuddleBar } from "@/app/AppHuddleBar";
import * as BuzzTheme from "@/app/BuzzThemeSurfaces";
import {
  type HuddlePresentation,
  shouldMountMainHuddleDrawerBar,
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
  /** Main-app presentation enum; gate mounts on `presentation === "drawer"`. */
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
  return (
    <HuddleProvider
      ownsAudioSession={!isRoom}
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
            data-huddle-open={isDrawerOpen}
            data-huddle-companion={isCompanionOpen || presentation === "window"}
            data-huddle-window={isRoom}
          >
            <div
              aria-hidden="true"
              className={cn(
                "buzz-huddle-drawer-backdrop",
                isDrawerOpen && "buzz-huddle-drawer-backdrop-open",
              )}
            />
            <div
              className={cn(
                "buzz-huddle-app-surface z-10 flex min-h-0 flex-row overflow-hidden bg-background",
                isDrawerOpen &&
                  (isRoom
                    ? "buzz-huddle-app-surface-room-open"
                    : "buzz-huddle-app-surface-open"),
              )}
            >
              <BuzzTheme.GradientLayer />
              {children}
            </div>
            {/* Strict XOR: main mounts HuddleBar/drawer only when
                presentation === "drawer". Companion room always keeps its bar. */}
            {isRoom || shouldMountMainHuddleDrawerBar(presentation) ? (
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
