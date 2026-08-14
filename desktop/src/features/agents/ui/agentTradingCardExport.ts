import type {
  SnapshotFormat,
  SnapshotMemoryLevel,
} from "@/shared/api/tauriPersonas";
import { getInitials } from "@/shared/lib/initials";
import {
  fallbackAgentCardColor,
  sampleRepresentativeAvatarColor,
} from "./agentCardPreviewColor";
import { resolveSnapshotAvatarPng } from "./snapshotAvatarPng";
import {
  AGENT_CARD_HEIGHT,
  AGENT_CARD_MEMORY_LABELS,
  AGENT_CARD_WIDTH,
  agentCardFormatLabel,
  agentCardTitleSize,
  stableAgentCardNumber,
  truncateAgentCardTitle,
} from "./agentTradingCardContent";

const AVATAR_X = 175;
const AVATAR_Y = 308;
const AVATAR_SIZE = 864;
const AVATAR_CENTER_X = 607;
const AVATAR_CENTER_Y = 740;
const AVATAR_RADIUS = 432;
const SAMPLE_SIZE = 24;

type RenderAgentTradingCardInput = {
  accentColor?: string;
  agentAvatarUrl: string | null;
  avatarPngDataUrl?: string | null;
  agentId: string;
  agentName: string;
  format: SnapshotFormat;
  memoryLevel: SnapshotMemoryLevel;
  templateUrl: string;
};

/**
 * Flatten the complete default card into the PNG body used by `.agent.png`.
 *
 * The interactive preview is layered HTML/SVG/CSS, which cannot be passed to
 * the native snapshot encoder directly. This renderer deliberately composes
 * the same template, avatar mask, sampled palette, and variable copy onto an
 * opaque full-resolution canvas. The backend then injects the import manifest
 * into this image without changing its visible pixels.
 */
export async function renderAgentTradingCardPng({
  accentColor: resolvedAccentColor,
  agentAvatarUrl,
  avatarPngDataUrl,
  agentId,
  agentName,
  format,
  memoryLevel,
  templateUrl,
}: RenderAgentTradingCardInput): Promise<string> {
  const canvas = document.createElement("canvas");
  canvas.width = AGENT_CARD_WIDTH;
  canvas.height = AGENT_CARD_HEIGHT;
  const context = canvas.getContext("2d");
  if (!context) throw new Error("Canvas is unavailable.");

  const template = await loadImage(templateUrl);
  context.fillStyle = "white";
  context.fillRect(0, 0, AGENT_CARD_WIDTH, AGENT_CARD_HEIGHT);
  context.drawImage(template, 0, 0, AGENT_CARD_WIDTH, AGENT_CARD_HEIGHT);

  const avatar = await loadPortableAvatar(avatarPngDataUrl ?? agentAvatarUrl);
  const accentColor =
    resolvedAccentColor ??
    (avatar
      ? (sampleAvatarColor(avatar) ?? fallbackAgentCardColor(agentId))
      : fallbackAgentCardColor(agentId));

  drawTintedFoilFrame(context, accentColor);
  drawAvatar(context, avatar, accentColor, agentName);
  await loadCardFonts();
  drawVariableCardContent(context, {
    agentId,
    agentName,
    format,
    memoryLevel,
  });

  const pngDataUrl = canvas.toDataURL("image/png");
  if (
    !pngDataUrl.startsWith("data:image/png;base64,") ||
    pngDataUrl.length < 100
  ) {
    throw new Error("Card PNG encoding failed.");
  }
  return pngDataUrl;
}

async function loadPortableAvatar(
  avatarUrl: string | null,
): Promise<HTMLImageElement | null> {
  const source = avatarUrl?.trim();
  if (!source) return null;

  const portableSource = isRasterDataUrl(source)
    ? source
    : await resolveSnapshotAvatarPng(source);
  if (!portableSource) return null;
  try {
    return await loadImage(portableSource);
  } catch {
    return null;
  }
}

function isRasterDataUrl(value: string): boolean {
  return /^data:image\/(?:png|jpe?g|gif|webp)(?:;[^,]*)?,/iu.test(value);
}

function loadImage(source: string): Promise<HTMLImageElement> {
  return new Promise((resolve, reject) => {
    const image = new Image();
    image.decoding = "async";
    image.onload = () => resolve(image);
    image.onerror = () =>
      reject(new Error("Card artwork could not be loaded."));
    image.src = source;
  });
}

function sampleAvatarColor(avatar: HTMLImageElement): string | null {
  const canvas = document.createElement("canvas");
  canvas.width = SAMPLE_SIZE;
  canvas.height = SAMPLE_SIZE;
  const context = canvas.getContext("2d", { willReadFrequently: true });
  if (!context) return null;
  context.drawImage(avatar, 0, 0, SAMPLE_SIZE, SAMPLE_SIZE);
  return sampleRepresentativeAvatarColor(
    context.getImageData(0, 0, SAMPLE_SIZE, SAMPLE_SIZE).data,
  );
}

