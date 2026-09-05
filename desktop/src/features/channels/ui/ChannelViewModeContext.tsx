import { LayoutDashboard, MessageSquareText } from "lucide-react";
import * as React from "react";

import type { ChannelViewMode } from "@/features/channels/lib/canvasBoard";
import { Tabs, TabsList, TabsTrigger } from "@/shared/ui/tabs";

type ChannelViewModeContextValue = {
  boardAvailable: boolean;
  mode: ChannelViewMode;
  onModeChange: (mode: ChannelViewMode) => void;
};

const ChannelViewModeContext = React.createContext<
  ChannelViewModeContextValue | undefined
>(undefined);

export function ChannelViewModeProvider({
  children,
  value,
}: {
  children: React.ReactNode;
  value: ChannelViewModeContextValue;
}) {
  return (
    <ChannelViewModeContext.Provider value={value}>
      {children}
    </ChannelViewModeContext.Provider>
  );
}

export function ChannelViewModeToggle() {
  const context = React.useContext(ChannelViewModeContext);

  if (!context?.boardAvailable) {
    return null;
  }

  return (
    <Tabs
      onValueChange={(value) => context.onModeChange(value as ChannelViewMode)}
      value={context.mode}
    >
      <TabsList
        aria-label="Channel view"
        className="h-8 gap-0.5 rounded-lg p-0.5"
        data-testid="channel-view-mode"
      >
        <TabsTrigger
          aria-label="Board view"
          className="h-7 gap-1.5 px-2 text-xs"
          data-testid="channel-view-board"
          value="board"
        >
          <LayoutDashboard className="h-3.5 w-3.5" />
          <span className="hidden sm:inline">Board</span>
        </TabsTrigger>
        <TabsTrigger
          aria-label="Stream view"
          className="h-7 gap-1.5 px-2 text-xs"
          data-testid="channel-view-stream"
          value="stream"
        >
          <MessageSquareText className="h-3.5 w-3.5" />
          <span className="hidden sm:inline">Stream</span>
        </TabsTrigger>
      </TabsList>
    </Tabs>
  );
}

export function useChannelViewMode(): ChannelViewModeContextValue {
  const context = React.useContext(ChannelViewModeContext);
  if (!context) {
    throw new Error(
      "useChannelViewMode must be used inside ChannelViewModeProvider",
    );
  }
  return context;
}
