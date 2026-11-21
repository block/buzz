import { isTauri } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";

import { invokeTauri } from "@/shared/api/tauri";

/** Open an HTTP(S) or other external URL in the system browser. */
export async function openExternalUrl(url: string): Promise<void> {
  if (isTauri()) {
    await invokeTauri("open_external_url", { url });
    return;
  }
  await openUrl(url);
}
