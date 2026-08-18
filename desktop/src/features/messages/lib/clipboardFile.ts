type ClipboardFileItem = {
  getAsFile(): File | null;
  kind: string;
  type: string;
};

type ClipboardFileSource = {
  files?: Iterable<File> | null;
  getData?(format: string): string;
  items?: Iterable<ClipboardFileItem> | null;
  types?: Iterable<string> | null;
};

/** Return a real file payload from a paste event. */
export function firstClipboardFile(
  clipboardData: ClipboardFileSource | null | undefined,
): File | null {
  if (!clipboardData) return null;
  for (const item of clipboardData.items ?? []) {
    if (item.kind !== "file") continue;
    const file = item.getAsFile();
    if (file) return file;
  }
  for (const file of clipboardData.files ?? []) {
    if (file) return file;
  }
  return null;
}

/** Whether the browser advertised an image clipboard MIME type. */
export function hasClipboardImageType(
  clipboardData: ClipboardFileSource | null | undefined,
): boolean {
  if (!clipboardData) return false;
  for (const type of clipboardData.types ?? []) {
    if (type.toLowerCase().startsWith("image/")) return true;
  }
  return false;
}

/**
 * Whether paste should use the native image bridge.
 *
 * WebKitGTK can expose a Wayland screenshot as an empty DataTransfer. Empty
 * browser payloads have no useful default paste behavior, so they may safely
 * fall back to the native clipboard. Non-empty text/HTML stays on the editor
 * paste path.
 */
export function shouldReadNativeClipboardImage(
  clipboardData: ClipboardFileSource | null | undefined,
): boolean {
  if (!clipboardData || firstClipboardFile(clipboardData)) return false;
  for (const item of clipboardData.items ?? []) {
    if (item.kind === "file" && item.type.toLowerCase().startsWith("image/")) {
      return true;
    }
  }
  let hasAdvertisedType = false;
  for (const type of clipboardData.types ?? []) {
    hasAdvertisedType = true;
    if (type.toLowerCase().startsWith("image/")) return true;
  }
  if (hasAdvertisedType) return false;
  const plainText = clipboardData.getData?.("text/plain") ?? "";
  const html = clipboardData.getData?.("text/html") ?? "";
  return plainText.length === 0 && html.length === 0;
}

/** Convert native clipboard failures into concise composer feedback. */
export function clipboardPasteErrorMessage(error: unknown): string {
  const message = error instanceof Error ? error.message : String(error ?? "");
  return message.toLowerCase().includes("clipboard contains no image")
    ? "Clipboard does not contain an image."
    : "Could not paste the clipboard image.";
}
