import * as React from "react";

export type SystemColorScheme = "dark" | "light";

function getSystemColorScheme(): SystemColorScheme {
  if (
    typeof window === "undefined" ||
    typeof window.matchMedia !== "function"
  ) {
    return "light";
  }

  return window.matchMedia("(prefers-color-scheme: dark)").matches
    ? "dark"
    : "light";
}

export function useSystemColorScheme(): SystemColorScheme {
  const [colorScheme, setColorScheme] =
    React.useState<SystemColorScheme>(getSystemColorScheme);

  React.useEffect(() => {
    const mediaQuery = window.matchMedia("(prefers-color-scheme: dark)");
    const updateColorScheme = () => {
      setColorScheme(mediaQuery.matches ? "dark" : "light");
    };

    updateColorScheme();
    mediaQuery.addEventListener("change", updateColorScheme);

    return () => {
      mediaQuery.removeEventListener("change", updateColorScheme);
    };
  }, []);

  return colorScheme;
}
