import type * as React from "react";
import { AppHuddleBar } from "@/app/AppHuddleBar";
import * as BuzzTheme from "@/app/BuzzThemeSurfaces";
import { HuddleProvider } from "@/features/huddle";
import { RemindMeLaterProvider } from "@/features/reminders/ui/RemindMeLaterProvider";
import { cn } from "@/shared/lib/cn";

type AppHuddleShellProps = {
  children: React.ReactNode;
  currentPubkey?: string;
  isCompanionOpen: boolean;
  isDrawerOpen: boolean;
  isRoom: boolean;
  onCompanionOpen: () => void;
  onHuddleStartPendingChange: (pending: boolean) => void;
  onHuddleStarted: (ephemeralChannelId: string) => void | Promise<void>;
  onShowHuddleInMainApp: (ephemeralChannelId: string) => void;
  onViewHuddleChannel: (ephemeralChannelId: string) => void;
  onVisibilityChange: (visible: boolean) => void;
  /**
   * Terminal substrate layer. Rendered behind the app surface (which carries
   * z-10) so the ⌘J handoff can reveal it by fading the surface above. Not
   * mounted in the dedicated Huddle room window.
   */
  terminal?: React.ReactNode;
};

export function AppHuddleShell({
  children,
  currentPubkey,
  isCompanionOpen,
  isDrawerOpen,
  isRoom,
  onCompanionOpen,
  onHuddleStartPendingChange,
  onHuddleStarted,
  onShowHuddleInMainApp,
  onViewHuddleChannel,
  onVisibilityChange,
  terminal,
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
      <RemindMeLaterProvider pubkey={currentPubkey}>
        <div
          className="buzz-huddle-shell relative h-dvh overflow-hidden overscroll-none"
          data-huddle-open={isDrawerOpen}
          data-huddle-window={isRoom}
        >
          {isRoom ? null : terminal}
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
          {isRoom || !isCompanionOpen ? (
            <div className="absolute inset-x-0 bottom-0 z-0 h-(--buzz-huddle-drawer-height)">
              <AppHuddleBar
                mode={isRoom ? "room" : "main"}
                onOpenHuddleWindow={isRoom ? undefined : onCompanionOpen}
                onVisibilityChange={onVisibilityChange}
              />
            </div>
          ) : null}
        </div>
      </RemindMeLaterProvider>
    </HuddleProvider>
  );
}
