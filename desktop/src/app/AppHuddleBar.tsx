import type * as React from "react";

import { HuddleBar } from "@/features/huddle";

import { AppProfilePanelProvider } from "@/app/AppProfilePanelProvider";

type AppHuddleBarProps = Pick<
  React.ComponentProps<typeof HuddleBar>,
  "onOpenThread" | "onVisibilityChange"
>;

export function AppHuddleBar({
  onOpenThread,
  onVisibilityChange,
}: AppHuddleBarProps) {
  return (
    <AppProfilePanelProvider>
      <HuddleBar
        className="h-full"
        onOpenThread={onOpenThread}
        onVisibilityChange={onVisibilityChange}
      />
    </AppProfilePanelProvider>
  );
}
