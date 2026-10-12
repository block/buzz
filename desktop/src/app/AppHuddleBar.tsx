import type * as React from "react";

import { HuddleBar } from "@/features/huddle";

import { AppProfilePanelProvider } from "@/app/AppProfilePanelProvider";
import { huddlesAvailable } from "@/features/huddle/lib/huddlesAvailable";

type AppHuddleBarProps = Pick<
  React.ComponentProps<typeof HuddleBar>,
  "mode" | "onOpenHuddleWindow" | "onOpenThread" | "onVisibilityChange"
>;

export function AppHuddleBar({
  mode,
  onOpenHuddleWindow,
  onOpenThread,
  onVisibilityChange,
}: AppHuddleBarProps) {
  if (!huddlesAvailable()) return null;
  return (
    <AppProfilePanelProvider>
      <HuddleBar
        className="min-h-(--buzz-huddle-drawer-height)"
        mode={mode}
        onOpenHuddleWindow={onOpenHuddleWindow}
        onOpenThread={onOpenThread}
        onVisibilityChange={onVisibilityChange}
      />
    </AppProfilePanelProvider>
  );
}
