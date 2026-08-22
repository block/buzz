import type * as React from "react";

import { CoverDrawer } from "@/features/channels/ui/CoverDrawer";

type AgentActivityDrawerProps = {
  channelName: string;
  children: React.ReactNode;
  onClose: () => void;
};

/**
 * The agent activity presentation at wide viewports: a {@link CoverDrawer}
 * holding the channel-scoped agent session panel.
 *
 * Unlike the thread, activity has no split/focus choice — a transcript of tool
 * calls, diffs, and command output is only legible at this width, so it always
 * covers and never offers a presentation toggle. That also means it needs no
 * conditional focus-restore rule: closing it is always a real dismissal, so
 * focus returns to whatever opened it.
 *
 * Escape stays with the panel rather than being claimed by the drawer. The
 * panel already closes on Escape in this presentation, and routing the key
 * through its `useEscapeKey` keeps the settings menu's own dismissal first —
 * the thread drawer claims the key instead because its composer's mention
 * autocomplete would otherwise swallow a press meant for the thread.
 */
export function AgentActivityDrawer({
  channelName,
  children,
  onClose,
}: AgentActivityDrawerProps) {
  return (
    <CoverDrawer
      ariaLabel="Agent activity"
      onClose={onClose}
      ownsEscape={false}
      scrimLabel={`Back to #${channelName}`}
      testId="agent-activity-drawer"
    >
      {children}
    </CoverDrawer>
  );
}
