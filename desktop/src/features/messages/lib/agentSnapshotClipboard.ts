import type { BlobDescriptor } from "@/shared/api/tauri";
import type { ImetaMedia } from "./imetaMediaMarkdown";

// Keep the original attribute name for clipboard compatibility. The payload
// now accepts both agent and team snapshot filenames.
const CLIPBOARD_ATTRIBUTE = "data-buzz-agent-snapshot";
const MAX_SNAPSHOT_BYTES = 10 * 1024 * 1024;
const MAX_DISPLAY_NAME_LENGTH = 200;
const MAX_FILENAME_LENGTH = 255;

type SnapshotClipboardPayload = {
  version: 1;
  displayName: string;
  filename: string;
  sha256: string;
  size: number;
  type: string;
  url: string;
};

function escapeHtml(value: string): string {
  return value
    .replaceAll("&", "&amp;")
    .replaceAll('"', "&quot;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;");
}

function isSafeSnapshotUrl(value: string): boolean {
  // The raw value is later emitted inside Markdown's `(url)` syntax. Keep
  // delimiters and whitespace out even when URL parsing would normalize them.
  if (/[\s()]/u.test(value)) return false;

  try {
    const url = new URL(value);
    return url.protocol === "https:" || url.protocol === "http:";
  } catch {
    return false;
  }
}

/** Build Buzz-specific clipboard HTML with the raw URL as its visible link. */
export function buildSnapshotClipboardHtml({
  attachment,
  displayName,
  snapshotKind,
}: {
  attachment: ImetaMedia;
  displayName: string;
  snapshotKind: "agent" | "team";
}): string {
  const payload: SnapshotClipboardPayload = {
    version: 1,
    displayName,
    filename: attachment.filename ?? `shared.${snapshotKind}.png`,
    sha256: attachment.sha256,
    size: attachment.size,
    type: attachment.type,
    url: attachment.url,
  };
  const encodedPayload = encodeURIComponent(JSON.stringify(payload));

  return `<a ${CLIPBOARD_ATTRIBUTE}="${encodedPayload}" href="${escapeHtml(attachment.url)}">${escapeHtml(displayName)}</a>`;
}

export function buildAgentSnapshotClipboardHtml(
  input: Omit<Parameters<typeof buildSnapshotClipboardHtml>[0], "snapshotKind">,
): string {
  return buildSnapshotClipboardHtml({ ...input, snapshotKind: "agent" });
}

export function buildTeamSnapshotClipboardHtml(
  input: Omit<Parameters<typeof buildSnapshotClipboardHtml>[0], "snapshotKind">,
): string {
  return buildSnapshotClipboardHtml({ ...input, snapshotKind: "team" });
}

/**
 * Restore a copied agent or team snapshot as composer attachment state.
 *
 * Clipboard HTML is untrusted input, so every field is checked before it can
 * become an outbound imeta tag. Invalid or non-snapshot payloads fall through to
 * the composer's normal paste behavior.
 */
export function parseSnapshotClipboardHtml(html: string): ImetaMedia | null {
  const match = html.match(
    new RegExp(`\\b${CLIPBOARD_ATTRIBUTE}=(?:"([^"]+)"|'([^']+)')`, "i"),
  );
  const encodedPayload = match?.[1] ?? match?.[2];
  if (!encodedPayload) return null;

  let payload: unknown;
  try {
    payload = JSON.parse(decodeURIComponent(encodedPayload));
  } catch {
    return null;
  }

  if (!payload || typeof payload !== "object") return null;
  const candidate = payload as Partial<SnapshotClipboardPayload>;
  const displayName = candidate.displayName?.trim();
  const filename = candidate.filename?.trim();
  const sha256 = candidate.sha256?.trim().toLowerCase();

  if (
    candidate.version !== 1 ||
    !displayName ||
    displayName.length > MAX_DISPLAY_NAME_LENGTH ||
    !filename ||
    filename.length > MAX_FILENAME_LENGTH ||
    filename.includes("/") ||
    filename.includes("\\") ||
    !/\.(?:agent|team)\.png$/i.test(filename) ||
    !sha256 ||
    !/^[0-9a-f]{64}$/.test(sha256) ||
    typeof candidate.size !== "number" ||
    !Number.isInteger(candidate.size) ||
    candidate.size <= 0 ||
    candidate.size > MAX_SNAPSHOT_BYTES ||
    candidate.type !== "image/png" ||
    typeof candidate.url !== "string" ||
    !isSafeSnapshotUrl(candidate.url)
  ) {
    return null;
  }

  return {
    displayLabel: displayName,
    filename,
    sha256,
    size: candidate.size,
    type: "image/png",
    uploaded: 0,
    url: candidate.url,
  };
}

export const parseAgentSnapshotClipboardHtml = parseSnapshotClipboardHtml;

/** Convert a Buzz snapshot clipboard payload into one pending attachment. */
export function handleSnapshotPaste(
  event: Pick<ClipboardEvent, "clipboardData" | "preventDefault">,
  setPendingImeta: (
    update: (current: BlobDescriptor[]) => BlobDescriptor[],
  ) => void,
): boolean {
  const html = event.clipboardData?.getData("text/html") ?? "";
  const pastedSnapshot = parseSnapshotClipboardHtml(html);
  if (!pastedSnapshot) return false;

  event.preventDefault();
  setPendingImeta((current) =>
    current.some(
      (attachment) =>
        attachment.url === pastedSnapshot.url &&
        attachment.sha256 === pastedSnapshot.sha256,
    )
      ? current
      : [...current, pastedSnapshot],
  );
  return true;
}

export const handleAgentSnapshotPaste = handleSnapshotPaste;
