import { useEffect } from "react";

import { useFontScale } from "@/shared/lib/fontScalePreference";

/**
 * Applies the persisted font-scale preference to the document root as a CSS
 * `font-size` multiplier. Mounted once near the app root; every rem-based
 * style cascades from this value.
 *
 * The base font-size is 16px (the browser default), so a scale of 1.2 sets
 * `font-size: 19.2px` on `<html>`, and every `rem` unit grows proportionally.
 */
export function FontScaleApplier() {
  const fontScale = useFontScale();

  useEffect(() => {
    document.documentElement.style.fontSize = `${fontScale * 16}px`;
  }, [fontScale]);

  return null;
}
