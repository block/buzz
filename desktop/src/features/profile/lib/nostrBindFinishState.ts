export interface NostrBindBrowserFinishState {
  title: string;
  description: string;
  fallbackLabel: string;
  closeLabel: string;
}

export function nostrBindBrowserFinishState(
  callbackError: string | null,
): NostrBindBrowserFinishState {
  if (callbackError) {
    return {
      title: "The browser did not open",
      description:
        "Buzz signed the approval, but could not open Run402. Use the manual recovery below; no ownership changed.",
      fallbackLabel: "Use manual recovery",
      closeLabel: "Close",
    };
  }

  return {
    title: "Check Run402 in your browser",
    description:
      "The Run402 tab showing the six-digit code is the only place that can confirm whether co-ownership completed.",
    fallbackLabel: "Run402 didn’t receive the approval?",
    closeLabel: "Close",
  };
}
