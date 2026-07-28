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

test("goBattleRhythm navigates to the battle rhythm route", async () => {
  let appNavigation = null;
  function NavigationProbe() {
    appNavigation = useAppNavigation();
    return null;
  }
  const rootRoute = createRootRoute({ component: NavigationProbe });
  const indexRoute = createRoute({
    getParentRoute: () => rootRoute,
    path: "/",
  });
  const battleRhythmRoute = createRoute({
    getParentRoute: () => rootRoute,
    path: "/battle-rhythm",
  });
  const router = createRouter({
    history: createMemoryHistory({ initialEntries: ["/"] }),
    routeTree: rootRoute.addChildren([indexRoute, battleRhythmRoute]),
  });
  await router.load();
  renderToStaticMarkup(React.createElement(RouterProvider, { router }));
  assert.ok(appNavigation);
  assert.equal(typeof appNavigation.goBattleRhythm, "function");
  await appNavigation.goBattleRhythm();
  assert.equal(router.state.location.pathname, "/battle-rhythm");
});

test("goPlans navigates to the naval planning route", async () => {
  let appNavigation = null;
  function NavigationProbe() {
    appNavigation = useAppNavigation();
    return null;
  }
  const rootRoute = createRootRoute({ component: NavigationProbe });
  const indexRoute = createRoute({
    getParentRoute: () => rootRoute,
    path: "/",
  });
  const plansRoute = createRoute({
    getParentRoute: () => rootRoute,
    path: "/plans",
  });
  const router = createRouter({
    history: createMemoryHistory({ initialEntries: ["/"] }),
    routeTree: rootRoute.addChildren([indexRoute, plansRoute]),
  });
  await router.load();
  renderToStaticMarkup(React.createElement(RouterProvider, { router }));
  assert.ok(appNavigation);
  await appNavigation.goPlans();
  assert.equal(router.state.location.pathname, "/plans");
});
