import * as React from "react";
import { createPortal } from "react-dom";
import ReactMarkdown, {
  type Components,
  defaultUrlTransform,
} from "react-markdown";
import { Copy, Download, FileText, ZoomIn, ZoomOut } from "lucide-react";
import { useReducedMotion } from "motion/react";
import remarkBreaks from "remark-breaks";
import remarkGfm from "remark-gfm";
import { toast } from "sonner";

import {
  getSingletonHighlighter,
  type HighlighterGeneric,
  type BundledLanguage,
  type BundledTheme,
  type ThemedToken,
} from "shiki";

import { useTheme } from "@/shared/theme/ThemeProvider";
import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import {
  isMessageLink,
  parseMessageLink,
  type ParsedMessageLink,
} from "@/features/messages/lib/messageLink";
import { UserProfilePopover } from "@/features/profile/ui/UserProfilePopover";
import { invokeTauri } from "@/shared/api/tauri";
import type { Channel } from "@/shared/api/types";
import { useChannelNavigation } from "@/shared/context/ChannelNavigationContext";
import { copyCodeBlockToClipboard } from "@/shared/lib/codeBlockClipboard";
import { cn } from "@/shared/lib/cn";
import { rewriteRelayUrl } from "@/shared/lib/mediaUrl";
import rehypeImageGallery from "@/shared/lib/rehypeImageGallery";
import rehypeSearchHighlight from "@/shared/lib/rehypeSearchHighlight";
import remarkChannelLinks from "@/shared/lib/remarkChannelLinks";
import remarkCustomEmoji, {
  type CustomEmoji,
} from "@/shared/lib/remarkCustomEmoji";
import remarkMentions from "@/shared/lib/remarkMentions";
import remarkSpoilers from "@/shared/lib/remarkSpoilers";
import remarkMessageLinks from "@/features/messages/lib/remarkMessageLinks";
import { Button } from "@/shared/ui/button";
import {
  INLINE_CODE_CHIP_CLASS,
  MENTION_CHIP_BASE_CLASSES,
  MENTION_CHIP_HOVER_CLASSES,
  MENTION_CHIP_PREFIX_CLASS,
  MESSAGE_MARKDOWN_CLASS,
} from "@/shared/ui/mentionChip";
import { Popover, PopoverContent, PopoverTrigger } from "@/shared/ui/popover";
import {
  POPOVER_CUSTOM_ENTER_MOTION_CLASS,
  POPOVER_SHADOW_STYLE,
  POPOVER_SURFACE_CLASS,
} from "@/shared/ui/popoverSurface";
import { SpoilerParticles } from "@/shared/ui/SpoilerParticles";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";

import {
  classifyChildren,
  hasBlockMedia,
  isImageOnlyParagraph,
  shallowArrayEqual,
} from "./markdownUtils";
import { resolveFileCard } from "./markdownFileCard";
import { VideoPlayer, type VideoReviewContext } from "./VideoPlayer";

type ImetaEntry = {
  dim?: string;
  image?: string;
  thumb?: string;
  m?: string;
  size?: number;
  filename?: string;
  duration?: number;
};

type ImetaLookup = Map<string, ImetaEntry>;

let shikiHighlighter: HighlighterGeneric<BundledLanguage, BundledTheme> | null =
  null;
let shikiInitPromise: Promise<void> | null = null;
const loadedLangs = new Set<string>();
const loadedThemes = new Set<string>();
const tokenCache = new Map<string, ThemedToken[][]>();
const MAX_CACHE_ENTRIES = 100;
const MAX_LOADED_LANGUAGES = 30;
const MAX_HIGHLIGHT_LINES = 150;
const CODE_BLOCK_CLASS =
  "code-block-lines block min-w-full whitespace-pre font-mono text-sm font-medium text-foreground";
const DIFF_ADD_RE = /\s*\/\/\s*\[!code\s*\+\+\]\s*$/;
const DIFF_REMOVE_RE = /\s*\/\/\s*\[!code\s*--\]\s*$/;

function ensureHighlighter(): Promise<void> {
  if (shikiHighlighter) return Promise.resolve();
  if (!shikiInitPromise) {
    shikiInitPromise = getSingletonHighlighter({
      themes: [],
      langs: [],
    }).then((h) => {
      shikiHighlighter = h;
    });
  }
  return shikiInitPromise;
}

function extractLanguage(className?: string): string {
  if (typeof className !== "string") return "";
  const match = className.match(/language-(\S+)/);
  return match ? match[1] : "";
}

function stripDiffMarker(tokens: ThemedToken[], marker: RegExp): ThemedToken[] {
  const last = tokens[tokens.length - 1];
  if (!last) return tokens;
  const stripped = last.content.replace(marker, "");
  if (stripped === last.content) return tokens;
  if (stripped === "") return tokens.slice(0, -1);
  return [...tokens.slice(0, -1), { ...last, content: stripped }];
}

function useStableArray<T>(arr: T[]): T[] {
  const ref = React.useRef(arr);
  if (
    arr.length !== ref.current.length ||
    arr.some((item, i) => item !== ref.current[i])
  ) {
    ref.current = arr;
  }
  return ref.current;
}

function aspectRatioFromDim(dim?: string): number | undefined {
  if (!dim) return undefined;
  const match = dim.match(/^(\d+)x(\d+)$/i);
  if (!match) return undefined;
  const width = Number(match[1]);
  const height = Number(match[2]);
  if (!Number.isFinite(width) || !Number.isFinite(height) || height <= 0) {
    return undefined;
  }
  return width / height;
}

/**
 * Parse a NIP-92 `dim` value ("WxH") into intrinsic pixel dimensions. Used to
 * stamp explicit `width`/`height` attributes on inline images so the browser
 * reserves aspect-ratio-correct layout space *before* the image decodes. This
 * is what keeps the timeline from jumping when a tall image loads late — the
 * row's height is known at first paint instead of growing from ~0 on load.
 */
function dimensionsFromDim(
  dim?: string,
): { width: number; height: number } | undefined {
  if (!dim) return undefined;
  const match = dim.match(/^(\d+)x(\d+)$/i);
  if (!match) return undefined;
  const width = Number(match[1]);
  const height = Number(match[2]);
  if (
    !Number.isFinite(width) ||
    !Number.isFinite(height) ||
    width <= 0 ||
    height <= 0
  ) {
    return undefined;
  }
  return { width, height };
}

function isInsideHiddenSpoiler(element: Element): boolean {
  return (
    element.closest('.buzz-spoiler[data-spoiler][data-revealed="false"]') !==
    null
  );
}

/**
 * Video review context flows through React context instead of
 * `createMarkdownComponents` arguments. The component map must keep a stable
 * identity across re-renders: a new map means new element types, which makes
 * React unmount and remount every rendered node — including `<video>`
 * elements, killing playback (and any in-progress review comment draft)
 * whenever the timeline re-renders.
 */
const VideoReviewMarkdownContext = React.createContext<
  VideoReviewContext | undefined
>(undefined);

type MarkdownRuntime = {
  agentMentionPubkeysByName?: Record<string, string>;
  channels: Channel[];
  imetaByUrl?: ImetaLookup;
  mentionPubkeysByName?: Record<string, string>;
  onOpenChannel: (channelId: string) => void;
  onOpenMessageLink: (link: ParsedMessageLink) => void;
};

function useLatestRef<T>(value: T) {
  const ref = React.useRef(value);
  ref.current = value;
  return ref;
}

function MarkdownVideoPlayer({
  alt,
  entry,
  resolvedSrc,
  src,
}: {
  alt?: string;
  entry?: ImetaEntry;
  resolvedSrc: string;
  src?: string;
}) {
  const videoReviewContext = React.useContext(VideoReviewMarkdownContext);
  // Look up poster frame from imeta tags (NIP-71 `image` field).
  // Fall back to `thumb` for compatibility with older events.
  const posterUrl = entry?.image ?? entry?.thumb;
  const resolvedPoster = posterUrl ? rewriteRelayUrl(posterUrl) : undefined;
  const resolvedReviewContext = React.useMemo(
    () =>
      videoReviewContext
        ? {
            ...videoReviewContext,
            title:
              videoReviewContext.title ?? entry?.filename ?? alt ?? "Video",
          }
        : undefined,
    [alt, entry?.filename, videoReviewContext],
  );

  return (
    <VideoPlayer
      src={resolvedSrc}
      aspectRatio={aspectRatioFromDim(entry?.dim)}
      poster={resolvedPoster}
      durationSeconds={entry?.duration}
      reviewKey={src ?? resolvedSrc}
      reviewContext={resolvedReviewContext}
    />
  );
}

/**
 * `urlTransform` for `<ReactMarkdown>` that preserves `buzz://message?…`
 * links. The default transform strips unknown schemes (returns `""`) before
 * the `a` component override can see them, which would break copy → paste →
 * click end-to-end. Everything else delegates to `defaultUrlTransform`.
 */
function messageLinkUrlTransform(value: string, key: string): string {
  if (key === "href" && isMessageLink(value)) {
    return value;
  }
  return defaultUrlTransform(value);
}

