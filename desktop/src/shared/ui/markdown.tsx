import * as React from "react";
import { createPortal } from "react-dom";
import ReactMarkdown, { type Components } from "react-markdown";
import {
  ChevronLeft,
  ChevronRight,
  Download,
  ZoomIn,
  ZoomOut,
} from "lucide-react";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import remarkBreaks from "remark-breaks";
import remarkGfm from "remark-gfm";
import { toast } from "sonner";

import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import {
  parseMessageLink,
  resolveMessageLinkRenderTarget,
  type ParsedMessageLink,
} from "@/features/messages/lib/messageLink";
import { UserProfilePopover } from "@/features/profile/ui/UserProfilePopover";
import { invokeTauri } from "@/shared/api/tauri";
import { useChannelNavigation } from "@/shared/context/ChannelNavigationContext";
import { cn } from "@/shared/lib/cn";
import {
  extractSupportedLinkPreviews,
  parseSupportedLinkPreview,
} from "@/shared/lib/linkPreview";
import { useResolvedLinkPreviews } from "@/shared/lib/useResolvedLinkPreviews";
import { rewriteRelayUrl } from "@/shared/lib/mediaUrl";
import rehypeImageGallery from "@/shared/lib/rehypeImageGallery";
import rehypeSearchHighlight from "@/shared/lib/rehypeSearchHighlight";
import remarkChannelLinks from "@/shared/lib/remarkChannelLinks";
import remarkCustomEmoji from "@/shared/lib/remarkCustomEmoji";
import remarkMentions from "@/shared/lib/remarkMentions";
import remarkSpoilers from "@/shared/lib/remarkSpoilers";
import remarkMessageLinks from "@/features/messages/lib/remarkMessageLinks";
import { AttachmentGroup } from "@/shared/ui/attachment";
import { LinkPreviewAttachment } from "@/shared/ui/link-preview-attachment";
import {
  INLINE_CODE_CHIP_CLASS,
  MENTION_CHIP_BASE_CLASSES,
  MENTION_CHIP_HOVER_CLASSES,
  MENTION_CHIP_PREFIX_CLASS,
  MESSAGE_MARKDOWN_CLASS,
} from "@/shared/ui/mentionChip";
import {
  POPOVER_CUSTOM_ENTER_MOTION_CLASS,
  POPOVER_SHADOW_STYLE,
  POPOVER_SURFACE_CLASS,
} from "@/shared/ui/popoverSurface";

import {
  classifyChildren,
  hasBlockMedia,
  isImageOnlyParagraph,
  shallowArrayEqual,
} from "./markdownUtils";
import {
  CODE_BLOCK_CLASS,
  extractLanguage,
  MarkdownCodeBlock,
  SyntaxHighlightedCode,
} from "./markdown/CodeBlock";
import { FileCard } from "./markdown/FileCard";
import { InlineEmojiPopover } from "./markdown/InlineEmojiPopover";
import { MarkdownInput } from "./markdown/MarkdownInput";
import { MessageLinkPill } from "./markdown/MessageLinkPill";
import { resolveFileCard } from "./markdownFileCard";
import type {
  ImetaEntry,
  MarkdownProps,
  MarkdownRuntime,
  MarkdownVariant,
} from "./markdown/types";
import { SpoilerInline } from "./markdown/SpoilerInline";
import {
  aspectRatioFromDim,
  dimensionsFromDim,
  getDecodedImageDimensions,
  imageReserveStyle,
  isInsideHiddenSpoiler,
  getReactNodeText,
  messageLinkUrlTransform,
  rememberDecodedImageDimensions,
  useFrozenImageReserve,
  useStableArray,
} from "./markdown/utils";
import { VideoPlayer, type VideoReviewContext } from "./VideoPlayer";

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

type ImageLightboxBox = {
  height: number;
  left: number;
  top: number;
  width: number;
};

type ImageGalleryDirection = "forward" | "backward";

type ImageGalleryItem = {
  alt: string | undefined;
  dim?: string;
  resolvedSrc: string;
  src: string | undefined;
  thumbnailBox?: ImageLightboxBox;
};

type ImageBlockProps = {
  alt: string | undefined;
  dim?: string;
  resolvedSrc: string | undefined;
  src: string | undefined;
};

