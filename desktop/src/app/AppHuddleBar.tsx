import type * as React from "react";

import { HuddleBar } from "@/features/huddle";

import { AppProfilePanelProvider } from "@/app/AppProfilePanelProvider";

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
  return (
    <AppProfilePanelProvider>
      <HuddleBar
        className="h-full"
        mode={mode}
        onOpenHuddleWindow={onOpenHuddleWindow}
        onOpenThread={onOpenThread}
        onVisibilityChange={onVisibilityChange}
      />
    </AppProfilePanelProvider>
  );
}
