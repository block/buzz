import { writeTextToClipboard } from "@/shared/lib/clipboard";

import { browserShareToJson, suggestBrowserShareFilename } from "./serialize";
import type { HulaBrowserShareV1 } from "./types";

/** Trigger a JSON file download in the renderer. */
export function downloadBrowserShare(share: HulaBrowserShareV1): void {
  const text = browserShareToJson(share);
  const filename = suggestBrowserShareFilename(share);
  const blob = new Blob([text], { type: "application/json" });
  const href = URL.createObjectURL(blob);
  try {
    const anchor = document.createElement("a");
    anchor.href = href;
    anchor.download = filename;
    anchor.rel = "noopener";
    document.body.appendChild(anchor);
    anchor.click();
    anchor.remove();
  } finally {
    URL.revokeObjectURL(href);
  }
}

export async function copyBrowserShareToClipboard(
  share: HulaBrowserShareV1,
): Promise<void> {
  await writeTextToClipboard(browserShareToJson(share));
}
