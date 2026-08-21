/**
 * Center-crop an image file to a small square data URL for avatars and icons.
 * The 128px WebP/PNG output is compact enough for inline profile data while
 * retaining enough detail for every current avatar surface.
 */

const ICON_SIZE = 128;

export async function downscaleSquareImageToDataUrl(
  file: File,
): Promise<string> {
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