type MarkdownProps = {
  channelNames?: string[];
  className?: string;
  content: string;
  customEmoji?: CustomEmoji[];
  imetaByUrl?: ImetaLookup;
  interactive?: boolean;
  agentMentionPubkeysByName?: Record<string, string>;
  mentionNames?: string[];
  mentionPubkeysByName?: Record<string, string>;
  searchQuery?: string;
  videoReviewContext?: VideoReviewContext;
};

type ImageLightboxBox = {
  height: number;
  left: number;
  top: number;
  width: number;
};

const IMAGE_LIGHTBOX_ENTER_MS = 260;
const IMAGE_LIGHTBOX_EXIT_MS = 170;
const IMAGE_LIGHTBOX_FADE_ENTER_MS = 180;
const IMAGE_LIGHTBOX_FADE_EXIT_MS = 90;
const IMAGE_LIGHTBOX_REDUCED_MOTION_MS = 100;
const IMAGE_LIGHTBOX_ZOOM_TRANSITION_MS = 80;
const IMAGE_LIGHTBOX_BASE_VIEWPORT_RATIO = 0.8;
const IMAGE_LIGHTBOX_CONTROL_SUPPRESS_CLOSE_MS = 450;
const IMAGE_LIGHTBOX_TRACKPAD_ZOOM_IDLE_MS = 120;
const IMAGE_LIGHTBOX_WHEEL_ZOOM_SPEED = 0.002;
const IMAGE_LIGHTBOX_WHEEL_ZOOM_MAX_DELTA = 0.2;
const IMAGE_LIGHTBOX_MIN_ZOOM = 1;
const IMAGE_LIGHTBOX_MAX_ZOOM = 3;
const IMAGE_LIGHTBOX_ZOOM_STEP = 0.05;
const IMAGE_LIGHTBOX_EASE_OUT = "cubic-bezier(0.23, 1, 0.32, 1)";
const IMAGE_LIGHTBOX_EASE_IN_OUT = "cubic-bezier(0.77, 0, 0.175, 1)";

function imageLightboxBoxFromRect(rect: DOMRect): ImageLightboxBox {
  return {
    height: rect.height,
    left: rect.left,
    top: rect.top,
    width: rect.width,
  };
}

function imageLightboxTargetBox(sourceBox: ImageLightboxBox): ImageLightboxBox {
  const viewportWidth = window.innerWidth;
  const viewportHeight = window.innerHeight;
  const horizontalPadding = Math.min(80, Math.max(16, viewportWidth * 0.0625));
  const verticalPadding = Math.min(24, Math.max(16, viewportHeight * 0.033));
  const maxWidth = Math.max(
    1,
    Math.min(
      viewportWidth - horizontalPadding * 2,
      viewportWidth * IMAGE_LIGHTBOX_BASE_VIEWPORT_RATIO,
    ),
  );
  const maxHeight = Math.max(
    1,
    Math.min(
      viewportHeight - verticalPadding * 2,
      viewportHeight * IMAGE_LIGHTBOX_BASE_VIEWPORT_RATIO,
    ),
  );
  const scale = Math.min(
    maxWidth / sourceBox.width,
    maxHeight / sourceBox.height,
  );
  const width = Math.max(1, sourceBox.width * scale);
  const height = Math.max(1, sourceBox.height * scale);

  return {
    height,
    left: (viewportWidth - width) / 2,
    top: (viewportHeight - height) / 2,
    width,
  };
}

function imageLightboxStyle(box: ImageLightboxBox): React.CSSProperties {
  return {
    height: `${box.height}px`,
    left: `${box.left}px`,
    top: `${box.top}px`,
    width: `${box.width}px`,
  };
}

function clampImageLightboxZoom(value: number): number {
  return Math.min(
    IMAGE_LIGHTBOX_MAX_ZOOM,
    Math.max(IMAGE_LIGHTBOX_MIN_ZOOM, value),
  );
}

function normalizedWheelDeltaY(event: WheelEvent): number {
  if (event.deltaMode === WheelEvent.DOM_DELTA_LINE) {
    return event.deltaY * 16;
  }

  if (event.deltaMode === WheelEvent.DOM_DELTA_PAGE) {
    return event.deltaY * window.innerHeight;
  }

  return event.deltaY;
}

function imageLightboxTransform(
  sourceBox: ImageLightboxBox,
  targetBox: ImageLightboxBox,
): string {
  const scaleX = targetBox.width / Math.max(1, sourceBox.width);
  const scaleY = targetBox.height / Math.max(1, sourceBox.height);
  const translateX = targetBox.left - sourceBox.left;
  const translateY = targetBox.top - sourceBox.top;

  return `translate3d(${translateX}px, ${translateY}px, 0) scale(${scaleX}, ${scaleY})`;
}

function imageLightboxZoomBox(
  targetBox: ImageLightboxBox,
  zoom: number,
): ImageLightboxBox {
  const width = targetBox.width * zoom;
  const height = targetBox.height * zoom;

  return {
    height,
    left: targetBox.left + (targetBox.width - width) / 2,
    top: targetBox.top + (targetBox.height - height) / 2,
    width,
  };
}

type WebKitGestureLikeEvent = Event & {
  scale?: number;
};

type ImageContextMenuPosition = {
  x: number;
  y: number;
};

function getImageLightboxFocusableElements(
  container: HTMLElement,
): HTMLElement[] {
  return Array.from(
    container.querySelectorAll<HTMLElement>(
      [
        "a[href]",
        "button:not(:disabled)",
        "input:not(:disabled)",
        "select:not(:disabled)",
        "textarea:not(:disabled)",
        "[tabindex]:not([tabindex='-1'])",
      ].join(","),
    ),
  ).filter(
    (element) =>
      !element.hasAttribute("disabled") &&
      element.getAttribute("aria-hidden") !== "true" &&
      element.getClientRects().length > 0,
  );
}

function useDismissImageContextMenu(isOpen: boolean, onDismiss: () => void) {
  React.useEffect(() => {
    if (!isOpen) return;
    // Defer attaching the dismiss listeners until after the current event
    // loop turn. The right-click that opens the menu (a `contextmenu` on
    // mousedown) is often followed by a trailing `click`/`pointerup` on the
    // same interaction; attaching synchronously lets that trailing event —
    // and the platform `click` some webviews emit on right-button release —
    // immediately dismiss the menu, so it only flashes. Deferring guarantees
    // the opening interaction can never be the one that closes it.
    let attached = false;
    const timer = window.setTimeout(() => {
      attached = true;
      window.addEventListener("click", onDismiss);
      window.addEventListener("contextmenu", onDismiss);
      window.addEventListener("scroll", onDismiss, true);
    }, 0);
    return () => {
      window.clearTimeout(timer);
      if (attached) {
        window.removeEventListener("click", onDismiss);
        window.removeEventListener("contextmenu", onDismiss);
        window.removeEventListener("scroll", onDismiss, true);
      }
    };
  }, [isOpen, onDismiss]);
}

function ImageDownloadContextMenu({
  onDownload,
  position,
}: {
  onDownload: () => void;
  position: ImageContextMenuPosition;
}) {
  return (
    <div
      className={cn(
        "fixed z-[100] min-w-60 origin-top-left rounded-xl p-1 slide-in-from-top-1",
        POPOVER_CUSTOM_ENTER_MOTION_CLASS,
        POPOVER_SURFACE_CLASS,
      )}
      data-image-lightbox-controls=""
      style={{ ...POPOVER_SHADOW_STYLE, left: position.x, top: position.y }}
    >
      <button
        type="button"
        className="flex min-h-9 w-full cursor-default select-none items-center rounded-lg py-2 pl-2 pr-4 text-sm outline-hidden hover:bg-muted/50 hover:text-foreground"
        onClick={onDownload}
      >
        Download image
      </button>
    </div>
  );
}

