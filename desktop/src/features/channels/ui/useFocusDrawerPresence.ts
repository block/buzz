import * as React from "react";

/** Keeps the covered channel inert until the focus drawer finishes exiting. */
export function useFocusDrawerPresence(open: boolean) {
  const [present, setPresent] = React.useState(false);

  React.useEffect(() => {
    if (open) setPresent(true);
  }, [open]);

  const markExitComplete = React.useCallback(() => setPresent(false), []);
  return {
    channelIsCovered: open || present,
    markExitComplete,
  };
}
