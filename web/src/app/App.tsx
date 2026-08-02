import { RouterProvider } from "@tanstack/react-router";

import { router } from "@/app/router";
import { LanguageSwitcher } from "@/shared/i18n/LanguageSwitcher";

export function App() {
  return (
    <>
      <LanguageSwitcher />
      <RouterProvider router={router} />
    </>
  );
}
