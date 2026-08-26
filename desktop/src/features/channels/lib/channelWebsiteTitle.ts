import { invokeTauri } from "@/shared/api/tauri";

import { MAX_CHANNEL_WEBSITE_TITLE_LEN } from "./channelWebsites";

function clipTitle(title: string | null | undefined): string | null {
  const trimmed = title?.trim() ?? "";
  if (!trimmed) return null;
  return trimmed.slice(0, MAX_CHANNEL_WEBSITE_TITLE_LEN);
}

/** Page title via Tauri link-preview metadata (same as Baza/unfurl). */
export async function fetchChannelWebsitePageTitle(
  url: string,
): Promise<string | null> {
  try {
    const metadata = await invokeTauri<{ title?: string | null } | null>(
      "fetch_link_preview_metadata",
      { href: url },
    );
    return clipTitle(metadata?.title);
  } catch {
    return null;
  }
}
