import type { ReactNode } from "react";
import { AppExtensionDock } from "@/extensions/AppExtensionDock";

/** Stable shell slot for product extensions that share horizontal workspace. */
export function AppExtensionLayout({ children }: { children: ReactNode }) {
  return (
    <div className="relative flex min-h-0 min-w-0 flex-1 overflow-hidden">
      {children}
      <AppExtensionDock />
    </div>
  );
}
