import { RouterProvider } from "@tanstack/react-router";

import { router } from "@/app/router";

export function App() {
  return <RouterProvider router={router} />;
}