function ImageZoomOverlay({
  alt,
  canDownload,
  onDownload,
  onClose,
  resolvedSrc,
  sourceBox,
}: {
  alt: string | undefined;
  canDownload: boolean;
  onDownload: () => void;
  onClose: () => void;
  resolvedSrc: string;
  sourceBox: ImageLightboxBox;
}) {
  const shouldReduceMotion = useReducedMotion();
  const prefersReducedMotion = shouldReduceMotion === true;
  const [phase, setPhase] = React.useState<
    "opening" | "open" | "closing" | "fading"
  >(() => (prefersReducedMotion ? "open" : "opening"));
  const [hasEntered, setHasEntered] = React.useState(prefersReducedMotion);
  const [isAdjustingZoom, setIsAdjustingZoom] = React.useState(false);
  const [menu, setMenu] = React.useState<ImageContextMenuPosition | null>(null);
  const [targetBox, setTargetBox] = React.useState(() =>
    imageLightboxTargetBox(sourceBox),
  );
  const [zoom, setZoom] = React.useState(IMAGE_LIGHTBOX_MIN_ZOOM);
  const controlPointerDownRef = React.useRef(false);
  const fadeTimerRef = React.useRef<number | null>(null);
  const closeTimerRef = React.useRef<number | null>(null);
  const dialogRef = React.useRef<HTMLDivElement | null>(null);
  const descriptionId = React.useId();
  const gestureScaleRef = React.useRef(1);
  const previouslyFocusedElementRef = React.useRef<HTMLElement | null>(null);
  const suppressCloseUntilRef = React.useRef(0);
  const zoomIdleTimerRef = React.useRef<number | null>(null);

  const markControlGesture = React.useCallback(() => {
    suppressCloseUntilRef.current =
      Date.now() + IMAGE_LIGHTBOX_CONTROL_SUPPRESS_CLOSE_MS;
  }, []);
  const closeMenu = React.useCallback(() => setMenu(null), []);

  const finishZoomGestureSoon = React.useCallback(() => {
    if (zoomIdleTimerRef.current != null) {
      window.clearTimeout(zoomIdleTimerRef.current);
    }
    zoomIdleTimerRef.current = window.setTimeout(() => {
      setIsAdjustingZoom(false);
      zoomIdleTimerRef.current = null;
    }, IMAGE_LIGHTBOX_TRACKPAD_ZOOM_IDLE_MS);
  }, []);

  const setClampedZoom = React.useCallback((nextZoom: number) => {
    setZoom(clampImageLightboxZoom(nextZoom));
  }, []);

  const updateZoom = React.useCallback((updater: (zoom: number) => number) => {
    setZoom((currentZoom) => clampImageLightboxZoom(updater(currentZoom)));
  }, []);

  const close = React.useCallback(() => {
    if (closeTimerRef.current != null) return;

    if (prefersReducedMotion) {
      setPhase("fading");
      closeTimerRef.current = window.setTimeout(() => {
        onClose();
      }, IMAGE_LIGHTBOX_REDUCED_MOTION_MS);
      return;
    }

    setPhase("closing");
    fadeTimerRef.current = window.setTimeout(() => {
      setPhase("fading");
    }, IMAGE_LIGHTBOX_EXIT_MS);
    closeTimerRef.current = window.setTimeout(() => {
      onClose();
    }, IMAGE_LIGHTBOX_EXIT_MS + IMAGE_LIGHTBOX_FADE_EXIT_MS);
  }, [onClose, prefersReducedMotion]);

  useDismissImageContextMenu(Boolean(menu), closeMenu);

  React.useEffect(() => {
    if (prefersReducedMotion) {
      setPhase("open");
      return;
    }

    let secondFrame = 0;
    const firstFrame = window.requestAnimationFrame(() => {
      secondFrame = window.requestAnimationFrame(() => setPhase("open"));
    });

    return () => {
      window.cancelAnimationFrame(firstFrame);
      if (secondFrame) {
        window.cancelAnimationFrame(secondFrame);
      }
    };
  }, [prefersReducedMotion]);

  React.useEffect(() => {
    if (phase !== "open") {
      return;
    }

    if (prefersReducedMotion) {
      setHasEntered(true);
      return;
    }

    const timer = window.setTimeout(() => {
      setHasEntered(true);
    }, IMAGE_LIGHTBOX_ENTER_MS);

    return () => window.clearTimeout(timer);
  }, [phase, prefersReducedMotion]);

  React.useEffect(() => {
    previouslyFocusedElementRef.current =
      document.activeElement instanceof HTMLElement
        ? document.activeElement
        : null;
    dialogRef.current?.focus();
  }, []);

  React.useEffect(() => {
    const previousOverflow = document.body.style.overflow;
    document.body.style.overflow = "hidden";
    return () => {
      document.body.style.overflow = previousOverflow;
    };
  }, []);

  React.useEffect(() => {
    const dialog = dialogRef.current;
    if (!dialog) {
      return;
    }

    const siblings = Array.from(document.body.children).filter(
      (element): element is HTMLElement =>
        element instanceof HTMLElement && element !== dialog,
    );
    const previousSiblingAttributes = siblings.map((element) => ({
      ariaHidden: element.getAttribute("aria-hidden"),
      element,
      inert: element.hasAttribute("inert"),
    }));

    for (const sibling of siblings) {
      sibling.setAttribute("aria-hidden", "true");
      sibling.setAttribute("inert", "");
    }

    return () => {
      for (const { ariaHidden, element, inert } of previousSiblingAttributes) {
        if (ariaHidden == null) {
          element.removeAttribute("aria-hidden");
        } else {
          element.setAttribute("aria-hidden", ariaHidden);
        }

        if (!inert) {
          element.removeAttribute("inert");
        }
      }

      if (previouslyFocusedElementRef.current?.isConnected) {
        previouslyFocusedElementRef.current.focus({ preventScroll: true });
      }
    };
  }, []);

  React.useEffect(() => {
    const handleResize = () => setTargetBox(imageLightboxTargetBox(sourceBox));
    window.addEventListener("resize", handleResize);
    return () => window.removeEventListener("resize", handleResize);
  }, [sourceBox]);

  React.useEffect(() => {
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        close();
        return;
      }

      if (event.key !== "Tab") {
        return;
      }

      const dialog = dialogRef.current;
      if (!dialog) {
        return;
      }

      const focusableElements = getImageLightboxFocusableElements(dialog);
      if (focusableElements.length === 0) {
        event.preventDefault();
        dialog.focus();
        return;
      }

      const firstElement = focusableElements[0];
      const lastElement = focusableElements[focusableElements.length - 1];
      const activeElement = document.activeElement;

      if (activeElement === dialog) {
        event.preventDefault();
        if (event.shiftKey) {
          lastElement.focus();
        } else {
          firstElement.focus();
        }
        return;
      }

      if (!dialog.contains(activeElement)) {
        event.preventDefault();
        firstElement.focus();
        return;
      }

      if (event.shiftKey && activeElement === firstElement) {
        event.preventDefault();
        lastElement.focus();
        return;
      }

      if (!event.shiftKey && activeElement === lastElement) {
        event.preventDefault();
        firstElement.focus();
      }
    };
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [close]);

  React.useEffect(() => {
    const dialog = dialogRef.current;
    if (!dialog || phase !== "open") {
      return;
    }

    const handleWheel = (event: WheelEvent) => {
      event.preventDefault();
      event.stopPropagation();
      markControlGesture();
      setIsAdjustingZoom(true);

      const normalizedDelta = normalizedWheelDeltaY(event);
      const zoomDelta = Math.max(
        -IMAGE_LIGHTBOX_WHEEL_ZOOM_MAX_DELTA,
        Math.min(
          IMAGE_LIGHTBOX_WHEEL_ZOOM_MAX_DELTA,
          -normalizedDelta * IMAGE_LIGHTBOX_WHEEL_ZOOM_SPEED,
        ),
      );
      updateZoom((currentZoom) => currentZoom * (1 + zoomDelta));
      finishZoomGestureSoon();
    };

    const handleGestureStart = (event: Event) => {
      event.preventDefault();
      markControlGesture();
      setIsAdjustingZoom(true);
      gestureScaleRef.current = 1;
    };

    const handleGestureChange = (event: Event) => {
      event.preventDefault();
      markControlGesture();
      setIsAdjustingZoom(true);

      const gestureEvent = event as WebKitGestureLikeEvent;
      const nextGestureScale =
        typeof gestureEvent.scale === "number" && gestureEvent.scale > 0
          ? gestureEvent.scale
          : 1;
      const previousGestureScale = Math.max(0.01, gestureScaleRef.current);
      gestureScaleRef.current = nextGestureScale;
      updateZoom(
        (currentZoom) =>
          currentZoom * (nextGestureScale / previousGestureScale),
      );
      finishZoomGestureSoon();
    };

    const handleGestureEnd = (event: Event) => {
      event.preventDefault();
      markControlGesture();
      gestureScaleRef.current = 1;
      finishZoomGestureSoon();
    };

    dialog.addEventListener("wheel", handleWheel, { passive: false });
    dialog.addEventListener("gesturestart", handleGestureStart, {
      passive: false,
    });
    dialog.addEventListener("gesturechange", handleGestureChange, {
      passive: false,
    });
    dialog.addEventListener("gestureend", handleGestureEnd, {
      passive: false,
    });

    return () => {
      dialog.removeEventListener("wheel", handleWheel);
      dialog.removeEventListener("gesturestart", handleGestureStart);
      dialog.removeEventListener("gesturechange", handleGestureChange);
      dialog.removeEventListener("gestureend", handleGestureEnd);
    };
  }, [finishZoomGestureSoon, markControlGesture, phase, updateZoom]);

  React.useEffect(() => {
    return () => {
      if (fadeTimerRef.current != null) {
        window.clearTimeout(fadeTimerRef.current);
      }
      if (closeTimerRef.current != null) {
        window.clearTimeout(closeTimerRef.current);
      }
      if (zoomIdleTimerRef.current != null) {
        window.clearTimeout(zoomIdleTimerRef.current);
      }
    };
  }, []);

  const isClosing = phase === "closing";
  const isOpen = phase === "open";
  const isFading = phase === "fading";
  const displayBox = imageLightboxZoomBox(targetBox, zoom);
  // Once fully settled at 1x, drop the transform to `none` so the wrapper
  // leaves the GPU-composited path and the <img> repaints through WebKit's
  // high-quality paint rasterizer — matching inline-image sharpness. An
  // identity `translate3d` would keep it composited, so it must be `none`.
  const atRest =
    isOpen &&
    hasEntered &&
    zoom === IMAGE_LIGHTBOX_MIN_ZOOM &&
    // Holds composited through the trackpad gesture-end idle window: after a
    // pinch settles back to exactly 1x, `isAdjustingZoom` stays true for
    // IMAGE_LIGHTBOX_TRACKPAD_ZOOM_IDLE_MS, avoiding a demote/re-promote thrash.
    !isAdjustingZoom;
  const transform = atRest
    ? "none"
    : prefersReducedMotion || isOpen
      ? imageLightboxTransform(targetBox, displayBox)
      : imageLightboxTransform(targetBox, sourceBox);
  const imageTransitionDuration = prefersReducedMotion
    ? IMAGE_LIGHTBOX_REDUCED_MOTION_MS
    : isClosing
      ? IMAGE_LIGHTBOX_EXIT_MS
      : hasEntered
        ? isAdjustingZoom
          ? 0
          : IMAGE_LIGHTBOX_ZOOM_TRANSITION_MS
        : IMAGE_LIGHTBOX_ENTER_MS;
  const backgroundTransitionDuration = prefersReducedMotion
    ? IMAGE_LIGHTBOX_REDUCED_MOTION_MS
    : isFading
      ? IMAGE_LIGHTBOX_FADE_EXIT_MS
      : IMAGE_LIGHTBOX_FADE_ENTER_MS;
  const zoomFillPercent =
    ((zoom - IMAGE_LIGHTBOX_MIN_ZOOM) /
      (IMAGE_LIGHTBOX_MAX_ZOOM - IMAGE_LIGHTBOX_MIN_ZOOM)) *
    100;
  const label = alt?.trim() || "Image preview";
  const handleImageContextMenu = React.useCallback(
    (event: React.MouseEvent<HTMLImageElement>) => {
      event.preventDefault();
      event.stopPropagation();
      event.nativeEvent.stopImmediatePropagation();
      markControlGesture();
      if (canDownload) {
        setMenu({ x: event.clientX, y: event.clientY });
      }
    },
    [canDownload, markControlGesture],
  );
  const handleMenuDownload = React.useCallback(() => {
    setMenu(null);
    markControlGesture();
    onDownload();
  }, [markControlGesture, onDownload]);

  return createPortal(
    <div
      aria-describedby={descriptionId}
      aria-label={label}
      aria-modal="true"
      className="fixed inset-0 z-50 cursor-zoom-out outline-hidden"
      onClick={(event) => {
        if (Date.now() < suppressCloseUntilRef.current) {
          return;
        }
        if (
          event.target instanceof HTMLElement &&
          event.target.closest("[data-image-lightbox-controls]")
        ) {
          markControlGesture();
          return;
        }
        close();
      }}
      onKeyDown={(event) => {
        if (event.key === "Escape") {
          event.preventDefault();
          close();
        }
      }}
      onPointerCancelCapture={() => {
        if (controlPointerDownRef.current) {
          markControlGesture();
          controlPointerDownRef.current = false;
        }
      }}
      onPointerDownCapture={(event) => {
        if (
          event.target instanceof HTMLElement &&
          event.target.closest("[data-image-lightbox-controls]")
        ) {
          controlPointerDownRef.current = true;
          markControlGesture();
        }
      }}
      onPointerUpCapture={() => {
        if (controlPointerDownRef.current) {
          markControlGesture();
          controlPointerDownRef.current = false;
        }
      }}
      ref={dialogRef}
      role="dialog"
      tabIndex={-1}
    >
      <p className="sr-only" id={descriptionId}>
        Full-size image preview. Press Escape or click to close.
      </p>
      <div
        className={cn(
          "absolute inset-0 bg-[#08090a] transition-opacity",
          isOpen || isClosing ? "opacity-100" : "opacity-0",
        )}
        style={{
          transitionDuration: `${backgroundTransitionDuration}ms`,
          transitionTimingFunction: IMAGE_LIGHTBOX_EASE_OUT,
        }}
      />
      <div
        className={cn(
          "absolute z-10 origin-top-left overflow-visible transition-[opacity,transform]",
          // Only promote to a composited layer while animating; demoting at
          // rest is what restores high-quality rasterization.
          !atRest && "will-change-transform",
        )}
        style={{
          ...imageLightboxStyle(targetBox),
          opacity: prefersReducedMotion && isClosing ? 0 : 1,
          transform,
          transitionDuration: `${imageTransitionDuration}ms`,
          // At rest, exclude `transform` from the transition so the swap to
          // `none` is instantaneous — a transitioned transform-to-`none` is
          // browser-inconsistent and can flash at the settle.
          transitionProperty:
            prefersReducedMotion || atRest ? "opacity" : "opacity, transform",
          transitionTimingFunction: isClosing
            ? IMAGE_LIGHTBOX_EASE_IN_OUT
            : IMAGE_LIGHTBOX_EASE_OUT,
        }}
      >
        <img
          alt={alt}
          className="h-full w-full rounded-lg object-contain shadow-2xl"
          src={resolvedSrc}
          onContextMenuCapture={handleImageContextMenu}
        />
      </div>
      <div
        className={cn(
          "absolute inset-x-0 bottom-4 z-20 flex justify-center px-4 transition-[opacity,transform]",
          isOpen ? "translate-y-0 opacity-100" : "translate-y-1.5 opacity-0",
        )}
        style={{
          transitionDuration: `${prefersReducedMotion ? IMAGE_LIGHTBOX_REDUCED_MOTION_MS : 160}ms`,
          transitionTimingFunction: IMAGE_LIGHTBOX_EASE_OUT,
        }}
      >
        <div
          aria-label="Image controls"
          className="relative isolate flex min-h-11 max-w-[calc(100vw-2rem)] items-center gap-2 rounded-xl px-2 py-1.5 text-white"
          data-image-lightbox-controls=""
          role="toolbar"
        >
          <div
            aria-hidden="true"
            className="pointer-events-none absolute inset-0 -z-10 rounded-[inherit] border border-white/10 bg-black/35 backdrop-blur-xl backdrop-saturate-150"
          />
          <button
            aria-label="Download image"
            className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg text-white transition-colors hover:bg-white/15 outline-hidden focus-visible:ring-2 focus-visible:ring-white/60 disabled:pointer-events-none disabled:opacity-45"
            disabled={!canDownload}
            type="button"
            onClick={(event) => {
              event.stopPropagation();
              onDownload();
            }}
          >
            <Download className="h-4 w-4" />
          </button>
          <div aria-hidden="true" className="h-5 w-px shrink-0 bg-white/15" />
          <ZoomOut aria-hidden="true" className="h-4 w-4 shrink-0 opacity-80" />
          <input
            aria-label="Image zoom"
            className="image-zoom-slider h-3 w-32 cursor-pointer sm:w-44"
            max={IMAGE_LIGHTBOX_MAX_ZOOM}
            min={IMAGE_LIGHTBOX_MIN_ZOOM}
            step={IMAGE_LIGHTBOX_ZOOM_STEP}
            style={
              {
                "--image-zoom-fill": `${zoomFillPercent}%`,
              } as React.CSSProperties
            }
            type="range"
            value={zoom}
            onBlur={() => setIsAdjustingZoom(false)}
            onChange={(event) => {
              markControlGesture();
              setClampedZoom(Number(event.target.value));
            }}
            onPointerCancel={() => setIsAdjustingZoom(false)}
            onPointerDown={() => {
              markControlGesture();
              setIsAdjustingZoom(true);
            }}
            onPointerUp={() => {
              markControlGesture();
              setIsAdjustingZoom(false);
            }}
          />
          <ZoomIn aria-hidden="true" className="h-4 w-4 shrink-0 opacity-80" />
          <span className="min-w-10 text-right text-xs font-medium tabular-nums text-white/90">
            {Math.round(zoom * 100)}%
          </span>
        </div>
      </div>
      {menu && canDownload ? (
        <ImageDownloadContextMenu
          onDownload={handleMenuDownload}
          position={menu}
        />
      ) : null}
    </div>,
    document.body,
  );
}

