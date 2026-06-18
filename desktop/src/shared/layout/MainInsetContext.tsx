import * as React from "react";

const MainInsetContext =
  React.createContext<React.RefObject<HTMLElement | null> | null>(null);

export function MainInsetProvider({
  children,
  mainInsetRef,
}: {
  children: React.ReactNode;
  mainInsetRef: React.RefObject<HTMLElement | null>;
}) {
  return (
    <MainInsetContext.Provider value={mainInsetRef}>
      {children}
    </MainInsetContext.Provider>
  );
}

/** Ref to the app `<main>` inset element where shared chrome CSS vars live. */
export function useMainInsetRef() {
  const ref = React.useContext(MainInsetContext);
  if (!ref) {
    throw new Error("useMainInsetRef must be used within MainInsetProvider");
  }
  return ref;
}
