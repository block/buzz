import * as React from "react";
import { motion, useReducedMotion } from "motion/react";

import tradingCardTemplateUrl from "@/features/agents/assets/buzz-trading-card-template.svg";
import { useAvatarPresentation } from "@/features/profile/avatarPresentationStore";
import type {
  SnapshotFormat,
  SnapshotMemoryLevel,
} from "@/shared/api/tauriPersonas";
import { getAvatarSnapshotUrl } from "@/shared/lib/animatedAvatar";
import { getInitials } from "@/shared/lib/initials";
import { rewriteRelayUrl } from "@/shared/lib/mediaUrl";
import { fetchMediaBytes } from "@/shared/api/tauriMedia";
import { useMediaProxyPort } from "@/shared/lib/useMediaProxyPort";
import {
  fallbackAgentCardColor,
  sampleRepresentativeAvatarColor,
} from "./agentCardPreviewColor";
import {
  AGENT_CARD_MEMORY_LABELS,
  agentCardTitleSize,
  stableAgentCardNumber,
  truncateAgentCardTitle,
} from "./agentTradingCardContent";
import "./agent-trading-card-preview.css";

const AVATAR_SAMPLE_SIZE = 24;
const MAX_EXPORTED_AVATAR_DIMENSION = 1024;
const MAX_CARD_TILT_DEGREES = 4;

export type AgentTradingCardAppearance = {
  accentColor: string;
  avatarPngDataUrl: string | null;
};