/**
 * Inline image embed with click-to-zoom lightbox and right-click download.
 *
 * IMPORTANT: the trigger is a plain button that we control ourselves — not
 * Radix's `<Trigger asChild>` cloning onto a wrapper. An earlier version used
 * that pattern and caused a 1-2px layout reflow in the surrounding message
 * body on hover. Keeping the trigger stable and managing the lightbox via
 * React state avoids that repaint.
 */
function ImageBlock({
  alt,
  dim,
  resolvedSrc,
  src,
}: {
  alt: string | undefined;
  dim?: string;
  resolvedSrc: string | undefined;
  src: string | undefined;
}) {
  const [lightboxBox, setLightboxBox] = React.useState<ImageLightboxBox | null>(
    null,
  );
  const [isHiddenInSpoiler, setIsHiddenInSpoiler] = React.useState(false);
  const [menu, setMenu] = React.useState<ImageContextMenuPosition | null>(null);
  const inlineImageRef = React.useRef<HTMLImageElement | null>(null);
  const triggerRef = React.useRef<HTMLButtonElement | null>(null);
  const [spoilerMediaSize, setSpoilerMediaSize] = React.useState<{
    height: number;
    src: string;
    width: number;
  } | null>(null);

  const updateSpoilerMediaSize = React.useCallback(
    (image: HTMLImageElement) => {
      const { naturalHeight, naturalWidth } = image;
      if (naturalHeight <= 0 || naturalWidth <= 0) return;

      const maxWidth = 384;
      const maxHeight = 256;
      const scale = Math.min(
        1,
        maxWidth / naturalWidth,
        maxHeight / naturalHeight,
      );
      setSpoilerMediaSize({
        height: Math.max(1, Math.round(naturalHeight * scale)),
        src: resolvedSrc ?? image.currentSrc,
        width: Math.max(1, Math.round(naturalWidth * scale)),
      });
    },
    [resolvedSrc],
  );

  const imageRef = React.useCallback(
    (image: HTMLImageElement | null) => {
      inlineImageRef.current = image;
      if (image?.complete) updateSpoilerMediaSize(image);
    },
    [updateSpoilerMediaSize],
  );

  const currentSpoilerMediaSize =
    spoilerMediaSize?.src === resolvedSrc ? spoilerMediaSize : null;
  const hiddenSpoilerMediaSize = isHiddenInSpoiler
    ? currentSpoilerMediaSize
    : null;

  const spoilerMediaStyle = hiddenSpoilerMediaSize
    ? ({
        "--buzz-spoiler-media-height": `${hiddenSpoilerMediaSize.height}px`,
        "--buzz-spoiler-media-width": `${hiddenSpoilerMediaSize.width}px`,
        height: `${hiddenSpoilerMediaSize.height}px`,
        width: `${hiddenSpoilerMediaSize.width}px`,
      } as React.CSSProperties)
    : undefined;

  React.useLayoutEffect(() => {
    const trigger = triggerRef.current;
    if (!trigger) return;

    const updateHiddenState = () => {
      setIsHiddenInSpoiler(isInsideHiddenSpoiler(trigger));
    };

    updateHiddenState();

    const spoiler = trigger.closest(".buzz-spoiler[data-spoiler]");
    if (!spoiler) return;

    const observer = new MutationObserver(updateHiddenState);
    observer.observe(spoiler, {
      attributeFilter: ["data-revealed"],
      attributes: true,
    });

    return () => observer.disconnect();
  }, []);

  const closeMenu = React.useCallback(() => setMenu(null), []);
  useDismissImageContextMenu(Boolean(menu), closeMenu);

  // Intrinsic dimensions from the NIP-92 `dim` tag, stamped as width/height
  // attributes so the browser reserves aspect-ratio space before the image
  // decodes. Without this the row grows from ~0 on load and shoves the
  // timeline — the exact reflow the anchored-scroll restore then has to fight.
  const intrinsicDimensions = dimensionsFromDim(dim);

  const handleContextMenu = (e: React.MouseEvent) => {
    e.preventDefault();
    if (isInsideHiddenSpoiler(e.currentTarget)) return;
    e.stopPropagation();
    e.nativeEvent.stopImmediatePropagation();
    setMenu({ x: e.clientX, y: e.clientY });
  };

  const openLightbox = React.useCallback(
    (image: HTMLImageElement) => {
      if (!resolvedSrc || isInsideHiddenSpoiler(image)) {
        return;
      }

      const rect = image.getBoundingClientRect();
      if (rect.width <= 0 || rect.height <= 0) {
        return;
      }

      setMenu(null);
      setLightboxBox(imageLightboxBoxFromRect(rect));
    },
    [resolvedSrc],
  );

  const handleImageTriggerClick = () => {
    if (inlineImageRef.current) {
      openLightbox(inlineImageRef.current);
    }
  };

  const handleDownload = () => {
    setMenu(null);
    if (!src) return;
    invokeTauri("download_image", { url: src }).catch((err: unknown) => {
      const msg = err instanceof Error ? err.message : "Download failed";
      toast.error(msg);
    });
  };

  return (
    <>
      <button
        aria-hidden={isHiddenInSpoiler ? true : undefined}
        aria-label={alt?.trim() ? `Zoom image: ${alt}` : "Zoom image"}
        className={cn(
          "mt-1 inline-block max-w-full cursor-zoom-in rounded-xl border-0 bg-transparent p-0 text-left align-top focus:outline-hidden focus-visible:ring-2 focus-visible:ring-ring/50",
          lightboxBox && "opacity-0",
        )}
        data-testid="message-image-lightbox-trigger"
        ref={triggerRef}
        tabIndex={isHiddenInSpoiler ? -1 : undefined}
        type="button"
        onClick={handleImageTriggerClick}
        onContextMenuCapture={handleContextMenu}
      >
        <img
          alt={alt}
          className="block max-h-64 max-w-sm rounded-xl object-contain"
          data-spoiler-media-size={hiddenSpoilerMediaSize ? "" : undefined}
          height={intrinsicDimensions?.height}
          ref={imageRef}
          src={resolvedSrc}
          style={spoilerMediaStyle}
          width={intrinsicDimensions?.width}
          onLoad={(event) => updateSpoilerMediaSize(event.currentTarget)}
        />
      </button>
      {menu && src ? (
        <ImageDownloadContextMenu onDownload={handleDownload} position={menu} />
      ) : null}
      {lightboxBox && resolvedSrc ? (
        <ImageZoomOverlay
          alt={alt}
          canDownload={Boolean(src)}
          onDownload={handleDownload}
          onClose={() => setLightboxBox(null)}
          resolvedSrc={resolvedSrc}
          sourceBox={lightboxBox}
        />
      ) : null}
    </>
  );
}

