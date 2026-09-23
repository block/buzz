import * as React from "react";

import {
  currentPopoutPayload,
  isPopoutSplitLayout,
  isPopoutThreadOnlyLayout,
  type PopoutPayload,
} from "./popoutWindow";

const PopoutLayoutContext = React.createContext<PopoutPayload | null>(null);

export function PopoutLayoutProvider({
  children,
  payload,
}: {
  children: React.ReactNode;
  payload: PopoutPayload | null;
}) {
  return (
    <PopoutLayoutContext.Provider value={payload}>
      {children}
    </PopoutLayoutContext.Provider>
  );
}

export function usePopoutLayoutPayload(): PopoutPayload | null {
  const embedded = React.useContext(PopoutLayoutContext);
  return currentPopoutPayload() ?? embedded;
}

export function usePopoutThreadOnlyLayout(): boolean {
  return isPopoutThreadOnlyLayout(usePopoutLayoutPayload());
}

export function usePopoutSplitLayout(): boolean {
  return isPopoutSplitLayout(usePopoutLayoutPayload());
}
