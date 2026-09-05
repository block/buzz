import { useCallback, useEffect, useState } from "react";

const STORAGE_KEY = "buzz-next-color-scheme";

export type ColorScheme = "light" | "dark";

function read(): ColorScheme {
  const stored = localStorage.getItem(STORAGE_KEY);
  if (stored === "light" || stored === "dark") return stored;
  return window.matchMedia("(prefers-color-scheme: dark)").matches
    ? "dark"
    : "light";
}

/**
 * Light and dark are the same role names with different ramp values, so this
 * only toggles one class on <html>. No component knows which mode is active.
 */
export function useColorScheme() {
  const [scheme, setScheme] = useState<ColorScheme>(read);

  useEffect(() => {
    document.documentElement.classList.toggle("dark", scheme === "dark");
    localStorage.setItem(STORAGE_KEY, scheme);
  }, [scheme]);

  const toggle = useCallback(() => {
    setScheme((current) => (current === "light" ? "dark" : "light"));
  }, []);

  return { scheme, toggle };
}