function getReactNodeText(node: React.ReactNode): string {
  if (typeof node === "string" || typeof node === "number") {
    return String(node);
  }

  if (Array.isArray(node)) {
    return node.map(getReactNodeText).join("");
  }

  if (React.isValidElement<{ children?: React.ReactNode }>(node)) {
    return getReactNodeText(node.props.children);
  }

  return "";
}

function getCodeBlockText(children: React.ReactNode) {
  return getReactNodeText(children).replace(/\n$/, "");
}

function InlineEmojiPopover({
  alt,
  resolvedSrc,
}: {
  alt: string | undefined;
  resolvedSrc: string;
}) {
  const [open, setOpen] = React.useState(false);
  const openTimeout = React.useRef<ReturnType<typeof setTimeout> | null>(null);
  const closeTimeout = React.useRef<ReturnType<typeof setTimeout> | null>(null);
  const label = alt?.trim() || "Custom emoji";

  const clearTimers = React.useCallback(() => {
    if (openTimeout.current) {
      clearTimeout(openTimeout.current);
      openTimeout.current = null;
    }
    if (closeTimeout.current) {
      clearTimeout(closeTimeout.current);
      closeTimeout.current = null;
    }
  }, []);

  const handleMouseEnter = React.useCallback(() => {
    clearTimers();
    openTimeout.current = setTimeout(() => setOpen(true), 200);
  }, [clearTimers]);

  const scheduleClose = React.useCallback(() => {
    clearTimers();
    closeTimeout.current = setTimeout(() => setOpen(false), 150);
  }, [clearTimers]);

  const handleFocus = React.useCallback(() => {
    clearTimers();
    setOpen(true);
  }, [clearTimers]);

  React.useEffect(() => clearTimers, [clearTimers]);

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <button
          type="button"
          className="inline-flex border-0 bg-transparent p-0 align-middle text-inherit"
          aria-label={label}
          onMouseEnter={handleMouseEnter}
          onMouseLeave={scheduleClose}
          onFocus={handleFocus}
          onBlur={scheduleClose}
        >
          <img
            alt={alt}
            title={label}
            src={resolvedSrc}
            data-custom-emoji=""
            className="mx-px inline-block h-[1.25em] w-auto max-w-none align-middle"
            draggable={false}
            onContextMenu={(e) => e.preventDefault()}
          />
        </button>
      </PopoverTrigger>
      <PopoverContent
        align="center"
        side="top"
        sideOffset={6}
        className="w-auto min-w-32 max-w-56 rounded-xl p-3"
        onMouseEnter={handleMouseEnter}
        onMouseLeave={scheduleClose}
        onOpenAutoFocus={(e) => e.preventDefault()}
        onCloseAutoFocus={(e) => e.preventDefault()}
      >
        <div className="flex flex-col items-center text-center">
          <div className="mb-2 flex h-14 w-14 items-center justify-center">
            <img
              alt={alt}
              src={resolvedSrc}
              className="inline-block h-12 w-12 object-contain"
              draggable={false}
            />
          </div>
          <div className="max-w-[12rem] text-balance text-sm font-semibold leading-snug text-popover-foreground">
            {label}
          </div>
        </div>
      </PopoverContent>
    </Popover>
  );
}

function MarkdownCodeBlock({
  children,
  language,
}: {
  children?: React.ReactNode;
  language?: string;
}) {
  const [isCopying, setIsCopying] = React.useState(false);
  const code = React.useMemo(() => getCodeBlockText(children), [children]);

  const handleCopy = React.useCallback(
    async (event: React.MouseEvent<HTMLButtonElement>) => {
      event.preventDefault();
      event.stopPropagation();
      setIsCopying(true);

      try {
        await copyCodeBlockToClipboard(code);
        toast.success("Copied code to clipboard");
      } catch (error) {
        console.error("Failed to copy code block", error);
        toast.error("Failed to copy code");
      } finally {
        setIsCopying(false);
      }
    },
    [code],
  );

  return (
    <div className="group relative" data-code-block="">
      <pre className="max-h-[400px] overflow-x-auto overflow-y-auto rounded-xl border border-border/70 bg-muted/60 px-3 py-1.5 pr-12 shadow-xs">
        {language && (
          <div className="mb-1 text-xs text-muted-foreground/70">
            {language}
          </div>
        )}
        {children}
      </pre>
      <Tooltip>
        <TooltipTrigger asChild>
          <Button
            aria-label="Copy code block"
            className="absolute right-2 top-2 h-7 w-7 bg-background/80 text-muted-foreground opacity-0 shadow-xs ring-1 ring-border/60 backdrop-blur-sm transition-opacity hover:bg-background hover:text-foreground hover:opacity-100 focus-visible:opacity-100 group-hover:opacity-100 group-focus-within:opacity-100 disabled:opacity-60"
            disabled={isCopying}
            onClick={handleCopy}
            size="icon"
            type="button"
            variant="ghost"
          >
            <Copy className="h-4 w-4" />
            <span className="sr-only">Copy code block</span>
          </Button>
        </TooltipTrigger>
        <TooltipContent>Copy code</TooltipContent>
      </Tooltip>
    </div>
  );
}

