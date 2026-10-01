/**
 * Whether a paste should fall back to reading the image off the native
 * clipboard.
 *
 * WebKitGTK reports an empty `DataTransfer` for an image-only clipboard, which
 * is byte-identical to what a middle-click paste of the PRIMARY selection
 * reports. Arboard always reads CLIPBOARD, so firing on a middle-click would
 * upload an unrelated image; requiring a keyboard paste keeps the fallback tied
 * to an explicit Ctrl/Cmd+V. Anything the webview did expose -- text, HTML, or
 * a file -- belongs to the handlers above this one.
 */
export function shouldReadNativeClipboardImage(
  clipboardData: DataTransfer | null | undefined,
  keyboardInitiated: boolean,
): boolean {
  if (!keyboardInitiated || !clipboardData) return false;
  return (
    (clipboardData.types?.length ?? 0) === 0 &&
    (clipboardData.files?.length ?? 0) === 0 &&
    (clipboardData.items?.length ?? 0) === 0
  );
}

/** True for the Ctrl/Cmd+V that precedes a keyboard-initiated paste event. */
export function isPasteShortcut(event: KeyboardEvent): boolean {
  return (
    Boolean(event.ctrlKey || event.metaKey) && event.key?.toLowerCase() === "v"
  );
}

/** Wrap raw clipboard PNG bytes as the File the upload path expects. */
export function clipboardImageFile(png: ArrayBuffer): File {
  return new File([png], clipboardImageFilename(), { type: "image/png" });
}

/** Name a clipboard image the way a screenshot tool would, for the imeta field. */
export function clipboardImageFilename(now = new Date()): string {
  const stamp = now
    .toISOString()
    .replace(/[:.]/g, "-")
    .replace(/-\d{3}Z$/, "Z");
  return `pasted-image-${stamp}.png`;
}
