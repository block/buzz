import {
  createContext,
  type ReactNode,
  useContext,
  useEffect,
  useLayoutEffect,
  useState,
} from "react";

/** Layout density is independent of color scheme and readable text size. */
export type Density = "normal" | "compact";
const STORAGE_KEY = "buzz-next-density";
const normalize = (value: string | null): Density =>
  value === "compact" ? "compact" : "normal";

/** Apply the saved density before mounting, including portal geometry. */
export function initializeDensity(): Density {
  let density: Density = "normal";
  try {
    density = normalize(localStorage.getItem(STORAGE_KEY));
  } catch {
    // Storage is optional. A blocked read cannot supply a saved preference.
  }
  document.documentElement.dataset.density = density;
  return density;
}

const DensityContext = createContext<{
  density: Density;
  setDensity: (density: Density) => void;
  saveError: string | null;
} | null>(null);

/** One root preference survives route changes and synchronizes browser tabs. */
export function DensityProvider({
  initialDensity,
  children,
}: {
  initialDensity: Density;
  children: ReactNode;
}) {
  const [density, updateDensity] = useState(initialDensity);
  const [saveError, setSaveError] = useState<string | null>(null);

  useLayoutEffect(() => {
    document.documentElement.dataset.density = density;
  }, [density]);

  useEffect(() => {
    const onStorage = (event: StorageEvent) => {
      if (event.storageArea !== localStorage) return;
      if (event.key !== STORAGE_KEY && event.key !== null) return;
      updateDensity(normalize(event.newValue));
      setSaveError(null);
    };
    window.addEventListener("storage", onStorage);
    return () => window.removeEventListener("storage", onStorage);
  }, []);

  const setDensity = (next: Density) => {
    updateDensity(next);
    try {
      localStorage.setItem(STORAGE_KEY, next);
      setSaveError(null);
    } catch {
      setSaveError(
        "Density changed for this visit. Your browser couldn’t save it. Select it again to retry.",
      );
    }
  };

  return (
    <DensityContext.Provider value={{ density, setDensity, saveError }}>
      {children}
    </DensityContext.Provider>
  );
}

/** Read or change the application's shared density preference. */
export function useDensity() {
  const value = useContext(DensityContext);
  if (!value) throw new Error("useDensity requires DensityProvider");
  return value;
}
