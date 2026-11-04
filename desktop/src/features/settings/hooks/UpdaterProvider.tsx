// Buzz source localisation: authored UI strings only; user values stay unchanged.
import { useUiLanguage } from "@/shared/i18n/uiTranslation";
import { createContext, useContext, type ReactNode } from "react";
import { useUpdater } from "./use-updater";

type UpdaterContextValue = ReturnType<typeof useUpdater>;

const UpdaterContext = createContext<UpdaterContextValue | null>(null);

export function UpdaterProvider({ children }: { children: ReactNode }) {
  useUiLanguage();

  const updater = useUpdater();
  return <UpdaterContext value={updater}>{children}</UpdaterContext>;
}

export function useUpdaterContext(): UpdaterContextValue {
  const ctx = useContext(UpdaterContext);
  if (!ctx) {
    throw new Error("useUpdaterContext must be used within an UpdaterProvider");
  }
  return ctx;
}
