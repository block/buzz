import { focusManager, QueryClient } from "@tanstack/react-query";

import {
  isDocumentVisible,
  subscribeDocumentVisibility,
} from "@/shared/lib/useDocumentVisible";

export function createBuzzQueryClient() {
  focusManager.setEventListener((setFocused) => {
    setFocused(isDocumentVisible());
    return subscribeDocumentVisibility(setFocused);
  });
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