export function AgentTradingCardPreview({
  agentAvatarUrl,
  agentId,
  agentName,
  cardImageUrl,
  format,
  memoryLevel,
  onAppearanceChange,
}: {
  agentAvatarUrl: string | null;
  agentId: string;
  agentName: string;
  /** Finished custom-card art. When present, it replaces the SVG body while
   * retaining the same clipped foil, hover plane, and entrance reveal. */
  cardImageUrl?: string;
  format: SnapshotFormat;
  memoryLevel: SnapshotMemoryLevel;
  onAppearanceChange?: (appearance: AgentTradingCardAppearance) => void;
}) {
  const avatarClipId = React.useId().replaceAll(":", "");
  const shouldReduceMotion = useReducedMotion();
  const avatarPresentation = useAvatarPresentation(agentAvatarUrl);
  useMediaProxyPort();
  const presentedAvatarUrl = avatarPresentation?.displayUrl ?? agentAvatarUrl;
  const snapshotUrl = getAvatarSnapshotUrl(presentedAvatarUrl);
  const avatarSrc = snapshotUrl ? rewriteRelayUrl(snapshotUrl) : null;
  const fallbackColor = React.useMemo(
    () => fallbackAgentCardColor(agentId),
    [agentId],
  );
  const [palette, setPalette] = React.useState(() => ({
    avatarPngDataUrl: null as string | null,
    color: fallbackColor,
    ready: snapshotUrl === null,
    sourceUrl: snapshotUrl,
  }));
  const [failedAvatarSrc, setFailedAvatarSrc] = React.useState<string | null>(
    null,
  );
  const [entranceComplete, setEntranceComplete] = React.useState(
    Boolean(shouldReduceMotion),
  );
  const [cardImageReady, setCardImageReady] = React.useState(
    cardImageUrl === undefined,
  );
  const hoverBoundsRef = React.useRef<DOMRect | null>(null);
  const pointerFrameRef = React.useRef<number | null>(null);
  const pendingPointerRef = React.useRef<{
    clientX: number;
    clientY: number;
    element: HTMLDivElement;
  } | null>(null);
  const shimmerRef = React.useRef<HTMLDivElement>(null);

  React.useEffect(() => {
    if (shouldReduceMotion) setEntranceComplete(true);
  }, [shouldReduceMotion]);

  React.useEffect(() => {
    setCardImageReady(cardImageUrl === undefined);
  }, [cardImageUrl]);

  React.useEffect(
    () => () => {
      if (pointerFrameRef.current !== null) {
        cancelAnimationFrame(pointerFrameRef.current);
      }
    },
    [],
  );

  React.useEffect(() => {
    setEntranceComplete(Boolean(shouldReduceMotion));
    if (!avatarSrc || !snapshotUrl) {
      setPalette({
        avatarPngDataUrl: null,
        color: fallbackColor,
        ready: true,
        sourceUrl: snapshotUrl,
      });
      return;
    }

    setPalette({
      avatarPngDataUrl: null,
      color: fallbackColor,
      ready: false,
      sourceUrl: snapshotUrl,
    });

    let cancelled = false;
    let blobUrl: string | null = null;

    const sampleImage = async () => {
      try {
        const image = await loadSamplingImage(snapshotUrl, avatarSrc, (url) => {
          if (cancelled) {
            URL.revokeObjectURL(url);
          } else {
            blobUrl = url;
          }
        });
        if (cancelled) return;
        const canvas = document.createElement("canvas");
        canvas.width = AVATAR_SAMPLE_SIZE;
        canvas.height = AVATAR_SAMPLE_SIZE;
        const context = canvas.getContext("2d", { willReadFrequently: true });
        if (!context) return;
        context.drawImage(image, 0, 0, canvas.width, canvas.height);
        const sampledColor = sampleRepresentativeAvatarColor(
          context.getImageData(0, 0, canvas.width, canvas.height).data,
        );
        setPalette({
          avatarPngDataUrl: rasterizeDecodedAvatar(image),
          color: sampledColor ?? fallbackColor,
          ready: true,
          sourceUrl: snapshotUrl,
        });
      } catch {
        // An external avatar without CORS can still prevent sampling. The
        // deterministic per-agent fallback gives the foil a stable palette.
        if (!cancelled) {
          setPalette({
            avatarPngDataUrl: null,
            color: fallbackColor,
            ready: true,
            sourceUrl: snapshotUrl,
          });
        }
      }
    };
    void sampleImage();

    return () => {
      cancelled = true;
      if (blobUrl) URL.revokeObjectURL(blobUrl);
    };
  }, [avatarSrc, fallbackColor, shouldReduceMotion, snapshotUrl]);

  const paletteReady =
    palette.sourceUrl === snapshotUrl &&
    (snapshotUrl === null || palette.ready);
  const revealReady = paletteReady && cardImageReady;
  const accentColor =
    palette.sourceUrl === snapshotUrl ? palette.color : fallbackColor;
  React.useEffect(() => {
    if (!onAppearanceChange || !paletteReady) return;
    onAppearanceChange({
      accentColor,
      avatarPngDataUrl:
        palette.sourceUrl === snapshotUrl ? palette.avatarPngDataUrl : null,
    });
  }, [accentColor, onAppearanceChange, palette, paletteReady, snapshotUrl]);
  const title = truncateAgentCardTitle(agentName);
  const cardNumber = stableAgentCardNumber(agentId);
  const style = {
    "--agent-card-accent": accentColor,
  } as AgentCardPreviewStyle;
  const showAvatar = avatarSrc !== null && failedAvatarSrc !== avatarSrc;

  const updatePointerEffect = React.useCallback(
    (element: HTMLDivElement, clientX: number, clientY: number) => {
      if (shouldReduceMotion || !entranceComplete) return;
      const bounds = hoverBoundsRef.current ?? element.getBoundingClientRect();
      const normalizedX = clamp(
        ((clientX - bounds.left) / bounds.width - 0.5) * 2,
        -1,
        1,
      );
      const normalizedY = clamp(
        ((clientY - bounds.top) / bounds.height - 0.5) * 2,
        -1,
        1,
      );
      shimmerRef.current?.style.setProperty(
        "--agent-card-pointer-x",
        `${((normalizedX + 1) / 2) * 100}%`,
      );
      shimmerRef.current?.style.setProperty(
        "--agent-card-pointer-y",
        `${((normalizedY + 1) / 2) * 100}%`,
      );
      element.style.transform = `rotateX(${-normalizedY * MAX_CARD_TILT_DEGREES}deg) rotateY(${normalizedX * MAX_CARD_TILT_DEGREES}deg)`;
    },
    [entranceComplete, shouldReduceMotion],
  );

  const schedulePointerEffect = React.useCallback(
    (element: HTMLDivElement, clientX: number, clientY: number) => {
      pendingPointerRef.current = { clientX, clientY, element };
      if (pointerFrameRef.current !== null) return;

      pointerFrameRef.current = requestAnimationFrame(() => {
        pointerFrameRef.current = null;
        const pendingPointer = pendingPointerRef.current;
        pendingPointerRef.current = null;
        if (!pendingPointer) return;
        updatePointerEffect(
          pendingPointer.element,
          pendingPointer.clientX,
          pendingPointer.clientY,
        );
      });
    },
    [updatePointerEffect],
  );

  const resetPointerEffect = React.useCallback((element: HTMLDivElement) => {
    hoverBoundsRef.current = null;
    pendingPointerRef.current = null;
    if (pointerFrameRef.current !== null) {
      cancelAnimationFrame(pointerFrameRef.current);
      pointerFrameRef.current = null;
    }
    shimmerRef.current?.style.setProperty("--agent-card-pointer-x", "50%");
    shimmerRef.current?.style.setProperty("--agent-card-pointer-y", "50%");
    element.style.transform = "none";
  }, []);

  return (
    <div className="flex w-full justify-center">
      <motion.div
        animate={
          shouldReduceMotion || revealReady
            ? { opacity: 1, rotateY: 0, scale: 1 }
            : { opacity: 0, rotateY: -105, scale: 0.86 }
        }
        className="agent-trading-card-preview__stage w-full max-w-[16.5rem]"
        data-entrance-complete={entranceComplete}
        data-palette-ready={paletteReady}
        data-reveal-ready={revealReady}
        data-testid="agent-card-stage"
        initial={
          shouldReduceMotion
            ? false
            : { opacity: 0, rotateY: -105, scale: 0.86 }
        }
        style={style}
        transition={
          shouldReduceMotion
            ? { duration: 0 }
            : revealReady
              ? { delay: 0.24, duration: 1, ease: [0.22, 1, 0.36, 1] }
              : { duration: 0 }
        }
      >
        <div
          aria-hidden
          className="agent-trading-card-preview__burst"
          data-testid="agent-card-color-burst"
        >
          <span className="agent-trading-card-preview__burst-lobe agent-trading-card-preview__burst-lobe--one" />
          <span className="agent-trading-card-preview__burst-lobe agent-trading-card-preview__burst-lobe--two" />
          <span
            className="agent-trading-card-preview__burst-lobe agent-trading-card-preview__burst-lobe--three"
            onAnimationEnd={() => setEntranceComplete(true)}
          />
        </div>
        <div
          aria-label="Shareable agent card"
          className="agent-trading-card-preview__tilt-plane"
          data-accent-color={accentColor}
          data-testid="agent-card-live-preview"
          onPointerEnter={(event) => {
            if (event.pointerType === "touch") return;
            hoverBoundsRef.current =
              event.currentTarget.getBoundingClientRect();
            schedulePointerEffect(
              event.currentTarget,
              event.clientX,
              event.clientY,
            );
          }}
          onPointerLeave={(event) => resetPointerEffect(event.currentTarget)}
          onPointerMove={(event) =>
            event.pointerType === "touch"
              ? undefined
              : schedulePointerEffect(
                  event.currentTarget,
                  event.clientX,
                  event.clientY,
                )
          }
          role="img"
        >
          <div className="agent-trading-card-preview">
            {cardImageUrl ? (
              <img
                alt=""
                aria-hidden
                className="agent-trading-card-preview__generated"
                data-testid="agent-custom-card-generated-image"
                draggable={false}
                onError={() => setCardImageReady(true)}
                onLoad={() => setCardImageReady(true)}
                src={cardImageUrl}
              />
            ) : (
              <>
                <img
                  alt=""
                  aria-hidden
                  className="agent-trading-card-preview__template"
                  data-testid="agent-card-template"
                  draggable={false}
                  src={tradingCardTemplateUrl}
                />
                <div
                  aria-hidden
                  className="agent-trading-card-preview__palette"
                />
                <svg
                  aria-hidden
                  className="agent-trading-card-preview__content"
                  role="presentation"
                  viewBox="0 0 1227 1839"
                >
                  <defs>
                    <clipPath id={avatarClipId}>
                      <circle cx="607" cy="740" r="432" />
                    </clipPath>
                  </defs>

                  <circle cx="607" cy="740" fill={accentColor} r="432" />
                  {showAvatar ? (
                    <image
                      clipPath={`url(#${avatarClipId})`}
                      data-testid="agent-card-preview-avatar"
                      height="864"
                      href={avatarSrc}
                      onError={() => setFailedAvatarSrc(avatarSrc)}
                      onLoad={() => setFailedAvatarSrc(null)}
                      preserveAspectRatio="xMidYMid slice"
                      width="864"
                      x="175"
                      y="308"
                    />
                  ) : (
                    <text
                      dominantBaseline="middle"
                      fill="white"
                      fontFamily="Inter, sans-serif"
                      fontSize="250"
                      fontWeight="700"
                      textAnchor="middle"
                      x="607"
                      y="760"
                    >
                      {getInitials(agentName)}
                    </text>
                  )}

                  <rect fill="white" height="66" width="132" x="992" y="105" />
                  <text
                    fill="black"
                    fontFamily="Inter, sans-serif"
                    fontSize="42"
                    fontWeight="600"
                    textAnchor="end"
                    x="1110"
                    y="151"
                  >
                    {cardNumber}
                  </text>

                  <rect
                    fill="white"
                    height="390"
                    width="1028"
                    x="99"
                    y="1370"
                  />
                  <text
                    data-testid="agent-card-preview-name"
                    fill="black"
                    fontFamily="Inter, sans-serif"
                    fontSize={agentCardTitleSize(title)}
                    fontWeight="800"
                    x="115"
                    y="1465"
                  >
                    {title}
                  </text>
                  <text
                    fill="black"
                    fontFamily="Inter, sans-serif"
                    fontSize="40"
                    fontWeight="600"
                    x="115"
                    y="1535"
                  >
                    SHAREABLE BUZZ AGENT
                  </text>

                  <line
                    stroke="black"
                    strokeWidth="4"
                    x1="120"
                    x2="120"
                    y1="1633"
                    y2="1715"
                  />
                  <text
                    data-testid="agent-card-preview-memory"
                    fill="black"
                    fontFamily="Inter, sans-serif"
                    fontSize="36"
                    x="139"
                    y="1663"
                  >
                    <tspan fontWeight="700">Memory</tspan>
                    <tspan x="139" y="1710">
                      {AGENT_CARD_MEMORY_LABELS[memoryLevel]}
                    </tspan>
                  </text>

                  <line
                    stroke="black"
                    strokeWidth="4"
                    x1="663"
                    x2="663"
                    y1="1633"
                    y2="1715"
                  />
                  <text
                    data-testid="agent-card-preview-format"
                    fill="black"
                    fontFamily="Inter, sans-serif"
                    fontSize="36"
                    x="682"
                    y="1663"
                  >
                    <tspan fontWeight="700">Format</tspan>
                    <tspan x="682" y="1710">
                      {format.toUpperCase()}
                    </tspan>
                  </text>
                </svg>
              </>
            )}
            <div
              ref={shimmerRef}
              aria-hidden
              className="agent-trading-card-preview__shimmer"
            />
          </div>
        </div>
      </motion.div>
    </div>
  );
}

