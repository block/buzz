import * as React from "react";

import type { AuxiliaryPanelMode } from "@/shared/layout/AuxiliaryPanelHeader";

export type AuxiliaryPanelLayout = "standalone" | "split";

export type AuxiliaryPanelContextValue = {
  isFloatingOverlay: boolean;
  isOverlay: boolean;
  isSinglePanelView: boolean;
  isSplitLayout: boolean;
  layout: AuxiliaryPanelLayout;
  mode: AuxiliaryPanelMode;
  onClose: () => void;
  transparentChrome: boolean;
  widthPx: number;
};

export const AuxiliaryPanelContext =
  React.createContext<AuxiliaryPanelContextValue | null>(null);

/** Read chrome/layout state from the nearest `AuxiliaryPanel` ancestor. */
export function useAuxiliaryPanel(): AuxiliaryPanelContextValue {
  const context = React.useContext(AuxiliaryPanelContext);
  if (!context) {
    throw new Error("useAuxiliaryPanel must be used within AuxiliaryPanel");
  }

  return context;
}
