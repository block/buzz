import { clsx, type ClassValue } from "clsx";
import { extendTailwindMerge } from "tailwind-merge";

// tailwind-merge cannot infer whether a custom `text-*` token is a font size
// or a color. Register the semantic type ramp so role classes compose exactly
// like Tailwind's stock `text-sm` / `text-base` utilities.
const twMerge = extendTailwindMerge({
  extend: {
    theme: {
      text: [
        "display-page-title",
        "display-section-title",
        "body-medium",
        "label-medium",
        "body-small",
        "label-small",
        "caption",
      ],
    },
  },
});

export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs));
}