type AgentCardPreviewStyle = React.CSSProperties & {
  "--agent-card-accent": string;
};

function clamp(value: number, minimum: number, maximum: number): number {
  return Math.min(Math.max(value, minimum), maximum);
}

async function loadSamplingImage(
  sourceUrl: string,
  displayUrl: string,
  onBlobUrl: (url: string) => void,
): Promise<HTMLImageElement> {
  if (isRelayMediaUrl(sourceUrl)) {
    try {
      const bytes = await fetchMediaBytes(sourceUrl);
      const blobUrl = URL.createObjectURL(
        new Blob([bytes], { type: imageMimeType(sourceUrl) }),
      );
      onBlobUrl(blobUrl);
      return await decodeImage(blobUrl, false);
    } catch {
      // Fall through to the display URL for browser-readable sources and the
      // E2E bridge. Native relay media normally succeeds through IPC.
    }
  }
  return await decodeImage(displayUrl, /^https?:/iu.test(displayUrl));
}

function decodeImage(
  source: string,
  crossOrigin: boolean,
): Promise<HTMLImageElement> {
  const image = new Image();
  if (crossOrigin) image.crossOrigin = "anonymous";
  image.referrerPolicy = "no-referrer";
  image.src = source;
  return image.decode().then(() => image);
}

function isRelayMediaUrl(url: string): boolean {
  return /^https?:\/\/[^/]+\/media\/[\da-f]{64}(?:\.thumb)?\.(?:jpe?g|png|gif|webp)(?:\?.*)?$/iu.test(
    url,
  );
}

function imageMimeType(url: string): string {
  const pathname = (() => {
    try {
      return new URL(url).pathname.toLowerCase();
    } catch {
      return url.toLowerCase();
    }
  })();
  if (pathname.endsWith(".png")) return "image/png";
  if (pathname.endsWith(".gif")) return "image/gif";
  if (pathname.endsWith(".webp")) return "image/webp";
  return "image/jpeg";
}

function rasterizeDecodedAvatar(image: HTMLImageElement): string {
  const sourceWidth = image.naturalWidth || image.width || 512;
  const sourceHeight = image.naturalHeight || image.height || 512;
  const scale = Math.min(
    1,
    MAX_EXPORTED_AVATAR_DIMENSION / Math.max(sourceWidth, sourceHeight),
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
