import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { RouterProvider, createRouter } from "@tanstack/react-router";

import "@fontsource-variable/inter/wght.css";
import "@fontsource/jetbrains-mono/400.css";
import "./shared/styles/globals.css";
import { routeTree } from "./app/routeTree.gen";
import { useKeyboardFocusVisibility } from "./app/useKeyboardFocusVisibility";
import { applyStoredColorScheme } from "./shared/theme/useColorScheme";

// Before the first render, so a stored dark choice never shows a light frame.
applyStoredColorScheme();

const router = createRouter({ routeTree });

declare module "@tanstack/react-router" {
  interface Register {
    router: typeof router;
  }
}

function AppRoot() {
  useKeyboardFocusVisibility();
  return <RouterProvider router={router} />;
}

const root = document.getElementById("root");
if (!root) throw new Error("Missing #root element");

createRoot(root).render(
  <StrictMode>
    <AppRoot />
  </StrictMode>,
);
