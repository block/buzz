const MAX_CARD_AVATAR_DIMENSION = 1024;

/**
 * Rasterize Buzz's inline SVG/emoji avatar format for the native card command.
 *
 * The profile and card preview intentionally keep the SVG data URL so it stays
 * crisp. The native image pipeline accepts raster bytes, though, so only that
 * boundary receives a PNG derived from the exact same source.
 */
export async function rasterizeInlineSvgAvatarForCard(
  avatarUrl: string | null,
): Promise<string | undefined> {
  const source = avatarUrl?.trim();
  if (!source?.toLowerCase().startsWith("data:image/svg+xml")) {
    return undefined;
  }

  const image = await loadImage(source);
  const sourceWidth = image.naturalWidth || image.width || 512;
  const sourceHeight = image.naturalHeight || image.height || 512;
  const scale = Math.min(
    1,
    MAX_CARD_AVATAR_DIMENSION / Math.max(sourceWidth, sourceHeight),
  );
  const canvas = document.createElement("canvas");
  canvas.width = Math.max(1, Math.round(sourceWidth * scale));
  canvas.height = Math.max(1, Math.round(sourceHeight * scale));
  const context = canvas.getContext("2d");
  if (!context) throw new Error("Canvas is unavailable.");
  context.drawImage(image, 0, 0, canvas.width, canvas.height);

  const pngDataUrl = canvas.toDataURL("image/png");
  if (!pngDataUrl.startsWith("data:image/png;base64,")) {
    throw new Error("Avatar PNG encoding failed.");
  }
  return pngDataUrl;
}

function loadImage(source: string): Promise<HTMLImageElement> {
  return new Promise((resolve, reject) => {
    const image = new Image();
    image.decoding = "async";
    image.onload = () => resolve(image);
    image.onerror = () => reject(new Error("Avatar SVG could not be loaded."));
    image.src = source;
  });
}
