import { fetchAvatarBlob } from "@/features/profile/lib/selfProfileStorage";
import { getAvatarSnapshotUrl } from "@/shared/lib/animatedAvatar";
import {
  DEFAULT_AVATAR_PIXEL_SIZE,
  downscaleAvatarDataUrl,
} from "@/shared/lib/avatarDownscale";
import { rewriteRelayUrl } from "@/shared/lib/mediaUrl";
// Type-only: this module stays free of the Tauri IPC surface so its rules can
// be unit-tested under plain Node.
import type { ProjectCanvasAvatarUpload } from "./projectCanvasCommands";

/**
 * Avatar delivery for sandboxed Project Canvas frames.
 *
 * A canvas frame cannot reach the network, so the host fetches every avatar
 * itself. There are two ways to get one to a widget:
 *
 * 1. **Publish the bytes** and let the frame load
 *    `__buzz/avatar/<pubkey>`. The picture never touches an RPC message, so
 *    no per-response ceiling applies and a channel can show every member's
 *    real face. This is the path {@link toCanvasAvatarUploads} feeds.
 * 2. **Inline a `data:` URL** in the RPC payload. Still supported, because
 *    widgets written before the route existed read `avatarDataUrl` directly —
 *    but it is charged against `PROJECT_CANVAS_MAX_PORT_MESSAGE_BYTES`
 *    (64 KiB) for the whole response, so the ceilings below apply. Overrun it
 *    and the host replies `too-large` and the widget loses the entire result,
 *    display names included.
 *
 * The host does both: it publishes, and it inlines as many as the budget
 * allows. A widget on either path renders a real avatar.
 */

/**
 * Square size, in pixels, avatars are re-encoded to. Canvas avatars render
 * between 20px and 42px, so this stays crisp on a 2x display.
 */
export const CANVAS_AVATAR_PIXEL_SIZE = DEFAULT_AVATAR_PIXEL_SIZE;

/** Per-avatar ceiling. A 96px WebP/JPEG avatar lands far under this. */
export const CANVAS_AVATAR_MAX_DATA_URL_LENGTH = 16 * 1_024;

/**
 * Combined ceiling across one response. Leaves ~24 KiB of the 64 KiB message
 * budget for names, pubkeys, and the envelope — ample for the 32-person
 * maximum a lookup can return.
 */
export const CANVAS_AVATAR_TOTAL_DATA_URL_BUDGET = 40 * 1_024;

/**
 * Fetches `avatarUrl` and re-encodes it small enough to embed in a canvas RPC
 * payload. Returns null when there is no avatar, the fetch fails, or the image
 * cannot be brought under {@link CANVAS_AVATAR_MAX_DATA_URL_LENGTH} — all of
 * which mean "render initials".
 *
 * Animated avatars collapse to their poster frame, and relay-hosted URLs go
 * through the media proxy, matching how the rest of the app resolves an avatar.
 */
export async function fetchCanvasAvatarDataUrl(
  avatarUrl: string | null,
): Promise<string | null> {
  const snapshotUrl = getAvatarSnapshotUrl(avatarUrl);
  if (!snapshotUrl) return null;
  const blob = await fetchAvatarBlob(rewriteRelayUrl(snapshotUrl));
  if (!blob) return null;
  return await downscaleAvatarDataUrl(blob, {
    maxDataUrlLength: CANVAS_AVATAR_MAX_DATA_URL_LENGTH,
    pixelSize: CANVAS_AVATAR_PIXEL_SIZE,
  });
}

/**
 * Splits a base64 data URL into the media type and payload the publish command
 * takes. Returns null for anything that is not one, so a URL-encoded or
 * otherwise unexpected data URL is skipped rather than sent as garbage.
 */
export function splitAvatarDataUrl(
  dataUrl: string,
): { contentType: string; data: string } | null {
  const separator = dataUrl.indexOf(";base64,");
  if (!dataUrl.startsWith("data:image/") || separator < 0) return null;
  const contentType = dataUrl.slice("data:".length, separator);
  const data = dataUrl.slice(separator + ";base64,".length);
  if (!contentType || !data) return null;
  return { contentType, data };
}

/**
 * Turns fetched avatars into the batch the publish command takes.
 *
 * Entries with no picture, or whose data URL cannot be split, are skipped
 * rather than failing the batch — the backend rejects a malformed batch
 * wholesale, and one odd avatar must not cost the rest their pictures.
 */
export function toCanvasAvatarUploads(
  avatars: ReadonlyArray<{ dataUrl: string | null; pubkey: string }>,
): ProjectCanvasAvatarUpload[] {
  const uploads: ProjectCanvasAvatarUpload[] = [];
  for (const { dataUrl, pubkey } of avatars) {
    const parts = dataUrl ? splitAvatarDataUrl(dataUrl) : null;
    if (parts) uploads.push({ ...parts, pubkey });
  }
  return uploads;
}

/**
 * Drops avatars that would push a response past `totalBudget`, preserving
 * order: earlier entries keep their image and later ones degrade to initials.
 *
 * Returns a same-length array so callers can zip it back onto their rows. A
 * single avatar larger than the whole budget is dropped rather than allowed
 * through, so the ceiling always holds.
 */
export function selectAvatarsWithinBudget(
  dataUrls: ReadonlyArray<string | null>,
  totalBudget: number = CANVAS_AVATAR_TOTAL_DATA_URL_BUDGET,
): Array<string | null> {
  let spent = 0;
  return dataUrls.map((dataUrl) => {
    if (!dataUrl) return null;
    if (spent + dataUrl.length > totalBudget) return null;
    spent += dataUrl.length;
    return dataUrl;
  });
}