/** Human-readable byte size: "820 B", "12.4 KB", "3.1 MB". */
function formatFileSize(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes < 0) return "";
  if (bytes < 1024) return `${bytes} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let size = bytes / 1024;
  let i = 0;
  while (size >= 1024 && i < units.length - 1) {
    size /= 1024;
    i += 1;
  }
  return `${size < 10 ? size.toFixed(1) : Math.round(size)} ${units[i]}`;
}

/**
 * File card for a generic (non-image, non-video) attachment: icon, filename,
 * size, and a download action.
 *
 * Downloads go through the native `download_file` Tauri command (HTTP inside
 * the app's tunnel + a save dialog), not a plain `<a download>` link. A bare
 * link navigates the webview to the blob URL, which escapes to the OS browser
 * and gets bounced to a corporate CDN interstitial ("browser not supported").
 * The native command mirrors the image-download path.
 */
function FileCard({
  href,
  filename,
  size,
}: {
  href: string;
  filename: string;
  size?: number;
}) {
  const sizeLabel = size != null ? formatFileSize(size) : "";
  return (
    <button
      type="button"
      onClick={() => {
        invokeTauri("download_file", { url: href, filename }).catch(
          (err: unknown) => {
            const msg = err instanceof Error ? err.message : "Download failed";
            toast.error(msg);
          },
        );
      }}
      data-testid="file-card"
      className="my-1 inline-flex max-w-sm items-center gap-3 rounded-xl border border-border/70 bg-muted/40 px-3 py-2 text-left no-underline transition-colors hover:bg-muted/70"
    >
      <span className="flex h-9 w-9 shrink-0 items-center justify-center rounded-lg bg-background text-muted-foreground">
        <FileText className="h-4 w-4" />
      </span>
      <span className="min-w-0 flex-1">
        <span className="block truncate text-sm font-medium text-foreground">
          {filename}
        </span>
        {sizeLabel ? (
          <span className="block text-xs text-muted-foreground">
            {sizeLabel}
          </span>
        ) : null}
      </span>
      <Download className="h-4 w-4 shrink-0 text-muted-foreground" />
    </button>
  );
}

function SyntaxHighlightedCode({
  code,
  language,
  ...props
}: {
  code: string;
  language: string;
} & React.ComponentProps<"code">) {
  const { themeName } = useTheme();
  const [loadedKey, setLoadedKey] = React.useState(0);

  React.useEffect(() => {
    let cancelled = false;
    async function loadAssets() {
      try {
        await ensureHighlighter();
        if (!shikiHighlighter || cancelled) return;
        let loaded = false;
        if (!loadedLangs.has(language)) {
          if (loadedLangs.size >= MAX_LOADED_LANGUAGES) return;
          try {
            await shikiHighlighter.loadLanguage(language as BundledLanguage);
            loadedLangs.add(language);
            loaded = true;
          } catch {
            return;
          }
        }
        if (!loadedThemes.has(themeName as string)) {
          try {
            await shikiHighlighter.loadTheme(themeName as BundledTheme);
            loadedThemes.add(themeName as string);
            loaded = true;
          } catch {
            return;
          }
        }
        if (loaded && !cancelled) setLoadedKey((k) => k + 1);
      } catch {
        /* ignore */
      }
    }
    if (!loadedLangs.has(language) || !loadedThemes.has(themeName as string)) {
      loadAssets();
    }
    return () => {
      cancelled = true;
    };
  }, [language, themeName]);

  // biome-ignore lint/correctness/useExhaustiveDependencies: loadedKey intentionally triggers re-memoization after async asset loading
  const tokens = React.useMemo(() => {
    if (
      !shikiHighlighter ||
      !loadedLangs.has(language) ||
      !loadedThemes.has(themeName as string)
    )
      return null;
    if ((code.match(/\n/g) || []).length > MAX_HIGHLIGHT_LINES) return null;
    const cacheKey = `${language}:${themeName}:${code}`;
    const cached = tokenCache.get(cacheKey);
    if (cached) return cached;
    try {
      const result = shikiHighlighter.codeToTokens(code, {
        lang: language as BundledLanguage,
        theme: themeName as BundledTheme,
      });
      if (tokenCache.size >= MAX_CACHE_ENTRIES) {
        const firstKey = tokenCache.keys().next().value;
        if (firstKey !== undefined) tokenCache.delete(firstKey);
      }
      tokenCache.set(cacheKey, result.tokens);
      return result.tokens;
    } catch {
      return null;
    }
  }, [code, language, themeName, loadedKey]);

  const codeClassName = CODE_BLOCK_CLASS;

  if (!tokens) {
    const lines = code.split("\n");
    return (
      <code {...props} className={codeClassName}>
        {lines.map((line, i) => (
          // biome-ignore lint/suspicious/noArrayIndexKey: lines are positional
          <span key={i} data-line="">
            {line}
          </span>
        ))}
      </code>
    );
  }

  return (
    <code {...props} className={codeClassName}>
      {tokens.map((line, lineIdx) => {
        const lineText = line.map((t) => t.content).join("");
        const isAdd = DIFF_ADD_RE.test(lineText);
        const isRemove = DIFF_REMOVE_RE.test(lineText);
        const diffClass = isAdd
          ? "code-line-diff-add"
          : isRemove
            ? "code-line-diff-remove"
            : undefined;

        const renderedTokens =
          isAdd || isRemove
            ? stripDiffMarker(line, isAdd ? DIFF_ADD_RE : DIFF_REMOVE_RE)
            : line;

        return (
          <span
            // biome-ignore lint/suspicious/noArrayIndexKey: tokens are positional and never reordered
            key={lineIdx}
            data-line=""
            className={diffClass}
          >
            {renderedTokens.map((token, tokenIdx) => (
              <span
                // biome-ignore lint/suspicious/noArrayIndexKey: tokens are positional and never reordered
                key={tokenIdx}
                style={token.color ? { color: token.color } : undefined}
              >
                {token.content}
              </span>
            ))}
          </span>
        );
      })}
    </code>
  );
}

function SpoilerInline({
  block = false,
  children,
  interactive = true,
}: {
  block?: boolean;
  children?: React.ReactNode;
  interactive?: boolean;
}) {
  const [revealed, setRevealed] = React.useState(false);
  const contentRef = React.useRef<HTMLElement | null>(null);
  const isBlock = block || hasBlockMedia(React.Children.toArray(children));

  const setContentElement = React.useCallback((node: HTMLElement | null) => {
    contentRef.current = node;
  }, []);

  const toggleRevealed = React.useCallback(() => {
    setRevealed((value) => !value);
  }, []);

  const handlePointerDownCapture = React.useCallback(
    (event: React.PointerEvent<HTMLElement>) => {
      if (revealed) return;
      event.stopPropagation();
    },
    [revealed],
  );

  const handleClickCapture = React.useCallback(
    (event: React.MouseEvent<HTMLElement>) => {
      if (revealed) return;
      event.preventDefault();
      event.stopPropagation();
      toggleRevealed();
    },
    [revealed, toggleRevealed],
  );

  const handleClick = React.useCallback(
    (event: React.MouseEvent<HTMLElement>) => {
      if (revealed && isBlock && event.target !== event.currentTarget) return;
      toggleRevealed();
    },
    [isBlock, revealed, toggleRevealed],
  );

  const handleKeyDown = React.useCallback(
    (event: React.KeyboardEvent<HTMLElement>) => {
      if (event.key !== "Enter" && event.key !== " ") return;
      event.preventDefault();
      toggleRevealed();
    },
    [toggleRevealed],
  );

  const revealProps = {
    "aria-label": revealed ? "Hide spoiler" : "Reveal spoiler",
    "aria-pressed": revealed,
    onClick: handleClick,
    onClickCapture: handleClickCapture,
    onKeyDown: handleKeyDown,
    onPointerDownCapture: handlePointerDownCapture,
    role: "button",
    tabIndex: 0,
  } as const;

  if (!interactive) {
    if (isBlock) {
      return (
        <div
          className="buzz-spoiler buzz-spoiler--block buzz-spoiler--inert"
          data-revealed="false"
          data-spoiler=""
        >
          <SpoilerParticles active contentRef={contentRef} />
          <div className="buzz-spoiler__content" ref={setContentElement}>
            {children}
          </div>
        </div>
      );
    }

    return (
      <span
        className="buzz-spoiler buzz-spoiler--inert"
        data-revealed="false"
        data-spoiler=""
      >
        <SpoilerParticles active contentRef={contentRef} />
        <span className="buzz-spoiler__content" ref={setContentElement}>
          {children}
        </span>
      </span>
    );
  }

  if (isBlock) {
    return (
      <div
        {...revealProps}
        className="buzz-spoiler buzz-spoiler--block"
        data-revealed={revealed ? "true" : "false"}
        data-spoiler=""
      >
        <SpoilerParticles active={!revealed} contentRef={contentRef} />
        <div className="buzz-spoiler__content" ref={setContentElement}>
          {children}
        </div>
      </div>
    );
  }

  return (
    <span
      {...revealProps}
      className="buzz-spoiler"
      data-revealed={revealed ? "true" : "false"}
      data-spoiler=""
    >
      <SpoilerParticles active={!revealed} contentRef={contentRef} />
      <span className="buzz-spoiler__content" ref={setContentElement}>
        {children}
      </span>
    </span>
  );
}

function createMarkdownComponents(
  runtimeRef: React.RefObject<MarkdownRuntime>,
  interactive = true,
): Components {
  const paragraphClassName = "leading-[inherit]";
  const listItemClassName = "my-1 [&_p]:inline";
  const listClassName = "space-y-1 pl-6 marker:text-muted-foreground";

  return {
    spoiler: ({
      children,
      ...props
    }: {
      "data-block-spoiler"?: string;
      children?: React.ReactNode;
    }) => (
      <SpoilerInline
        block={props["data-block-spoiler"] != null}
        interactive={interactive}
      >
        {children}
      </SpoilerInline>
    ),
    a: ({ children, href, ...props }) => {
      const { imetaByUrl, onOpenMessageLink } = runtimeRef.current;
      if (!interactive) {
        return <span className="font-medium text-current">{children}</span>;
      }

      // Markdown image-link syntax (`[![alt](src)](href)`) otherwise nests the
      // image lightbox button inside an anchor. Keep the image as the lightbox
      // trigger and suppress the parent link activation for block media.
      if (hasBlockMedia(React.Children.toArray(children))) {
        return <>{children}</>;
      }

      // Generic file attachment: a `[filename](url)` link whose href matches an
      // imeta entry with a non-image, non-video MIME. Render a download card
      // instead of a plain link. (Media uses the `img` renderer, not this path.)
      const card = resolveFileCard(
        href ? imetaByUrl?.get(href) : undefined,
        href,
        getReactNodeText(children),
      );
      if (card) {
        return (
          <FileCard
            href={card.href}
            filename={card.filename}
            size={card.size}
          />
        );
      }

      // Intercept `buzz://message?channel=…&id=…` links so a click navigates
      // in-app instead of opening the URL in the OS browser. http(s) links
      // continue to use the existing target="_blank" behavior.
      if (isMessageLink(href)) {
        const parsed = parseMessageLink(href ?? "");
        if (parsed.ok) {
          const target = parsed.value;
          return (
            <a
              {...props}
              className="font-medium text-primary underline underline-offset-4 transition-colors hover:text-primary/80 cursor-pointer"
              href={href}
              onClick={(event) => {
                event.preventDefault();
                onOpenMessageLink(target);
              }}
            >
              {children}
            </a>
          );
        }
        // Malformed message deep link — fall through to the default
        // anchor (renders as a normal external link).
      }
      return (
        <a
          {...props}
          className="font-medium text-primary underline underline-offset-4 transition-colors hover:text-primary/80"
          href={href}
          rel="noreferrer"
          target="_blank"
        >
          {children}
        </a>
      );
    },
    blockquote: ({ children }) => (
      <blockquote className="border-l-2 border-border pl-4 italic text-muted-foreground [&>*:first-child]:mt-0 [&>*+*]:mt-2">
        {children}
      </blockquote>
    ),
    br: () => <br />,
    code: ({ children, className, ...props }: React.ComponentProps<"code">) => {
      const rawCode = String(children);
      const code = rawCode.replace(/\n$/, "");
      const isFencedCodeBlock =
        typeof className === "string" && className.includes("language-");

      if (isFencedCodeBlock || rawCode.endsWith("\n") || code.includes("\n")) {
        const language = extractLanguage(className);

        if (language) {
          return (
            <SyntaxHighlightedCode code={code} language={language} {...props} />
          );
        }

        const lines = code.split("\n");
        return (
          <code {...props} className={CODE_BLOCK_CLASS}>
            {lines.map((line, i) => (
              // biome-ignore lint/suspicious/noArrayIndexKey: lines are positional
              <span key={i} data-line="">
                {line}
              </span>
            ))}
          </code>
        );
      }

      return (
        <code {...props} className={cn(INLINE_CODE_CHIP_CLASS, className)}>
          {children}
        </code>
      );
    },
    h1: ({ children }) => (
      <h1 className="text-xl font-semibold leading-8 tracking-tight">
        {children}
      </h1>
    ),
    h2: ({ children }) => (
      <h2 className="text-lg font-semibold leading-7 tracking-tight">
        {children}
      </h2>
    ),
    h3: ({ children }) => (
      <h3 className="text-base font-semibold leading-6 tracking-tight">
        {children}
      </h3>
    ),
    h4: ({ children }) => (
      <h4 className="text-sm font-semibold leading-5 tracking-tight">
        {children}
      </h4>
    ),
    h5: ({ children }) => (
      <h5 className="text-sm font-semibold leading-5 tracking-tight">
        {children}
      </h5>
    ),
    h6: ({ children }) => (
      <h6 className="text-sm font-medium leading-5 tracking-tight text-muted-foreground">
        {children}
      </h6>
    ),
    hr: () => <hr className="border-border/80" />,
    img: ({ alt, src }) => {
      const { imetaByUrl } = runtimeRef.current;
      const resolvedSrc = src ? rewriteRelayUrl(src) : src;
      if (!interactive) {
        const fallbackLabel = resolvedSrc?.endsWith(".mp4")
          ? "Video attachment"
          : "Image attachment";
        return <span>{alt?.trim() || fallbackLabel}</span>;
      }

      if (resolvedSrc?.endsWith(".mp4")) {
        const entry = src ? imetaByUrl?.get(src) : undefined;
        return (
          <span data-block-media="">
            <MarkdownVideoPlayer
              key={src ?? resolvedSrc}
              alt={alt}
              entry={entry}
              resolvedSrc={resolvedSrc}
              src={src}
            />
          </span>
        );
      }
      const entry = src ? imetaByUrl?.get(src) : undefined;
      return (
        <span data-block-media="" className="block">
          <ImageBlock
            alt={alt}
            dim={entry?.dim}
            resolvedSrc={resolvedSrc}
            src={src}
          />
        </span>
      );
    },
    li: ({ children }) => <li className={listItemClassName}>{children}</li>,
    ol: ({ children }) => (
      <ol className={cn("list-decimal", listClassName)}>{children}</ol>
    ),
    p: ({ children }) => {
      // Detect image-only paragraphs (images + <br> from remarkBreaks).
      // Multi-image: render as a 2-column grid gallery.
      // Single media: render as a plain <div> to avoid invalid <p><div> nesting
      // (the img component returns block-level wrappers for lightbox/video).
      const childArray = React.Children.toArray(children);
      const { imageChildren } = classifyChildren(childArray);

      if (isImageOnlyParagraph(childArray)) {
        return (
          <div className="mt-1 grid max-w-lg grid-cols-2 gap-1.5 [&_br]:hidden [&_[data-block-media]]:mt-0 [&_[data-block-media]]:max-w-none [&_img]:mt-0 [&_img]:w-full [&_img]:max-w-full">
            {imageChildren}
          </div>
        );
      }

      if (hasBlockMedia(childArray)) {
        return <div className={paragraphClassName}>{children}</div>;
      }

      return <p className={paragraphClassName}>{children}</p>;
    },
    pre: ({ children }) => {
      if (!interactive) return <span>{children}</span>;
      let language = "";
      React.Children.forEach(children, (child) => {
        if (
          React.isValidElement<Record<string, unknown>>(child) &&
          typeof child.props?.className === "string"
        ) {
          language = extractLanguage(child.props.className);
        }
      });
      return (
        <MarkdownCodeBlock language={language}>{children}</MarkdownCodeBlock>
      );
    },
    strong: ({ children }) => (
      <strong className="font-semibold">{children}</strong>
    ),
    table: ({ children }) => (
      <div
        className="overflow-x-auto rounded-2xl border border-border/70"
        data-table-block=""
      >
        <table className="w-full border-collapse text-left text-sm">
          {children}
        </table>
      </div>
    ),
    td: ({ children }) => (
      <td className="border-t border-border/70 px-3 py-2 align-top">
        {children}
      </td>
    ),
    th: ({ children }) => (
      <th className="bg-muted/60 px-3 py-2 font-semibold text-foreground">
        {children}
      </th>
    ),
    ul: ({ children }) => (
      <ul className={cn("list-disc", listClassName)}>{children}</ul>
    ),
    mention: ({ children }: { children?: React.ReactNode }) => {
      const { agentMentionPubkeysByName, mentionPubkeysByName } =
        runtimeRef.current;
      const mentionText = String(children ?? "");
      const mentionName = mentionText.replace(/^@/, "").trim().toLowerCase();
      const pubkey = mentionPubkeysByName?.[mentionName];
      const isAgentMention =
        pubkey !== undefined &&
        agentMentionPubkeysByName?.[mentionName] === pubkey;
      const mentionLabel = mentionText.replace(/^@/, "");
      const renderedMentionText = isAgentMention ? (
        mentionLabel
      ) : (
        <>
          <span className={MENTION_CHIP_PREFIX_CLASS}>@</span>
          {mentionLabel}
        </>
      );
      const mentionNode = (
        <span
          data-mention=""
          className={cn(
            "cursor-pointer",
            MENTION_CHIP_BASE_CLASSES,
            MENTION_CHIP_HOVER_CLASSES,
            isAgentMention && "agent-mention-highlight",
          )}
        >
          {renderedMentionText}
        </span>
      );

      if (!interactive) {
        return mentionNode;
      }

      return pubkey ? (
        <UserProfilePopover pubkey={pubkey} triggerElement="span">
          {mentionNode}
        </UserProfilePopover>
      ) : (
        mentionNode
      );
    },
    emoji: ({ src, alt }: { src?: string; alt?: string }) => {
      const resolvedSrc = src ? rewriteRelayUrl(src) : src;
      if (!resolvedSrc) {
        return <span>{alt}</span>;
      }
      if (!interactive) {
        return <span>{alt}</span>;
      }
      return <InlineEmojiPopover alt={alt} resolvedSrc={resolvedSrc} />;
    },
    "channel-link": ({ children }: { children?: React.ReactNode }) => {
      const { channels, onOpenChannel } = runtimeRef.current;
      const text = String(children ?? "");
      const channelName = text.startsWith("#") ? text.slice(1) : text;
      const channel = channels.find(
        (c) =>
          c.channelType !== "dm" &&
          c.name.toLowerCase() === channelName.toLowerCase(),
      );

      if (channel && interactive) {
        return (
          <button
            type="button"
            data-channel-link=""
            aria-label={`Open channel ${channelName}`}
            className={cn(
              "cursor-pointer",
              MENTION_CHIP_BASE_CLASSES,
              MENTION_CHIP_HOVER_CLASSES,
            )}
            onClick={() => {
              onOpenChannel(channel.id);
            }}
          >
            {children}
          </button>
        );
      }

      return (
        <span data-channel-link="" className={MENTION_CHIP_BASE_CLASSES}>
          {children}
        </span>
      );
    },
    "message-link": ({ children }: { children?: React.ReactNode }) => {
      const { channels, onOpenMessageLink } = runtimeRef.current;
      const href = String(children ?? "");
      const parsed = parseMessageLink(href);
      if (!parsed.ok) {
        // Malformed `buzz://message?…` — render the raw URL as plain text
        // rather than a misleading clickable pill.
        return <span data-message-link="">{href}</span>;
      }

      const { channelId, messageId } = parsed.value;
      const channel = channels.find((c) => c.id === channelId);
      const channelLabel = channel?.name ?? "channel";
      const shortId = messageId.slice(0, 6);

      if (!interactive) {
        return (
          <span data-message-link="">
            #{channelLabel} · {shortId}
          </span>
        );
      }

      return (
        <button
          type="button"
          data-message-link=""
          aria-label={`Open message in ${channelLabel}`}
          title={href}
          className={cn(
            "cursor-pointer",
            MENTION_CHIP_BASE_CLASSES,
            MENTION_CHIP_HOVER_CLASSES,
          )}
          onClick={() => {
            onOpenMessageLink(parsed.value);
          }}
        >
          #{channelLabel} · {shortId}
        </button>
      );
    },
  } as Components;
}

