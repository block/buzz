import * as React from "react";

export type TerminalContextOverride = {
  channelId: string;
  channelName: string;
};

const TerminalContextOverrideContext = React.createContext<React.Dispatch<
  React.SetStateAction<TerminalContextOverride | null>
> | null>(null);

export function TerminalContextOverrideProvider({
  children,
  onChange,
}: {
  children: React.ReactNode;
  onChange: React.Dispatch<
    React.SetStateAction<TerminalContextOverride | null>
  >;
}) {
  return (
    <TerminalContextOverrideContext.Provider value={onChange}>
      {children}
    </TerminalContextOverrideContext.Provider>
  );
}

export function useTerminalContextOverride(
  context: TerminalContextOverride | null,
) {
  const setOverride = React.useContext(TerminalContextOverrideContext);

  React.useEffect(() => {
    if (!setOverride) return;
    setOverride(context);
    return () => setOverride(null);
  }, [context, setOverride]);
}
