import { QueryClient } from "@tanstack/react-query";

export function createSproutQueryClient() {
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