function MarkdownInner({
  channelNames,
  className,
  content,
  customEmoji,
  imetaByUrl,
  interactive = true,
  agentMentionPubkeysByName,
  mentionNames,
  mentionPubkeysByName,
  searchQuery,
  videoReviewContext,
}: MarkdownProps) {
  const { channels: rawChannels } = useChannelNavigation();
  const channels = useStableArray(rawChannels);
  const { goChannel } = useAppNavigation();
  const onOpenChannel = React.useCallback(
    (channelId: string) => {
      void goChannel(channelId);
    },
    [goChannel],
  );
  const onOpenMessageLink = React.useCallback(
    (link: ParsedMessageLink) => {
      // Always route through `goChannel` with `messageId` set: the channel
      // route already handles scroll-into-view + highlight via
      // `useAnchoredScroll` + `getEventById` backfill, and works for
      // both stream-message replies and forum threads. Detecting "the thread
      // root is a forum post" up front would require an event lookup we don't
      // currently have synchronously; the brief explicitly allows skipping
      // that detection and falling through.
      void goChannel(link.channelId, {
        messageId: link.messageId,
        threadRootId: link.threadRootId,
      });
    },
    [goChannel],
  );
  const runtimeRef = useLatestRef<MarkdownRuntime>({
    agentMentionPubkeysByName,
    channels,
    imetaByUrl,
    mentionPubkeysByName,
    onOpenChannel,
    onOpenMessageLink,
  });

  const components = React.useMemo(
    () => createMarkdownComponents(runtimeRef, interactive),
    [runtimeRef, interactive],
  );

  // biome-ignore lint/suspicious/noExplicitAny: PluggableList type not directly importable
  const remarkPlugins = React.useMemo<any[]>(
    () => [
      remarkGfm,
      remarkBreaks,
      remarkSpoilers,
      remarkMessageLinks,
      [remarkMentions, { mentionNames }],
      [remarkChannelLinks, { channelNames }],
      [remarkCustomEmoji, { customEmoji }],
    ],
    [mentionNames, channelNames, customEmoji],
  );

  // biome-ignore lint/suspicious/noExplicitAny: PluggableList type not directly importable
  const rehypePlugins = React.useMemo<any[]>(() => {
    // biome-ignore lint/suspicious/noExplicitAny: PluggableList type not directly importable
    const plugins: any[] = [rehypeImageGallery];
    if (searchQuery && searchQuery.trim().length >= 2) {
      plugins.push([rehypeSearchHighlight, { query: searchQuery }]);
    }
    return plugins;
  }, [searchQuery]);

  let processedContent = content;

  if (/^(?:\s{2}\n)+/.test(content)) {
    processedContent = `\u200B${processedContent}`;
  }

  if (/(?:\s{2}\n)+$/.test(content)) {
    processedContent = `${processedContent}\u200B`;
  }

  const markdownNode = (
    <ReactMarkdown
      components={components}
      remarkPlugins={remarkPlugins}
      rehypePlugins={rehypePlugins}
      urlTransform={messageLinkUrlTransform}
    >
      {processedContent}
    </ReactMarkdown>
  );

  return (
    <div
      className={cn(
        MESSAGE_MARKDOWN_CLASS,
        [
          "max-w-none [overflow-wrap:anywhere] text-sm leading-5 text-foreground",
          "[&>*:first-child]:mt-0 [&>*:last-child]:mb-0",
          "[&>*+*]:mt-3",
          "[&>p+p]:mt-1.5",
          "[&>*+h1]:mt-3.5 [&>*+h2]:mt-3.5 [&>*+h3]:mt-3.5 [&>*+h4]:mt-3.5 [&>*+h5]:mt-3.5 [&>*+h6]:mt-3.5",
          "[&>h1+*]:mt-0.5 [&>h2+*]:mt-0.5 [&>h3+*]:mt-0.5 [&>h4+*]:mt-0.5 [&>h5+*]:mt-0.5 [&>h6+*]:mt-0.5",
          "[&>h1+h2]:mt-1.5! [&>h2+h3]:mt-1.5! [&>h3+h4]:mt-1.5! [&>h4+h5]:mt-1.5! [&>h5+h6]:mt-1.5!",
          "[&>*+blockquote]:mt-3.5 [&>blockquote+*]:mt-3.5",
          "[&>*+[data-code-block]]:mt-3.5 [&>[data-code-block]+*]:mt-3.5",
          "[&>*+[data-table-block]]:mt-3.5 [&>[data-table-block]+*]:mt-3.5",
          "[&>*+hr]:mt-4 [&>hr+*]:mt-4",
          "[&>p+ul]:mt-1.5 [&>p+ol]:mt-1.5 [&>div+ul]:mt-1.5 [&>div+ol]:mt-1.5",
        ].join(" "),
        className,
      )}
    >
      <VideoReviewMarkdownContext.Provider value={videoReviewContext}>
        {markdownNode}
      </VideoReviewMarkdownContext.Provider>
    </div>
  );
}

export const Markdown = React.memo(
  MarkdownInner,
  (prev, next) =>
    prev.content === next.content &&
    prev.className === next.className &&
    prev.customEmoji === next.customEmoji &&
    prev.interactive === next.interactive &&
    prev.agentMentionPubkeysByName === next.agentMentionPubkeysByName &&
    prev.mentionPubkeysByName === next.mentionPubkeysByName &&
    shallowArrayEqual(prev.mentionNames, next.mentionNames) &&
    shallowArrayEqual(prev.channelNames, next.channelNames) &&
    prev.imetaByUrl === next.imetaByUrl &&
    prev.searchQuery === next.searchQuery &&
    prev.videoReviewContext === next.videoReviewContext,
);

Markdown.displayName = "Markdown";
