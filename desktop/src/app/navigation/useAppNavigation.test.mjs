import assert from "node:assert/strict";
import test from "node:test";

import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import {
  createMemoryHistory,
  createRootRoute,
  createRoute,
  createRouter,
  RouterProvider,
} from "@tanstack/react-router";

import { useAppNavigation } from "./useAppNavigation.ts";

test("goCommandConsole navigates to the console route", async () => {
  let appNavigation = null;

  function NavigationProbe() {
    appNavigation = useAppNavigation();
    return null;
  }

  const rootRoute = createRootRoute({
    component: NavigationProbe,
  });
  const indexRoute = createRoute({
    getParentRoute: () => rootRoute,
    path: "/",
  });
  const consoleRoute = createRoute({
    getParentRoute: () => rootRoute,
    path: "/console",
  });
  const router = createRouter({
    history: createMemoryHistory({ initialEntries: ["/"] }),
    routeTree: rootRoute.addChildren([indexRoute, consoleRoute]),
  });

  await router.load();
  renderToStaticMarkup(React.createElement(RouterProvider, { router }));

  assert.ok(appNavigation, "navigation hook should render inside the router");
  assert.equal(typeof appNavigation.goCommandConsole, "function");

  await appNavigation.goCommandConsole();

  assert.equal(router.state.location.pathname, "/console");
});
