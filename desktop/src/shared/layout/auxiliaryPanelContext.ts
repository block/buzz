import * as React from "react";

export type AuxiliaryPanelMode = "docked" | "panel" | "single-panel";
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

export function requireAuxiliaryPanelContext(
  context: AuxiliaryPanelContextValue | null,
): AuxiliaryPanelContextValue {
  if (!context) {
    throw new Error("useAuxiliaryPanel must be used within AuxiliaryPanel");
  }

  return context;
}

export function resolveAuxiliaryPanelBodyMode({
  context,
  mode,
}: {
  context: AuxiliaryPanelContextValue | null;
  mode?: AuxiliaryPanelMode;
}): AuxiliaryPanelMode {
  const resolvedMode = mode ?? context?.mode;

  if (resolvedMode == null) {
    throw new Error(
      "AuxiliaryPanelBody requires `mode` or an AuxiliaryPanel ancestor",
    );
  }

  return resolvedMode;
}

/** Read chrome/layout state from the nearest `AuxiliaryPanel` ancestor. */
export function useAuxiliaryPanel(): AuxiliaryPanelContextValue {
  return requireAuxiliaryPanelContext(React.useContext(AuxiliaryPanelContext));
}
