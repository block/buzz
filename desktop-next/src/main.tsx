import "@fontsource-variable/inter/index.css";
import "@fontsource/jetbrains-mono/index.css";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { RouterProvider, createRouter } from "@tanstack/react-router";

import "./shared/styles/globals.css";
import { routeTree } from "./app/routeTree.gen";

import {
  DensityProvider,
  initializeDensity,
} from "./shared/theme/DensityProvider";

const initialDensity = initializeDensity();
const router = createRouter({ routeTree });

declare module "@tanstack/react-router" {
  interface Register {
    router: typeof router;
  }
}

const root = document.getElementById("root");
if (!root) throw new Error("Missing #root element");

createRoot(root).render(
  <StrictMode>
    <DensityProvider initialDensity={initialDensity}>
      <RouterProvider router={router} />
    </DensityProvider>
  </StrictMode>,
);
