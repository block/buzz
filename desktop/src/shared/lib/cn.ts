import { clsx, type ClassValue } from "clsx";
import { extendTailwindMerge } from "tailwind-merge";

const mergeClassNames = extendTailwindMerge({
  extend: {
    classGroups: {
      "font-size": [
        {
          text: ["message", "message-timestamp"],
        },
      ],
    },
  },
});

export function cn(...inputs: ClassValue[]) {
  return mergeClassNames(clsx(inputs));
}