const IMAGE_LIGHTBOX_ENTER_MS = 260;
const IMAGE_LIGHTBOX_EXIT_MS = 170;
const IMAGE_LIGHTBOX_FADE_ENTER_MS = 180;
const IMAGE_LIGHTBOX_FADE_EXIT_MS = 90;
const IMAGE_LIGHTBOX_GALLERY_SLIDE_MS = 280;
const IMAGE_LIGHTBOX_GALLERY_SLIDE_DISTANCE_PX = 48;
const IMAGE_LIGHTBOX_GALLERY_BLUR_PX = 4;
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
const IMAGE_LIGHTBOX_GALLERY_EASE: [number, number, number, number] = [
  0.22, 1, 0.36, 1,
];
const IMAGE_LIGHTBOX_MARKDOWN_SCOPE_SELECTOR = `.${MESSAGE_MARKDOWN_CLASS}`;

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

function imageLightboxBasisBoxForItem(
  item: ImageGalleryItem,
  fallbackBox: ImageLightboxBox,
): ImageLightboxBox {
  const dimensions =
    dimensionsFromDim(item.dim) ?? getDecodedImageDimensions(item.resolvedSrc);
  if (!dimensions) {
    return item.thumbnailBox ?? fallbackBox;
  }

  return {
    ...fallbackBox,
    height: dimensions.height,
    width: dimensions.width,
  };
}

function imageLightboxThumbnailBoxForItem(
  item: ImageGalleryItem,
  sourceScope: Element | null | undefined,
): ImageLightboxBox | null {
  const root = sourceScope?.isConnected ? sourceScope : document.body;
  const triggers = Array.from(
    root.querySelectorAll<HTMLElement>("[data-image-lightbox-trigger]"),
  );

  for (const trigger of triggers) {
    const isCurrentItem =
      trigger.dataset.imageLightboxResolvedSrc === item.resolvedSrc ||
      (item.src != null && trigger.dataset.imageLightboxSrc === item.src);
    if (!isCurrentItem) {
      continue;
    }

    const image = trigger.querySelector("img");
    const rect = (image ?? trigger).getBoundingClientRect();
    if (rect.width > 0 && rect.height > 0) {
      return imageLightboxBoxFromRect(rect);
    }
  }

  return null;
}

function imageLightboxReturnBoxForItem(
  item: ImageGalleryItem,
  fallbackBox: ImageLightboxBox,
  sourceScope: Element | null | undefined,
): ImageLightboxBox {
  return (
    imageLightboxThumbnailBoxForItem(item, sourceScope) ??
    item.thumbnailBox ??
    fallbackBox
  );
}

function imageLightboxSourceScopeForTrigger(
  trigger: HTMLElement,
): Element | null {
  return (
    trigger.closest(IMAGE_LIGHTBOX_MARKDOWN_SCOPE_SELECTOR) ??
    trigger.closest("[data-testid='message-row']")
  );
}

function imageGalleryItemFromTrigger(
  trigger: HTMLElement,
  thumbnailBox?: ImageLightboxBox,
): ImageGalleryItem | null {
  const resolvedSrc = trigger.dataset.imageLightboxResolvedSrc;
  if (!resolvedSrc) {
    return null;
  }

  return {
    alt: trigger.dataset.imageLightboxAlt || undefined,
    dim: trigger.dataset.imageLightboxDim || undefined,
    resolvedSrc,
    src: trigger.dataset.imageLightboxSrc || undefined,
    thumbnailBox,
  };
}

function isVisibleImageLightboxTrigger(trigger: HTMLElement): boolean {
  if (isInsideHiddenSpoiler(trigger)) {
    return false;
  }

  const image = trigger.querySelector("img");
  for (const element of [trigger, image]) {
    if (!element) {
      continue;
    }

    const style = window.getComputedStyle(element);
    if (
      style.display === "none" ||
      style.visibility === "hidden" ||
      Number(style.opacity) === 0
    ) {
      return false;
    }
  }

  return true;
}

