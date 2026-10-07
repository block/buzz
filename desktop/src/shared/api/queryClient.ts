import { focusManager, QueryClient } from "@tanstack/react-query";

import {
  isAppFocused,
  subscribeAppFocus,
} from "@/shared/lib/useDocumentVisible";

let focusManagerConfigured = false;

function configureQueryFocusManager() {
  if (focusManagerConfigured) return;
  focusManagerConfigured = true;

  // Treat app blur as unfocused so query retries pause and stale queries with
  // refetchOnWindowFocus refresh on return. Mutations are unaffected by this
  // focus gate; presence heartbeats also explicitly use retry: 0.
  focusManager.setEventListener((setFocused) => {
    setFocused(isAppFocused());
    return subscribeAppFocus(setFocused);
  });
}

export function createBuzzQueryClient() {
  configureQueryFocusManager();
  return new QueryClient({
    defaultOptions: {
      queries: {
        retry: 1,
        refetchOnWindowFocus: false,
        networkMode: "always",
        gcTime: 5 * 60 * 1_000,
      },
      mutations: {
        networkMode: "always",
      },
    },
  });
}
