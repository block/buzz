import { createHashHistory, createRouter } from "@tanstack/react-router";

import { routeTree } from "@/app/routeTree.gen";

export const router = createRouter({
  routeTree,
  history: createHashHistory(),
  scrollRestoration: true,
  getScrollRestorationKey: (location: { pathname: string }) =>
    location.pathname,
});

declare module "@tanstack/react-router" {
  interface Register {
    router: typeof router;
  }
}