function visibleImageGalleryForTrigger(
  trigger: HTMLElement,
  fallbackItem: ImageGalleryItem,
  sourceScope: Element | null | undefined,
): { galleryIndex: number; galleryItems?: ImageGalleryItem[] } {
  const root = sourceScope?.isConnected ? sourceScope : null;
  const triggers = root
    ? Array.from(
        root.querySelectorAll<HTMLElement>("[data-image-lightbox-trigger]"),
      )
    : [trigger];
  const galleryItems: ImageGalleryItem[] = [];
  let galleryIndex = 0;
  let foundCurrentTrigger = false;

  for (const candidate of triggers) {
    if (!isVisibleImageLightboxTrigger(candidate)) {
      continue;
    }

    const image = candidate.querySelector("img");
    const rect = (image ?? candidate).getBoundingClientRect();
    if (rect.width <= 0 || rect.height <= 0) {
      continue;
    }

    const thumbnailBox = imageLightboxBoxFromRect(rect);
    const item = imageGalleryItemFromTrigger(candidate, thumbnailBox);
    if (!item) {
      continue;
    }

    if (candidate === trigger) {
      galleryIndex = galleryItems.length;
      foundCurrentTrigger = true;
    }
    galleryItems.push(item);
  }

  if (!foundCurrentTrigger) {
    galleryItems.unshift(fallbackItem);
    galleryIndex = 0;
  }

  return {
    galleryIndex,
    galleryItems: galleryItems.length > 1 ? galleryItems : undefined,
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
  galleryIndex = 0,
  galleryItems,
  onDownload,
  onClose,
  resolvedSrc,
  sourceBox,
  sourceScope,
  src,
}: {
  alt: string | undefined;
  galleryIndex?: number;
  galleryItems?: ImageGalleryItem[];
  onDownload: (src: string | undefined) => void;
  onClose: () => void;
  resolvedSrc: string;
  sourceBox: ImageLightboxBox;
  sourceScope?: Element | null;
  src: string | undefined;
}) {
  const shouldReduceMotion = useReducedMotion();
  const prefersReducedMotion = shouldReduceMotion === true;
  const fallbackGalleryItems = React.useMemo<ImageGalleryItem[]>(
    () => [{ alt, resolvedSrc, src, thumbnailBox: sourceBox }],
    [alt, resolvedSrc, sourceBox, src],
  );
  const items =
    galleryItems && galleryItems.length > 0
      ? galleryItems
      : fallbackGalleryItems;
  const safeInitialIndex =
    galleryIndex >= 0 && galleryIndex < items.length ? galleryIndex : 0;
  const [currentIndex, setCurrentIndex] = React.useState(safeInitialIndex);
  const [galleryDirection, setGalleryDirection] =
    React.useState<ImageGalleryDirection>("forward");
  const [phase, setPhase] = React.useState<
    "opening" | "open" | "closing" | "fading"
  >(() => (prefersReducedMotion ? "open" : "opening"));
  const [hasEntered, setHasEntered] = React.useState(prefersReducedMotion);
  const [isAdjustingZoom, setIsAdjustingZoom] = React.useState(false);
  const [isGalleryNavigating, setIsGalleryNavigating] = React.useState(false);
  const [menu, setMenu] = React.useState<ImageContextMenuPosition | null>(null);
  const currentItem = items[currentIndex] ?? items[0];
  const basisBox = React.useMemo(
    () => imageLightboxBasisBoxForItem(currentItem, sourceBox),
    [currentItem, sourceBox],
  );
  const [targetBox, setTargetBox] = React.useState(() =>
    imageLightboxTargetBox(basisBox),
  );
  const [returnBox, setReturnBox] = React.useState(sourceBox);
  const [zoom, setZoom] = React.useState(IMAGE_LIGHTBOX_MIN_ZOOM);
  const controlPointerDownRef = React.useRef(false);
  const fadeTimerRef = React.useRef<number | null>(null);
  const galleryTransitionTimerRef = React.useRef<number | null>(null);
  const closeTimerRef = React.useRef<number | null>(null);
  const dialogRef = React.useRef<HTMLDivElement | null>(null);
  const descriptionId = React.useId();
  const gestureScaleRef = React.useRef(1);
  const previouslyFocusedElementRef = React.useRef<HTMLElement | null>(null);
  const suppressCloseUntilRef = React.useRef(0);
  const zoomIdleTimerRef = React.useRef<number | null>(null);
  const hasPreviousImage = currentIndex > 0;
  const hasNextImage = currentIndex < items.length - 1;
  const canDownloadCurrentImage = Boolean(currentItem.src);
  const galleryTransitionFilter =
    !prefersReducedMotion && isGalleryNavigating
      ? `blur(${IMAGE_LIGHTBOX_GALLERY_BLUR_PX}px)`
      : "blur(0px)";
  const galleryImageVariants = React.useMemo(
    () => ({
      center: { filter: "blur(0px)", opacity: 1, x: 0 },
      enter: (direction: ImageGalleryDirection) => ({
        filter: galleryTransitionFilter,
        opacity: 0,
        x: prefersReducedMotion
          ? 0
          : direction === "forward"
            ? IMAGE_LIGHTBOX_GALLERY_SLIDE_DISTANCE_PX
            : -IMAGE_LIGHTBOX_GALLERY_SLIDE_DISTANCE_PX,
      }),
      exit: (direction: ImageGalleryDirection) => ({
        filter: galleryTransitionFilter,
        opacity: 0,
        x: prefersReducedMotion
          ? 0
          : direction === "forward"
            ? -IMAGE_LIGHTBOX_GALLERY_SLIDE_DISTANCE_PX
            : IMAGE_LIGHTBOX_GALLERY_SLIDE_DISTANCE_PX,
      }),
    }),
    [galleryTransitionFilter, prefersReducedMotion],
  );

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

    if (galleryTransitionTimerRef.current != null) {
      window.clearTimeout(galleryTransitionTimerRef.current);
      galleryTransitionTimerRef.current = null;
    }
    setIsGalleryNavigating(false);
    setReturnBox(
      imageLightboxReturnBoxForItem(currentItem, sourceBox, sourceScope),
    );

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
  }, [currentItem, onClose, prefersReducedMotion, sourceBox, sourceScope]);

  const navigateGallery = React.useCallback(
    (nextIndex: number) => {
      if (
        nextIndex < 0 ||
        nextIndex >= items.length ||
        nextIndex === currentIndex
      ) {
        return;
      }

      markControlGesture();
      setMenu(null);
      setGalleryDirection(nextIndex > currentIndex ? "forward" : "backward");
      if (galleryTransitionTimerRef.current != null) {
        window.clearTimeout(galleryTransitionTimerRef.current);
      }
      setIsGalleryNavigating(!prefersReducedMotion);
      galleryTransitionTimerRef.current = window.setTimeout(() => {
        setIsGalleryNavigating(false);
        galleryTransitionTimerRef.current = null;
      }, IMAGE_LIGHTBOX_GALLERY_SLIDE_MS);
      setIsAdjustingZoom(false);
      setZoom(IMAGE_LIGHTBOX_MIN_ZOOM);
      setCurrentIndex(nextIndex);
    },
    [currentIndex, items.length, markControlGesture, prefersReducedMotion],
  );

  const goToPreviousImage = React.useCallback(() => {
    navigateGallery(currentIndex - 1);
  }, [currentIndex, navigateGallery]);

  const goToNextImage = React.useCallback(() => {
    navigateGallery(currentIndex + 1);
  }, [currentIndex, navigateGallery]);

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
    const handleResize = () => setTargetBox(imageLightboxTargetBox(basisBox));
    window.addEventListener("resize", handleResize);
    return () => window.removeEventListener("resize", handleResize);
  }, [basisBox]);

  React.useEffect(() => {
    setTargetBox(imageLightboxTargetBox(basisBox));
  }, [basisBox]);

  React.useEffect(() => {
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        close();
        return;
      }

      const target = event.target;
      const isRangeInput =
        target instanceof HTMLInputElement && target.type === "range";
      if (!isRangeInput && event.key === "ArrowLeft" && hasPreviousImage) {
        event.preventDefault();
        goToPreviousImage();
        return;
      }
      if (!isRangeInput && event.key === "ArrowRight" && hasNextImage) {
        event.preventDefault();
        goToNextImage();
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
  }, [close, goToNextImage, goToPreviousImage, hasNextImage, hasPreviousImage]);

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
      if (galleryTransitionTimerRef.current != null) {
        window.clearTimeout(galleryTransitionTimerRef.current);
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
  const isReturning = isClosing || isFading;
  const displayBox = imageLightboxZoomBox(targetBox, zoom);
  const frameBox = isReturning ? returnBox : targetBox;
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
    : isReturning
      ? "none"
      : prefersReducedMotion || isOpen
        ? imageLightboxTransform(targetBox, displayBox)
        : imageLightboxTransform(targetBox, sourceBox);
  const imageTransitionProperty = prefersReducedMotion
    ? "opacity"
    : isReturning
      ? "height, left, opacity, top, transform, width"
      : atRest
        ? "opacity"
        : "opacity, transform";
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
  const label = currentItem.alt?.trim() || "Image preview";
  const handleImageContextMenu = React.useCallback(
    (event: React.MouseEvent<HTMLImageElement>) => {
      event.preventDefault();
      event.stopPropagation();
      event.nativeEvent.stopImmediatePropagation();
      markControlGesture();
      if (canDownloadCurrentImage) {
        setMenu({ x: event.clientX, y: event.clientY });
      }
    },
    [canDownloadCurrentImage, markControlGesture],
  );
  const handleMenuDownload = React.useCallback(() => {
    setMenu(null);
    markControlGesture();
    onDownload(currentItem.src);
  }, [currentItem.src, markControlGesture, onDownload]);

  return createPortal(
    <div
      aria-describedby={descriptionId}
      aria-label={label}
      aria-modal="true"
      className="dark video-review-theme fixed inset-0 z-50 cursor-zoom-out outline-hidden"
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
        data-image-lightbox-frame=""
        className={cn(
          "absolute z-10 origin-top-left overflow-visible transition-[opacity,transform]",
          // Only promote to a composited layer while animating; demoting at
          // rest is what restores high-quality rasterization.
          !atRest && "will-change-transform",
        )}
        style={{
          ...imageLightboxStyle(frameBox),
          opacity: prefersReducedMotion && isReturning ? 0 : 1,
          transform,
          transitionDuration: `${imageTransitionDuration}ms`,
          // At rest, exclude `transform` from the transition so the swap to
          // `none` is instantaneous. On close, animate the frame box instead
          // of non-uniformly scaling the image back into the thumbnail.
          transitionProperty: imageTransitionProperty,
          transitionTimingFunction: isClosing
            ? IMAGE_LIGHTBOX_EASE_IN_OUT
            : IMAGE_LIGHTBOX_EASE_OUT,
        }}
      >
        <div className="relative h-full w-full overflow-hidden rounded-lg shadow-2xl">
          <AnimatePresence
            custom={galleryDirection}
            initial={false}
            mode="popLayout"
          >
            <motion.img
              alt={currentItem.alt}
              animate="center"
              className="absolute inset-0 h-full w-full object-contain"
              custom={galleryDirection}
              exit="exit"
              initial="enter"
              key={currentItem.resolvedSrc}
              src={currentItem.resolvedSrc}
              transition={{
                duration: prefersReducedMotion
                  ? IMAGE_LIGHTBOX_REDUCED_MOTION_MS / 1000
                  : IMAGE_LIGHTBOX_GALLERY_SLIDE_MS / 1000,
                ease: IMAGE_LIGHTBOX_GALLERY_EASE,
              }}
              variants={galleryImageVariants}
              onContextMenuCapture={handleImageContextMenu}
            />
          </AnimatePresence>
        </div>
      </div>
      {hasPreviousImage ? (
        <button
          aria-label="Previous image"
          className={cn(
            "absolute left-3 top-1/2 z-20 flex h-11 w-11 -translate-y-1/2 items-center justify-center rounded-full bg-muted text-muted-foreground shadow-sm backdrop-blur-xl backdrop-saturate-150 transition-[background-color,color,opacity] duration-150 hover:text-foreground focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring/70 sm:left-6",
            isOpen ? "opacity-100" : "pointer-events-none opacity-0",
          )}
          data-image-lightbox-controls=""
          type="button"
          onClick={(event) => {
            event.stopPropagation();
            goToPreviousImage();
          }}
        >
          <ChevronLeft className="h-6 w-6 -translate-x-[0.5px]" />
        </button>
      ) : null}
      {hasNextImage ? (
        <button
          aria-label="Next image"
          className={cn(
            "absolute right-3 top-1/2 z-20 flex h-11 w-11 -translate-y-1/2 items-center justify-center rounded-full bg-muted text-muted-foreground shadow-sm backdrop-blur-xl backdrop-saturate-150 transition-[background-color,color,opacity] duration-150 hover:text-foreground focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring/70 sm:right-6",
            isOpen ? "opacity-100" : "pointer-events-none opacity-0",
          )}
          data-image-lightbox-controls=""
          type="button"
          onClick={(event) => {
            event.stopPropagation();
            goToNextImage();
          }}
        >
          <ChevronRight className="h-6 w-6 translate-x-[0.5px]" />
        </button>
      ) : null}
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
          className="relative isolate flex min-h-11 max-w-[calc(100vw-2rem)] items-center gap-2 rounded-xl px-2 py-1.5 text-muted-foreground"
          data-image-lightbox-controls=""
          role="toolbar"
        >
          <div
            aria-hidden="true"
            className="pointer-events-none absolute inset-0 -z-10 rounded-[inherit] bg-muted shadow-sm backdrop-blur-xl backdrop-saturate-150"
          />
          <button
            aria-label="Download image"
            className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg transition-colors hover:bg-muted-foreground/10 hover:text-foreground outline-hidden focus-visible:ring-2 focus-visible:ring-ring/70 disabled:pointer-events-none disabled:opacity-45"
            disabled={!canDownloadCurrentImage}
            type="button"
            onClick={(event) => {
              event.stopPropagation();
              onDownload(currentItem.src);
            }}
          >
            <Download className="h-4 w-4" />
          </button>
          <div
            aria-hidden="true"
            className="h-5 w-px shrink-0 bg-muted-foreground/15"
          />
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
          <span className="min-w-10 text-right text-xs font-medium tabular-nums text-muted-foreground">
            {Math.round(zoom * 100)}%
          </span>
        </div>
      </div>
      {menu && canDownloadCurrentImage ? (
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
function ImageBlock({ alt, dim, resolvedSrc, src }: ImageBlockProps) {
  const [lightboxState, setLightboxState] = React.useState<{
    galleryIndex: number;
    galleryItems?: ImageGalleryItem[];
    sourceBox: ImageLightboxBox;
    sourceScope: Element | null;
  } | null>(null);
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

  const handleImageLoad = React.useCallback(
    (image: HTMLImageElement) => {
      rememberDecodedImageDimensions(
        resolvedSrc,
        image.naturalWidth,
        image.naturalHeight,
      );
      updateSpoilerMediaSize(image);
    },
    [resolvedSrc, updateSpoilerMediaSize],
  );

  const imageRef = React.useCallback(
    (image: HTMLImageElement | null) => {
      inlineImageRef.current = image;
      if (image?.complete) handleImageLoad(image);
    },
    [handleImageLoad],
  );

  const { intrinsicDimensions, useFixedReserveBox } = useFrozenImageReserve(
    dim,
    resolvedSrc,
  );

  const currentSpoilerMediaSize =
    spoilerMediaSize?.src === resolvedSrc ? spoilerMediaSize : null;
  const hiddenSpoilerMediaSize = isHiddenInSpoiler
    ? currentSpoilerMediaSize
    : null;

  const spoilerMediaStyle = imageReserveStyle({
    hiddenSpoilerMediaSize,
    intrinsicDimensions,
    useFixedReserveBox,
  });

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
      const sourceBox = imageLightboxBoxFromRect(rect);
      const sourceScope = triggerRef.current
        ? imageLightboxSourceScopeForTrigger(triggerRef.current)
        : null;
      const gallery = triggerRef.current
        ? visibleImageGalleryForTrigger(
            triggerRef.current,
            { alt, dim, resolvedSrc, src, thumbnailBox: sourceBox },
            sourceScope,
          )
        : { galleryIndex: 0, galleryItems: undefined };
      setLightboxState({
        galleryIndex: gallery.galleryIndex,
        galleryItems: gallery.galleryItems,
        sourceBox,
        sourceScope,
      });
    },
    [alt, dim, resolvedSrc, src],
  );

  const handleImageTriggerClick = () => {
    if (inlineImageRef.current) {
      openLightbox(inlineImageRef.current);
    }
  };

  const handleDownload = React.useCallback(
    (downloadSrc: string | undefined) => {
      setMenu(null);
      if (!downloadSrc) return;
      invokeTauri("download_image", { url: downloadSrc }).catch(
        (err: unknown) => {
          const msg = err instanceof Error ? err.message : "Download failed";
          toast.error(msg);
        },
      );
    },
    [],
  );

  return (
    <>
      <button
        aria-hidden={isHiddenInSpoiler ? true : undefined}
        aria-label={alt?.trim() ? `Zoom image: ${alt}` : "Zoom image"}
        className={cn(
          "mt-1 inline-block min-w-0 max-w-full cursor-zoom-in rounded-xl border-0 bg-transparent p-0 text-left align-top focus:outline-hidden focus-visible:ring-2 focus-visible:ring-ring/50",
          lightboxState && "opacity-0",
        )}
        data-image-lightbox-resolved-src={resolvedSrc}
        data-image-lightbox-alt={alt}
        data-image-lightbox-dim={dim}
        data-image-lightbox-src={src}
        data-image-lightbox-trigger=""
        data-testid="message-image-lightbox-trigger"
        ref={triggerRef}
        tabIndex={isHiddenInSpoiler ? -1 : undefined}
        type="button"
        onClick={handleImageTriggerClick}
        onContextMenuCapture={handleContextMenu}
      >
        <img
          alt={alt}
          className="block h-auto max-h-64 max-w-[min(24rem,100%)] rounded-xl object-contain"
          data-spoiler-media-size={hiddenSpoilerMediaSize ? "" : undefined}
          height={intrinsicDimensions.height}
          ref={imageRef}
          src={resolvedSrc}
          style={spoilerMediaStyle}
          width={intrinsicDimensions.width}
          onLoad={(event) => handleImageLoad(event.currentTarget)}
        />
      </button>
      {menu && src ? (
        <ImageDownloadContextMenu
          onDownload={() => handleDownload(src)}
          position={menu}
        />
      ) : null}
      {lightboxState && resolvedSrc ? (
        <ImageZoomOverlay
          alt={alt}
          galleryIndex={lightboxState.galleryIndex}
          galleryItems={lightboxState.galleryItems}
          onDownload={handleDownload}
          onClose={() => setLightboxState(null)}
          resolvedSrc={resolvedSrc}
          sourceBox={lightboxState.sourceBox}
          sourceScope={lightboxState.sourceScope}
          src={src}
        />
      ) : null}
    </>
  );
}

function createMarkdownComponents(
  variant: MarkdownVariant,
  runtimeRef: React.RefObject<MarkdownRuntime>,
  interactive = true,
  mediaInset = false,
): Components {
  const paragraphClassName =
    variant === "tight"
      ? "leading-5"
      : variant === "compact"
        ? "leading-6"
        : "leading-[inherit]";
  const listItemClassName = "[&_p]:inline";
  const listClassName = "space-y-1 pl-6 marker:text-muted-foreground/80";

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

      const label = getReactNodeText(children);

      // Generic file attachment: a `[filename](url)` link whose href matches an
      // imeta entry with a non-image, non-video MIME. Render a download card
      // instead of a plain link. (Media uses the `img` renderer, not this path.)
      const card = resolveFileCard(
        href ? imetaByUrl?.get(href) : undefined,
        href,
        label,
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
      if (href) {
        const messageLinkTarget = resolveMessageLinkRenderTarget({
          href,
          label,
        });
        if (messageLinkTarget.kind !== "none") {
          if (messageLinkTarget.kind === "pill") {
            return (
              <MessageLinkPill
                channels={runtimeRef.current.channels}
                href={href}
                interactive={interactive}
                link={messageLinkTarget.link}
                onOpenMessageLink={onOpenMessageLink}
              />
            );
          }

          return (
            <a
              {...props}
              className="font-medium text-primary underline underline-offset-4 transition-colors hover:text-primary/80 cursor-pointer"
              href={href}
              onClick={(event) => {
                event.preventDefault();
                onOpenMessageLink(messageLinkTarget.link);
              }}
            >
              {children}
            </a>
          );
        }
        // Malformed message deep link — fall through to the default
        // anchor (renders as a normal external link).
      }

      const supportedLinkPreview = href
        ? parseSupportedLinkPreview(href)
        : null;
      const isLinearLink = supportedLinkPreview?.kind === "linear-issue";

      return (
        <a
          {...props}
          className={cn(
            "font-medium underline underline-offset-4 transition-colors",
            isLinearLink ? "linear-link" : "text-primary hover:text-primary/80",
          )}
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
          <span
            className={cn(
              mediaInset && "mx-1.5 block max-w-[calc(100%-0.75rem)]",
            )}
            data-block-media=""
          >
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
        <span data-block-media="" className="block min-w-0 max-w-full">
          <ImageBlock
            alt={alt}
            dim={entry?.dim}
            resolvedSrc={resolvedSrc}
            src={src}
          />
        </span>
      );
    },
    input: MarkdownInput,
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
          <div className="mt-1 grid w-full min-w-0 max-w-lg grid-cols-2 gap-1.5 [&_br]:hidden [&_[data-block-media]]:mt-0 [&_[data-block-media]]:max-w-none [&_img]:mt-0 [&_img]:w-full [&_img]:max-w-full">
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
        <UserProfilePopover
          botIdenticonValue={mentionLabel}
          pubkey={pubkey}
          role={isAgentMention ? "bot" : undefined}
          triggerElement="span"
        >
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

      return (
        <MessageLinkPill
          channels={channels}
          href={href}
          interactive={interactive}
          link={parsed.value}
          onOpenMessageLink={onOpenMessageLink}
        />
      );
    },
  } as Components;
}

