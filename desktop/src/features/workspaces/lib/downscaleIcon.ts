/**
 * Downscale an image file to a small square data-URL for use as a workspace
 * icon. The result is inlined into the kind:9033 command (and the NIP-11
 * document the relay serves) so it renders
 * across workspaces without cross-relay media fetches; the relay caps icon
 * data-URLs at 96 KB, and 128px WebP/PNG output stays far under that.
 */

const ICON_SIZE = 128;

export async function downscaleIconToDataUrl(file: File): Promise<string> {
  const bitmap = await createImageBitmap(file);
  try {
    const side = Math.min(bitmap.width, bitmap.height);
    const sx = (bitmap.width - side) / 2;
    const sy = (bitmap.height - side) / 2;

    const canvas = document.createElement("canvas");
    canvas.width = ICON_SIZE;
    canvas.height = ICON_SIZE;
    const ctx = canvas.getContext("2d");
    if (!ctx) {
      throw new Error("Could not process that image.");
    }
    ctx.imageSmoothingQuality = "high";
    ctx.drawImage(bitmap, sx, sy, side, side, 0, 0, ICON_SIZE, ICON_SIZE);

    // WebP keeps transparency and compresses well; fall back to PNG when the
    // WebView cannot encode WebP.
    const webp = canvas.toDataURL("image/webp", 0.85);
    return webp.startsWith("data:image/webp")
      ? webp
      : canvas.toDataURL("image/png");
  } finally {
    bitmap.close();
  }
}
