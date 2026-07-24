import * as React from "react";
import { createPortal } from "react-dom";
import type { Components } from "react-markdown";
import {
  ChevronLeft,
  ChevronRight,
  Download,
  ZoomIn,
  ZoomOut,
} from "lucide-react";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { toast } from "sonner";

import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { requestOpenSnapshotImport } from "@/features/agents/openSnapshotImportFromUrlEvent";
import {
  parseMessageLink,
  resolveMessageLinkRenderTarget,
  type ParsedMessageLink,
} from "@/features/messages/lib/messageLink";
import { UserProfilePopover } from "@/features/profile/ui/UserProfilePopover";
import { invokeTauri } from "@/shared/api/tauri";
import { useChannelNavigation } from "@/shared/context/ChannelNavigationContext";
import { cn } from "@/shared/lib/cn";
import { copyTextToClipboard } from "@/shared/lib/clipboard";
import {
  extractSupportedLinkPreviews,
  parseSupportedLinkPreview,
} from "@/shared/lib/linkPreview";
import { useResolvedLinkPreviews } from "@/shared/lib/useResolvedLinkPreviews";
import { rewriteRelayUrl } from "@/shared/lib/mediaUrl";
import { AttachmentGroup } from "@/shared/ui/attachment";
import { ConfigNudgeCard } from "@/shared/ui/config-nudge-attachment";
import { LinkPreviewAttachment } from "@/shared/ui/link-preview-attachment";
import { useSmoothCorners } from "@/shared/ui/smoothCorners";
import {
  computeConfigNudge,
  selectProseOrNudge,
} from "@/shared/lib/computeConfigNudge";
import {
  INLINE_CODE_CHIP_CLASS,
  MENTION_CHIP_BASE_CLASSES,
  MENTION_CHIP_HOVER_CLASSES,
  MENTION_CHIP_PREFIX_CLASS,
  MESSAGE_MARKDOWN_CLASS,
} from "@/shared/ui/mentionChip";