function MarkdownInner({
  channelNames,
  className,
  compact = false,
  content,
  customEmoji,
  imetaByUrl,
  interactive = true,
  agentMentionPubkeysByName,
  mediaInset = false,
  mentionNames,
  mentionPubkeysByName,
  searchQuery,
  tight = false,
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
  const linkPreviews = React.useMemo(
    () => (interactive ? extractSupportedLinkPreviews(content) : []),
    [content, interactive],
  );
  const runtimeRef = useLatestRef<MarkdownRuntime>({
    agentMentionPubkeysByName,
    channels,
    imetaByUrl,
    mentionPubkeysByName,
    onOpenChannel,
    onOpenMessageLink,
  });

  const variant: MarkdownVariant = tight
    ? "tight"
    : compact
      ? "compact"
      : "default";

  const components = React.useMemo(
    () =>
      createMarkdownComponents(variant, runtimeRef, interactive, mediaInset),
    [variant, runtimeRef, interactive, mediaInset],
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

  const resolvedLinkPreviews = useResolvedLinkPreviews(linkPreviews);

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
        // Variant overrides: density tweaks for agent-session transcript surfaces.
        // Layered after the base owl-spacing set so tailwind-merge lets the
        // narrower leading + tighter inter-block gaps win for compact/tight.
        variant === "compact" &&
          "leading-6 [&>*+*]:mt-2 [&>*+h1]:mt-3 [&>*+h2]:mt-3 [&>*+h3]:mt-3 [&>*+blockquote]:mt-3 [&>blockquote+*]:mt-3 [&>*+[data-code-block]]:mt-3 [&>[data-code-block]+*]:mt-3 [&>*+[data-table-block]]:mt-3 [&>[data-table-block]+*]:mt-3 [&>*+hr]:mt-3.5 [&>hr+*]:mt-3.5 [&>p+ul]:mt-1 [&>p+ol]:mt-1 [&>div+ul]:mt-1 [&>div+ol]:mt-1",
        variant === "tight" &&
          "leading-5 [&>*+*]:mt-2 [&>*+h1]:mt-2.5 [&>*+h2]:mt-2.5 [&>*+h3]:mt-2.5 [&>*+blockquote]:mt-3 [&>blockquote+*]:mt-3 [&>*+[data-code-block]]:mt-3 [&>[data-code-block]+*]:mt-3 [&>*+[data-table-block]]:mt-3 [&>[data-table-block]+*]:mt-3 [&>*+hr]:mt-3.5 [&>hr+*]:mt-3.5 [&>p+ul]:mt-1 [&>p+ol]:mt-1 [&>div+ul]:mt-1 [&>div+ol]:mt-1",
        className,
      )}
    >
      <VideoReviewMarkdownContext.Provider value={videoReviewContext}>
        {markdownNode}
        {resolvedLinkPreviews.length > 0 ? (
          <AttachmentGroup
            className="max-w-full flex-wrap overflow-visible pb-0"
            data-link-preview-list=""
          >
            {resolvedLinkPreviews.map((preview) => (
              <LinkPreviewAttachment key={preview.href} preview={preview} />
            ))}
          </AttachmentGroup>
        ) : null}
      </VideoReviewMarkdownContext.Provider>
    </div>
  );
}

export const Markdown = React.memo(
  MarkdownInner,
  (prev, next) =>
    prev.content === next.content &&
    prev.className === next.className &&
    prev.compact === next.compact &&
    prev.customEmoji === next.customEmoji &&
    prev.interactive === next.interactive &&
    prev.mediaInset === next.mediaInset &&
    prev.tight === next.tight &&
    prev.agentMentionPubkeysByName === next.agentMentionPubkeysByName &&
    prev.mentionPubkeysByName === next.mentionPubkeysByName &&
    shallowArrayEqual(prev.mentionNames, next.mentionNames) &&
    shallowArrayEqual(prev.channelNames, next.channelNames) &&
    prev.imetaByUrl === next.imetaByUrl &&
    prev.searchQuery === next.searchQuery &&
    prev.videoReviewContext === next.videoReviewContext,
);

Markdown.displayName = "Markdown";