function drawTintedFoilFrame(
  context: CanvasRenderingContext2D,
  accentColor: string,
) {
  context.save();
  context.globalCompositeOperation = "color";
  context.globalAlpha = 0.62;
  context.fillStyle = accentColor;
  context.beginPath();
  roundedRect(context, 0, 0, AGENT_CARD_WIDTH, AGENT_CARD_HEIGHT, 80);
  roundedRect(
    context,
    30,
    31,
    AGENT_CARD_WIDTH - 60,
    AGENT_CARD_HEIGHT - 62,
    50,
  );
  context.fill("evenodd");
  context.restore();
}

function drawAvatar(
  context: CanvasRenderingContext2D,
  avatar: HTMLImageElement | null,
  accentColor: string,
  agentName: string,
) {
  context.save();
  context.beginPath();
  context.arc(AVATAR_CENTER_X, AVATAR_CENTER_Y, AVATAR_RADIUS, 0, Math.PI * 2);
  context.clip();
  context.fillStyle = accentColor;
  context.fillRect(AVATAR_X, AVATAR_Y, AVATAR_SIZE, AVATAR_SIZE);
  if (avatar) {
    drawImageCover(
      context,
      avatar,
      AVATAR_X,
      AVATAR_Y,
      AVATAR_SIZE,
      AVATAR_SIZE,
    );
  } else {
    context.fillStyle = "white";
    context.font = "700 250px Inter, sans-serif";
    context.textAlign = "center";
    context.textBaseline = "middle";
    context.fillText(getInitials(agentName), AVATAR_CENTER_X, 760);
  }
  context.restore();
}

function drawImageCover(
  context: CanvasRenderingContext2D,
  image: HTMLImageElement,
  x: number,
  y: number,
  width: number,
  height: number,
) {
  const sourceWidth = image.naturalWidth || image.width;
  const sourceHeight = image.naturalHeight || image.height;
  if (sourceWidth <= 0 || sourceHeight <= 0) return;
  const scale = Math.max(width / sourceWidth, height / sourceHeight);
  const cropWidth = width / scale;
  const cropHeight = height / scale;
  context.drawImage(
    image,
    (sourceWidth - cropWidth) / 2,
    (sourceHeight - cropHeight) / 2,
    cropWidth,
    cropHeight,
    x,
    y,
    width,
    height,
  );
}

async function loadCardFonts() {
  if (!document.fonts) return;
  await Promise.allSettled([
    document.fonts.load("800 68px Inter"),
    document.fonts.load("700 36px Inter"),
    document.fonts.load("600 40px Inter"),
    document.fonts.load("400 36px Inter"),
  ]);
}

function drawVariableCardContent(
  context: CanvasRenderingContext2D,
  {
    agentId,
    agentName,
    format,
    memoryLevel,
  }: Omit<
    RenderAgentTradingCardInput,
    "accentColor" | "agentAvatarUrl" | "avatarPngDataUrl" | "templateUrl"
  >,
) {
  const title = truncateAgentCardTitle(agentName);
  context.fillStyle = "white";
  context.fillRect(992, 105, 132, 66);
  context.fillRect(99, 1370, 1028, 390);

  context.fillStyle = "black";
  context.textBaseline = "alphabetic";
  context.textAlign = "right";
  context.font = "600 42px Inter, sans-serif";
  context.fillText(stableAgentCardNumber(agentId), 1110, 151);

  context.textAlign = "left";
  context.font = `800 ${agentCardTitleSize(title)}px Inter, sans-serif`;
  context.fillText(title, 115, 1465);
  context.font = "600 40px Inter, sans-serif";
  context.fillText("SHAREABLE BUZZ AGENT", 115, 1535);

  context.lineWidth = 4;
  context.strokeStyle = "black";
  context.beginPath();
  context.moveTo(120, 1633);
  context.lineTo(120, 1715);
  context.moveTo(663, 1633);
  context.lineTo(663, 1715);
  context.stroke();

  context.font = "700 36px Inter, sans-serif";
  context.fillText("Memory", 139, 1663);
  context.fillText("Format", 682, 1663);
  context.font = "400 36px Inter, sans-serif";
  context.fillText(AGENT_CARD_MEMORY_LABELS[memoryLevel], 139, 1710);
  context.fillText(agentCardFormatLabel(format), 682, 1710);
}

function roundedRect(
  context: CanvasRenderingContext2D,
  x: number,
  y: number,
  width: number,
  height: number,
  radius: number,
) {
  context.moveTo(x + radius, y);
  context.lineTo(x + width - radius, y);
  context.quadraticCurveTo(x + width, y, x + width, y + radius);
  context.lineTo(x + width, y + height - radius);
  context.quadraticCurveTo(
    x + width,
    y + height,
    x + width - radius,
    y + height,
  );
  context.lineTo(x + radius, y + height);
  context.quadraticCurveTo(x, y + height, x, y + height - radius);
  context.lineTo(x, y + radius);
  context.quadraticCurveTo(x, y, x + radius, y);
  context.closePath();
}