import {
  classifyChildren,
  hasBlockMedia,
  isImageOnlyParagraph,
  shallowArrayEqual,
  shallowRecordEqual,
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
import {
  MediaContextMenu,
  type MediaContextMenuPosition,
  useDismissMediaContextMenu,
} from "./markdown/MediaContextMenu";
import { isVideoMedia } from "./markdown/mediaEntry";
import {
  clampImageLightboxZoom,
  type ImageGalleryDirection,
  type ImageGalleryItem,
  type ImageLightboxBox,
  type ImageLightboxCornerRadii,
  IMAGE_LIGHTBOX_CONTROL_SUPPRESS_CLOSE_MS,
  IMAGE_LIGHTBOX_EASE_IN_OUT,
  IMAGE_LIGHTBOX_EASE_OUT,
  IMAGE_LIGHTBOX_ENTER_MS,
  IMAGE_LIGHTBOX_EXIT_MS,
  IMAGE_LIGHTBOX_FADE_ENTER_MS,
  IMAGE_LIGHTBOX_FADE_EXIT_MS,
  IMAGE_LIGHTBOX_GALLERY_BLUR_PX,
  IMAGE_LIGHTBOX_GALLERY_EASE,
  IMAGE_LIGHTBOX_GALLERY_SLIDE_DISTANCE_PX,
  IMAGE_LIGHTBOX_GALLERY_SLIDE_MS,
  IMAGE_LIGHTBOX_MAX_ZOOM,
  IMAGE_LIGHTBOX_MIN_ZOOM,
  IMAGE_LIGHTBOX_REDUCED_MOTION_MS,
  IMAGE_LIGHTBOX_TRACKPAD_ZOOM_IDLE_MS,
  IMAGE_LIGHTBOX_WHEEL_ZOOM_MAX_DELTA,
  IMAGE_LIGHTBOX_WHEEL_ZOOM_SPEED,
  IMAGE_LIGHTBOX_ZOOM_STEP,
  IMAGE_LIGHTBOX_ZOOM_TRANSITION_MS,
  imageLightboxBasisBoxForItem,
  imageLightboxBoxFromRect,
  imageLightboxCornerRadiiFromElement,
  imageLightboxCornerRadiiStyle,
  imageLightboxExpandedCornerRadii,
  imageLightboxReturnTargetForItem,
  imageLightboxSourceScopeForTrigger,
  imageLightboxStyle,
  imageLightboxTargetBox,
  imageLightboxTransform,
  imageLightboxZoomBox,
  normalizedWheelDeltaY,
  visibleImageGalleryForTrigger,
} from "./markdown/imageLightbox";
import { MarkdownTable } from "./markdown/MarkdownTable";
import { MaskedLinkTooltip } from "./markdown/MaskedLinkTooltip";
import { ProgressiveImage } from "./markdown/ProgressiveImage";
import { MessageLinkPill } from "./markdown/MessageLinkPill";
import { renderCachedMarkdown } from "./markdown/nodeCache";
import {
  MarkdownRuntimeContext,
  useMarkdownRuntime,
} from "./markdown/runtimeContext";
import { AgentSnapshotCard } from "./markdown/AgentSnapshotCard";
import { resolveFileCard, resolveSnapshotCard } from "./markdownFileCard";
import type { MarkdownProps, MarkdownRuntime } from "./markdown/types";
import { SpoilerInline } from "./markdown/SpoilerInline";
import {
  imageReserveStyle,
  isInsideHiddenSpoiler,
  getReactNodeText,
  rememberDecodedImageDimensions,
  useFrozenImageReserve,
  useStableArray,
} from "./markdown/utils";
import {
  MarkdownVideoPlayer,
  VideoReviewMarkdownContext,
} from "./markdown/MarkdownVideoPlayer";

type ImageBlockProps = {
  alt: string | undefined;
  dim?: string;
  resolvedSrc: string | undefined;
  src: string | undefined;
  thumbSrc?: string;
};

type WebKitGestureLikeEvent = Event & {
  scale?: number;
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

function ImageZoomOverlay({
  alt,
  galleryIndex = 0,
  galleryItems,
  onCopy,
  onDownload,
  onClose,
  resolvedSrc,
  sourceBox,
  sourceCornerRadii,
  sourceScope,
  src,
}: {
  alt: string | undefined;
  galleryIndex?: number;
  galleryItems?: ImageGalleryItem[];
  onCopy: (src: string | undefined) => void;
  onDownload: (src: string | undefined) => void;
  onClose: () => void;
  resolvedSrc: string;
  sourceBox: ImageLightboxBox;
  sourceCornerRadii: ImageLightboxCornerRadii;
  sourceScope?: Element | null;
  src: string | undefined;
}) {
  const shouldReduceMotion = useReducedMotion();
  const prefersReducedMotion = shouldReduceMotion === true;
  const fallbackGalleryItems = React.useMemo<ImageGalleryItem[]>(
    () => [
      {
        alt,
        resolvedSrc,
        src,
        thumbnailBox: sourceBox,
        thumbnailCornerRadii: sourceCornerRadii,
      },
    ],
    [alt, resolvedSrc, sourceBox, sourceCornerRadii, src],
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
  const isReturning = phase === "closing" || phase === "fading";
  const [hasEntered, setHasEntered] = React.useState(prefersReducedMotion);
  const [isAdjustingZoom, setIsAdjustingZoom] = React.useState(false);
  const [isGalleryNavigating, setIsGalleryNavigating] = React.useState(false);
  const [menu, setMenu] = React.useState<MediaContextMenuPosition | null>(null);
  const currentItem = items[currentIndex] ?? items[0];
  const basisBox = React.useMemo(
    () => imageLightboxBasisBoxForItem(currentItem, sourceBox),
    [currentItem, sourceBox],
  );
  const [targetBox, setTargetBox] = React.useState(() =>
    imageLightboxTargetBox(basisBox),
  );
  const [returnBox, setReturnBox] = React.useState(sourceBox);
  const [returnCornerRadii, setReturnCornerRadii] =
    React.useState(sourceCornerRadii);
  const [zoom, setZoom] = React.useState(IMAGE_LIGHTBOX_MIN_ZOOM);
  const controlPointerDownRef = React.useRef(false);
  const fadeTimerRef = React.useRef<number | null>(null);
  const galleryTransitionTimerRef = React.useRef<number | null>(null);
  const closeTimerRef = React.useRef<number | null>(null);
  const dialogRef = React.useRef<HTMLDivElement | null>(null);
  const imageFrameSurfaceRef = React.useRef<HTMLDivElement | null>(null);
  const descriptionId = React.useId();
  const gestureScaleRef = React.useRef(1);
  const previouslyFocusedElementRef = React.useRef<HTMLElement | null>(null);
  const suppressCloseUntilRef = React.useRef(0);
  const zoomIdleTimerRef = React.useRef<number | null>(null);
  const hasPreviousImage = currentIndex > 0;
  const hasNextImage = currentIndex < items.length - 1;
  const canActOnCurrentImage = Boolean(currentItem.src);
  useSmoothCorners(imageFrameSurfaceRef);

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
    const returnTarget = imageLightboxReturnTargetForItem(
      currentItem,
      sourceBox,
      sourceCornerRadii,
      sourceScope,
    );
    setReturnBox(returnTarget.box);
    setReturnCornerRadii(returnTarget.cornerRadii);

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
  }, [
    currentItem,
    onClose,
    prefersReducedMotion,
    sourceBox,
    sourceCornerRadii,
    sourceScope,
  ]);

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

  useDismissMediaContextMenu(Boolean(menu), closeMenu);

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
  const displayBox = imageLightboxZoomBox(targetBox, zoom);
  const frameBox = isReturning ? returnBox : targetBox;
  const frameCornerRadii = isReturning
    ? returnCornerRadii
    : imageLightboxExpandedCornerRadii();
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
      ? "border-radius, height, left, opacity, top, transform, width"
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
      if (canActOnCurrentImage) {
        setMenu({ x: event.clientX, y: event.clientY });
      }
    },
    [canActOnCurrentImage, markControlGesture],
  );
  const handleMenuCopy = React.useCallback(() => {
    setMenu(null);
    markControlGesture();
    onCopy(currentItem.src);
  }, [currentItem.src, markControlGesture, onCopy]);
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
          ...imageLightboxCornerRadiiStyle(frameCornerRadii),
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
        <div
          className="relative h-full w-full shadow-2xl"
          style={{
            ...imageLightboxCornerRadiiStyle(frameCornerRadii),
            transitionDuration: `${imageTransitionDuration}ms`,
            transitionProperty: isReturning ? "border-radius" : "none",
            transitionTimingFunction: isClosing
              ? IMAGE_LIGHTBOX_EASE_IN_OUT
              : IMAGE_LIGHTBOX_EASE_OUT,
          }}
        >
          <div
            ref={imageFrameSurfaceRef}
            className="relative h-full w-full overflow-hidden"
            style={{
              ...imageLightboxCornerRadiiStyle(frameCornerRadii),
              transitionDuration: `${imageTransitionDuration}ms`,
              transitionProperty: isReturning ? "border-radius" : "none",
              transitionTimingFunction: isClosing
                ? IMAGE_LIGHTBOX_EASE_IN_OUT
                : IMAGE_LIGHTBOX_EASE_OUT,
            }}
          >
            <AnimatePresence
              custom={galleryDirection}
              initial={false}
              mode="popLayout"
            >
              <motion.img
                alt={currentItem.alt}
                animate="center"
                className={cn(
                  "absolute inset-0 h-full w-full",
                  // The expanded frame matches the image aspect ratio, so
                  // switching to cover at close starts without a visual jump.
                  // As the frame morphs to the mosaic tile's aspect ratio, the
                  // image is progressively cropped into the same fill geometry
                  // as its thumbnail instead of snapping after it lands.
                  isReturning ? "object-cover" : "object-contain",
                )}
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
            disabled={!canActOnCurrentImage}
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
      {menu && canActOnCurrentImage ? (
        <MediaContextMenu
          dataAttributes={[
            "data-image-context-menu",
            "data-image-lightbox-controls",
          ]}
          items={[
            { label: "Copy image", onSelect: handleMenuCopy },
            { label: "Download image", onSelect: handleMenuDownload },
          ]}
          portalContainer={dialogRef.current ?? undefined}
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
function ImageBlock({ alt, dim, resolvedSrc, src, thumbSrc }: ImageBlockProps) {
  const [lightboxState, setLightboxState] = React.useState<{
    galleryIndex: number;
    galleryItems?: ImageGalleryItem[];
    sourceBox: ImageLightboxBox;
    sourceCornerRadii: ImageLightboxCornerRadii;
    sourceScope: Element | null;
  } | null>(null);
  const [isHiddenInSpoiler, setIsHiddenInSpoiler] = React.useState(false);
  const [menu, setMenu] = React.useState<MediaContextMenuPosition | null>(null);
  const inlineImageRef = React.useRef<HTMLImageElement | null>(null);
  const thumbnailImageRef = React.useRef<HTMLImageElement | null>(null);
  const triggerRef = React.useRef<HTMLButtonElement | null>(null);
  useSmoothCorners(inlineImageRef);
  useSmoothCorners(thumbnailImageRef);

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
  useDismissMediaContextMenu(Boolean(menu), closeMenu);

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
      const sourceCornerRadii = imageLightboxCornerRadiiFromElement(image);
      const sourceScope = triggerRef.current
        ? imageLightboxSourceScopeForTrigger(triggerRef.current)
        : null;
      const gallery = triggerRef.current
        ? visibleImageGalleryForTrigger(
            triggerRef.current,
            {
              alt,
              dim,
              resolvedSrc,
              src,
              thumbnailBox: sourceBox,
              thumbnailCornerRadii: sourceCornerRadii,
            },
            sourceScope,
          )
        : { galleryIndex: 0, galleryItems: undefined };
      setLightboxState({
        galleryIndex: gallery.galleryIndex,
        galleryItems: gallery.galleryItems,
        sourceBox,
        sourceCornerRadii,
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

  const handleCopyImage = React.useCallback((copySrc: string | undefined) => {
    setMenu(null);
    if (!copySrc) return;
    invokeTauri("copy_image_to_clipboard", { url: copySrc })
      .then(() => {
        toast.success("Copied to clipboard");
      })
      .catch((err: unknown) => {
        const msg = err instanceof Error ? err.message : "Copy failed";
        toast.error(msg);
      });
  }, []);

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
          "mt-1 inline-block min-w-0 max-w-full cursor-zoom-in overflow-hidden rounded-2xl border-0 bg-transparent p-0 text-left align-top focus:outline-hidden focus-visible:ring-2 focus-visible:ring-ring/50",
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
        <ProgressiveImage
          alt={alt}
          fullImageRef={inlineImageRef}
          height={intrinsicDimensions.height}
          onFullLoad={handleImageLoad}
          onThumbnailLoad={updateSpoilerMediaSize}
          resolvedSrc={resolvedSrc}
          showSpoilerSize={Boolean(hiddenSpoilerMediaSize)}
          style={spoilerMediaStyle}
          thumbnailRef={thumbnailImageRef}
          thumbSrc={thumbSrc}
          width={intrinsicDimensions.width}
        />
      </button>
      {menu && src ? (
        <MediaContextMenu
          dataAttributes={["data-image-context-menu"]}
          items={[
            { label: "Copy image", onSelect: () => handleCopyImage(src) },
            { label: "Download image", onSelect: () => handleDownload(src) },
          ]}
          position={menu}
        />
      ) : null}
      {lightboxState && resolvedSrc ? (
        <ImageZoomOverlay
          alt={alt}
          galleryIndex={lightboxState.galleryIndex}
          galleryItems={lightboxState.galleryItems}
          onCopy={handleCopyImage}
          onDownload={handleDownload}
          onClose={() => setLightboxState(null)}
          resolvedSrc={resolvedSrc}
          sourceBox={lightboxState.sourceBox}
          sourceCornerRadii={lightboxState.sourceCornerRadii}
          sourceScope={lightboxState.sourceScope}
          src={src}
        />
      ) : null}
    </>
  );
}

function ImageMosaic({ children }: { children: React.ReactNode[] }) {
  const mosaicRef = React.useRef<HTMLDivElement | null>(null);
  const isTriptych = children.length === 3;
  const hasOddTail = children.length > 3 && children.length % 2 === 1;
  useSmoothCorners(mosaicRef);

  return (
    <div
      className={cn(
        "mt-1 grid w-full min-w-0 max-w-lg grid-cols-2 gap-1.5 overflow-hidden rounded-2xl [&_br]:hidden [&_[data-block-media]]:min-h-0 [&_[data-block-media]]:max-w-none [&_[data-block-media]]:overflow-hidden [&_[data-block-media]>button]:m-0 [&_[data-block-media]>button]:h-full [&_[data-block-media]>button]:w-full [&_[data-block-media]>button]:max-w-none [&_[data-block-media]>button]:rounded-none [&_[data-block-media]_[data-progressive-image-frame]]:!h-full [&_[data-block-media]_[data-progressive-image-frame]]:!w-full [&_[data-block-media]_img]:!h-full [&_[data-block-media]_img]:!max-h-none [&_[data-block-media]_img]:!w-full [&_[data-block-media]_img]:!max-w-none [&_[data-block-media]_img]:rounded-none [&_[data-block-media]_img]:object-cover",
        isTriptych
          ? "h-80 grid-rows-2 [&_[data-block-media]]:h-auto [&_[data-block-media]:first-child]:row-span-2"
          : "[&_[data-block-media]]:h-48",
        hasOddTail && "[&_[data-block-media]:last-child]:col-span-2",
      )}
      data-image-mosaic=""
      data-image-mosaic-count={children.length}
      ref={mosaicRef}
    >
      {children}
    </div>
  );
}

/**
 * An external `[text](href)` link with a custom right-click menu.
 *
 * Buzz renders inside a native webview whose default context menu has no
 * useful link actions, so a plain right-click on a link is a no-op. This adds
 * an in-app menu with "Open link" (via the OS opener, matching the anchor's
 * left-click `target="_blank"` behavior) and "Copy link" (the real href, not
 * the masked display text).
 */
function ExternalLinkAnchor({
  anchorProps,
  children,
  href,
  isLinearLink,
  label,
}: {
  anchorProps: React.ComponentPropsWithoutRef<"a">;
  children: React.ReactNode;
  href: string | undefined;
  isLinearLink: boolean;
  label: string;
}) {
  const [menu, setMenu] = React.useState<MediaContextMenuPosition | null>(null);
  const closeMenu = React.useCallback(() => setMenu(null), []);
  useDismissMediaContextMenu(Boolean(menu), closeMenu);

  const anchor = (
    <a
      {...anchorProps}
      className={cn(
        "font-medium underline underline-offset-4 transition-colors",
        isLinearLink ? "linear-link" : "text-primary hover:text-primary/80",
      )}
      href={href}
      onContextMenuCapture={(event) => {
        if (!href) return;
        event.preventDefault();
        setMenu({ x: event.clientX, y: event.clientY });
      }}
      rel="noreferrer"
      target="_blank"
    >
      {children}
    </a>
  );

  return (
    <>
      <MaskedLinkTooltip disabled={isLinearLink} href={href} label={label}>
        {anchor}
      </MaskedLinkTooltip>
      {menu && href ? (
        <MediaContextMenu
          dataAttributes={["data-link-context-menu"]}
          items={[
            {
              label: "Open link",
              onSelect: () => {
                closeMenu();
                void openUrl(href).catch(() => {
                  toast.error("Failed to open link");
                });
              },
            },
            {
              label: "Copy link",
              onSelect: () => {
                closeMenu();
                copyTextToClipboard(href, "Link copied to clipboard");
              },
            },
          ]}
          position={menu}
        />
      ) : null}
    </>
  );
}

function createMarkdownComponents(
  interactive = true,
  mediaInset = false,
): Components {
  const listItemClassName = "[&_p]:inline";
  const listClassName = "space-y-1 pl-6 marker:text-muted-foreground/80";

  function MarkdownAnchor({
    children,
    href,
    ...props
  }: React.ComponentPropsWithoutRef<"a">) {
    const {
      channels,
      imetaByUrl,
      onOpenMessageLink,
      onImportSnapshotFromUrl,
      snapshotSharedBy,
    } = useMarkdownRuntime();
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

    // Snapshot attachment (agent or team): classify before generic FileCard.
    // resolveSnapshotCard checks the filename suffix + SHA-256 field.
    const snapshotCard = resolveSnapshotCard(
      href ? imetaByUrl?.get(href) : undefined,
      href,
      label,
    );
    if (snapshotCard) {
      return (
        <AgentSnapshotCard
          displayName={snapshotCard.displayName}
          href={snapshotCard.href}
          filename={snapshotCard.filename}
          sharedBy={snapshotSharedBy}
          size={snapshotCard.size}
          sha256={snapshotCard.sha256}
          snapshotKind={snapshotCard.snapshotKind}
          thumb={snapshotCard.thumb}
          onImport={(fileBytes, fileName) => {
            onImportSnapshotFromUrl?.(
              fileBytes,
              fileName,
              snapshotCard.snapshotKind,
            );
          }}
        />
      );
    }

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
        <FileCard href={card.href} filename={card.filename} size={card.size} />
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
              channels={channels}
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

    const supportedLinkPreview = href ? parseSupportedLinkPreview(href) : null;
    const isLinearLink = supportedLinkPreview?.kind === "linear-issue";

    return (
      <ExternalLinkAnchor
        anchorProps={props}
        href={href}
        isLinearLink={isLinearLink}
        label={label}
      >
        {children}
      </ExternalLinkAnchor>
    );
  }

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
    a: MarkdownAnchor,
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
    img: function MarkdownImage({ alt, src }) {
      const { imetaByUrl } = useMarkdownRuntime();
      const entry = src ? imetaByUrl?.get(src) : undefined;
      const isVideo = src ? isVideoMedia(src, entry?.m) : false;
      if (!interactive) {
        const fallbackLabel = isVideo ? "Video attachment" : "Image attachment";
        return <span>{alt?.trim() || fallbackLabel}</span>;
      }

      const resolvedSrc = src ? rewriteRelayUrl(src) : src;
      if (isVideo && src && resolvedSrc) {
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
      return (
        <span data-block-media="" className="block min-w-0 max-w-full">
          <ImageBlock
            alt={alt}
            dim={entry?.dim}
            resolvedSrc={resolvedSrc}
            src={src}
            thumbSrc={entry?.thumb ? rewriteRelayUrl(entry.thumb) : undefined}
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
      // Detect media-only paragraphs (images + <br> from remarkBreaks).
      // Multi-image: render as a compact, count-aware mosaic. Two images split
      // a row, three form a hero-and-stack triptych, and larger odd counts let
      // the final image span both columns.
      // Single media: render as a plain <div> to avoid invalid <p><div> nesting
      // (the img component returns block-level wrappers for lightbox/video).
      const childArray = React.Children.toArray(children);
      const { imageChildren } = classifyChildren(childArray);

      if (isImageOnlyParagraph(childArray)) {
        return <ImageMosaic>{imageChildren}</ImageMosaic>;
      }

      if (hasBlockMedia(childArray)) {
        return <div>{children}</div>;
      }

      return <p>{children}</p>;
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
    table: ({ children }) => <MarkdownTable>{children}</MarkdownTable>,
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
    mention: function MarkdownMention({
      children,
    }: {
      children?: React.ReactNode;
    }) {
      const { agentMentionPubkeysByName, mentionPubkeysByName } =
        useMarkdownRuntime();
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
      // Only chips that actually open a profile get the clickable affordance.
      // A mention whose pubkey didn't resolve stays a plain chip — a pointer
      // cursor there promises a click that does nothing.
      const opensProfile = interactive && pubkey !== undefined;
      const mentionNode = (
        <span
          data-mention=""
          className={cn(
            MENTION_CHIP_BASE_CLASSES,
            opensProfile && "cursor-pointer",
            opensProfile && MENTION_CHIP_HOVER_CLASSES,
            isAgentMention && "agent-mention-highlight",
          )}
        >
          {renderedMentionText}
        </span>
      );

      return opensProfile ? (
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
    "channel-link": function MarkdownChannelLink({
      children,
    }: {
      children?: React.ReactNode;
    }) {
      const { channels, onOpenChannel } = useMarkdownRuntime();
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
    "message-link": function MarkdownMessageLink({
      children,
    }: {
      children?: React.ReactNode;
    }) {
      const { channels, onOpenMessageLink } = useMarkdownRuntime();
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

/**
 * The component map only varies by the two boolean render flags, so at most
 * four instances ever exist. Module-stable maps mean cached markdown element
 * trees (see ./markdown/nodeCache.ts) never embed per-mount closures.
 */
const MARKDOWN_COMPONENT_SCHEMA_VERSION = "4";
const markdownComponentsByVariant = new Map<string, MarkdownComponentSet>();

type MarkdownComponentSet = { components: Components; variant: string };

/**
 * Returns the component map together with the `variant` token that fully
 * identifies it. The token doubles as the variant segment of the parse-cache
 * key (see nodeCache.ts), so the map partitioning and the key partitioning
 * come from one place and cannot drift apart: a new render flag added here
 * automatically partitions the cache too.
 */
function getMarkdownComponents(
  interactive: boolean,
  mediaInset: boolean,
): MarkdownComponentSet {
  const variant = `${MARKDOWN_COMPONENT_SCHEMA_VERSION}:${interactive ? "i" : ""}${mediaInset ? "m" : ""}`;
  let entry = markdownComponentsByVariant.get(variant);
  if (!entry) {
    entry = {
      components: createMarkdownComponents(interactive, mediaInset),
      variant,
    };
    markdownComponentsByVariant.set(variant, entry);
  }
  return entry;
}

function MarkdownInner({
  channelNames,
  className,
  configNudgeAuthorPubkey,
  content,
  customEmoji,
  imetaByUrl,
  interactive = true,
  agentMentionPubkeysByName,
  mediaInset = false,
  mentionNames,
  mentionPubkeysByName,
  searchQuery,
  snapshotSharedBy,
  videoReviewContext,
}: MarkdownProps) {
  const { channels: rawChannels } = useChannelNavigation();
  const channels = useStableArray(rawChannels);
  const { goChannel, goAgents } = useAppNavigation();
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
  const configNudge = React.useMemo(
    () => computeConfigNudge(content, interactive, configNudgeAuthorPubkey),
    [content, interactive, configNudgeAuthorPubkey],
  );
  const runtime = React.useMemo<MarkdownRuntime>(
    () => ({
      agentMentionPubkeysByName,
      channels,
      imetaByUrl,
      mentionPubkeysByName,
      onOpenChannel,
      onOpenMessageLink,
      snapshotSharedBy,
      onImportSnapshotFromUrl: (
        fileBytes: number[],
        fileName: string,
        snapshotKind: "agent" | "team",
      ) => {
        requestOpenSnapshotImport({ fileBytes, fileName, snapshotKind });
        void goAgents();
      },
    }),
    [
      agentMentionPubkeysByName,
      channels,
      imetaByUrl,
      mentionPubkeysByName,
      onOpenChannel,
      onOpenMessageLink,
      snapshotSharedBy,
      goAgents,
    ],
  );

  let processedContent = content;

  // Note: stripping the sentinel here is intentionally omitted. When
  // configNudge !== null, selectProseOrNudge() returns null — suppressing
  // the prose node entirely — so processedContent is never rendered and
  // stripConfigNudgeSentinel would be dead work on that path.

  if (/^(?:\s{2}\n)+/.test(processedContent)) {
    processedContent = `\u200B${processedContent}`;
  }

  if (/(?:\s{2}\n)+$/.test(processedContent)) {
    processedContent = `${processedContent}\u200B`;
  }

  const resolvedLinkPreviews = useResolvedLinkPreviews(linkPreviews);

  // When a config-nudge suppresses the prose (selectProseOrNudge returns
  // null), skip the parse entirely — it would be thrown away unrendered.
  const componentSet = getMarkdownComponents(interactive, mediaInset);
  const markdownNode =
    configNudge === null
      ? renderCachedMarkdown({
          channelNames,
          components: componentSet.components,
          content: processedContent,
          customEmoji,
          mentionNames,
          searchQuery,
          variant: componentSet.variant,
        })
      : null;

  return (
    <div
      className={cn(
        MESSAGE_MARKDOWN_CLASS,
        [
          "max-w-none wrap-anywhere text-sm leading-5 text-foreground",
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
      <MarkdownRuntimeContext.Provider value={runtime}>
        <VideoReviewMarkdownContext.Provider value={videoReviewContext}>
          {selectProseOrNudge(configNudge, markdownNode)}
          {configNudge !== null ? (
            <AttachmentGroup
              className="max-w-full flex-wrap overflow-visible pb-0"
              data-config-nudge=""
            >
              <ConfigNudgeCard nudge={configNudge} />
            </AttachmentGroup>
          ) : null}
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
      </MarkdownRuntimeContext.Provider>
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
    prev.mediaInset === next.mediaInset &&
    shallowRecordEqual(
      prev.agentMentionPubkeysByName,
      next.agentMentionPubkeysByName,
    ) &&
    shallowRecordEqual(prev.mentionPubkeysByName, next.mentionPubkeysByName) &&
    shallowArrayEqual(prev.mentionNames, next.mentionNames) &&
    shallowArrayEqual(prev.channelNames, next.channelNames) &&
    prev.imetaByUrl === next.imetaByUrl &&
    prev.configNudgeAuthorPubkey === next.configNudgeAuthorPubkey &&
    prev.searchQuery === next.searchQuery &&
    prev.snapshotSharedBy === next.snapshotSharedBy &&
    prev.videoReviewContext === next.videoReviewContext,
);
Markdown.displayName = "Markdown";
export { SyntaxHighlightedCode } from "./markdown/CodeBlock";
