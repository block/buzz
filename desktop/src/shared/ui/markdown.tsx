import * as React from "react";
import { createPortal } from "react-dom";
import type { Components } from "react-markdown";
import { ChevronLeft, ChevronRight, Download } from "lucide-react";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";

import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { requestOpenSnapshotImport } from "@/features/agents/openSnapshotImportFromUrlEvent";
import { parseChannelLink } from "@/features/messages/lib/channelLink";
import { isAudioAttachment } from "@/features/messages/lib/audioAttachment";
import {
  parseMessageLink,
  resolveMessageLinkRenderTarget,
  type ParsedMessageLink,
} from "@/features/messages/lib/messageLink";
import { renderAudioMessageAttachment } from "@/features/messages/ui/AudioMessageAttachment";
import { useChannelNavigation } from "@/shared/context/ChannelNavigationContext";
import { cn } from "@/shared/lib/cn";
import { parseEntityLink } from "@/shared/lib/entityLink";
import { parseSupportedLinkPreview } from "@/shared/lib/linkPreview";
import { rewriteRelayUrl } from "@/shared/lib/mediaUrl";
import { useRelayOrigin } from "@/shared/lib/useRelayOrigin";
import { AttachmentGroup } from "@/shared/ui/attachment";
import { ConfigNudgeCard } from "@/shared/ui/config-nudge-attachment";
import { MarkdownMention } from "./markdown/MarkdownMention";
import { LinkPreviewList } from "@/shared/ui/link-preview-list";
import { useSmoothCorners } from "@/shared/ui/smoothCorners";
import {
  computeConfigNudge,
  selectNudgeLeadingContent,
  selectProseOrNudge,
} from "@/shared/lib/computeConfigNudge";
import {
  INLINE_CODE_CHIP_CLASS,
  MESSAGE_MARKDOWN_CLASS,
} from "@/shared/ui/mentionChip";

import {
  classifyChildren,
  hasBlockMedia,
  isImageOnlyParagraph,
  markdownPropsAreEqual,
} from "./markdownUtils";
import { ImageMosaic } from "./markdown/ImageMosaic";
import { copyImageToClipboard, downloadImage } from "./markdown/imageActions";
import { ImageGalleryStatus } from "./markdown/ImageGalleryStatus";
import { ImageLightboxZoomControls } from "./markdown/ImageLightboxZoomControls";
import {
  CODE_BLOCK_CLASS,
  extractLanguage,
  MarkdownCodeBlock,
  SyntaxHighlightedCode,
} from "./markdown/CodeBlock";
import { EntityLinkAnchor, useOpenEntityLink } from "./markdown/entityLinks";
import { ExternalLinkAnchor } from "./markdown/ExternalLinkAnchor";
import { FileCard } from "./markdown/FileCard";
import {
  AuthoredDeepLinkAnchor,
  ChannelDeepLinkAnchor,
  MarkdownChannelDeepLink,
  MarkdownChannelReference,
} from "./markdown/ChannelDeepLink";
import { InlineEmojiPopover } from "./markdown/InlineEmojiPopover";
import { createLinkPreviewImageLightbox } from "./markdown/LinkPreviewImageLightbox";
import { MarkdownInput } from "./markdown/MarkdownInput";
import {
  MediaContextMenu,
  type MediaContextMenuPosition,
  useDismissMediaContextMenu,
} from "./markdown/MediaContextMenu";
import { isRelayDownloadable, isVideoMedia } from "./markdown/mediaEntry";
import {
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
  IMAGE_LIGHTBOX_MIN_ZOOM,
  IMAGE_LIGHTBOX_REDUCED_MOTION_MS,
  IMAGE_LIGHTBOX_TRACKPAD_ZOOM_IDLE_MS,
  IMAGE_LIGHTBOX_WHEEL_ZOOM_MAX_DELTA,
  IMAGE_LIGHTBOX_WHEEL_ZOOM_SPEED,
  IMAGE_LIGHTBOX_ZOOM_TRANSITION_MS,
  imageLightboxBasisBoxForItem,
  imageLightboxBoxFromRect,
  imageLightboxCornerRadiiFromElement,
  imageLightboxCornerRadiiStyle,
  imageLightboxExpandedCornerRadii,
  getImageLightboxFocusableElements,
  imageLightboxReturnTargetForItem,
  imageLightboxSourceScopeForTrigger,
  imageLightboxStyle,
  imageLightboxTargetBox,
  imageLightboxTransform,
  imageLightboxZoomBox,
  imageLightboxZoomStateAtPoint,
  imageLightboxZoomStateAtZoom,
  normalizedWheelDeltaY,
  visibleImageGalleryForTrigger,
} from "./markdown/imageLightbox";
import { MarkdownTable } from "./markdown/MarkdownTable";
import { ProgressiveImage } from "./markdown/ProgressiveImage";
import { MessageLinkPill } from "./markdown/MessageLinkPill";
import { renderCachedMarkdown } from "./markdown/nodeCache";
import { useMessageLinkPreviews } from "./markdown/useMessageLinkPreviews";
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

function ImageZoomOverlay({
  alt,
  galleryIndex = 0,
  galleryItems,
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
  const [{ zoom, zoomOffset }, setZoomState] = React.useState(() => ({
    zoom: IMAGE_LIGHTBOX_MIN_ZOOM,
    zoomOffset: { x: 0, y: 0 },
  }));
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

  const setClampedZoom = React.useCallback(
    (nextZoom: number) =>
      setZoomState((current) =>
        imageLightboxZoomStateAtZoom(current, nextZoom),
      ),
    [],
  );

  const updateZoom = React.useCallback(
    (updater: (zoom: number) => number) =>
      setZoomState((current) =>
        imageLightboxZoomStateAtZoom(current, updater(current.zoom)),
      ),
    [],
  );

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
      setZoomState({
        zoom: IMAGE_LIGHTBOX_MIN_ZOOM,
        zoomOffset: { x: 0, y: 0 },
      });
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
  const displayBox = imageLightboxZoomBox(targetBox, zoom, zoomOffset);
  const frameBox = isReturning ? returnBox : targetBox;
  const frameCornerRadii = isReturning
    ? returnCornerRadii
    : imageLightboxExpandedCornerRadii();
  const atRest =
    isOpen &&
    hasEntered &&
    zoom === IMAGE_LIGHTBOX_MIN_ZOOM &&
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
  const label = currentItem.alt?.trim() || "Image preview";
  const handleImageClick = React.useCallback(
    (event: React.MouseEvent<HTMLImageElement>) => {
      event.preventDefault();
      event.stopPropagation();
      if (!isOpen || isReturning) {
        return;
      }

      setIsAdjustingZoom(false);
      setZoomState((current) =>
        imageLightboxZoomStateAtPoint(targetBox, current, {
          x: event.clientX,
          y: event.clientY,
        }),
      );
    },
    [isOpen, isReturning, targetBox],
  );
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
    copyImageToClipboard(currentItem.src);
  }, [currentItem.src, markControlGesture]);
  const handleMenuDownload = React.useCallback(() => {
    setMenu(null);
    markControlGesture();
    downloadImage(currentItem.src);
  }, [currentItem.src, markControlGesture]);

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
          event.target instanceof Element &&
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
          event.target instanceof Element &&
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
        Full-size image preview. Press Escape or click outside the image to
        close. Click the image to zoom.
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
                  isReturning || zoom > IMAGE_LIGHTBOX_MIN_ZOOM
                    ? "cursor-zoom-out"
                    : "cursor-zoom-in",
                )}
                custom={galleryDirection}
                exit="exit"
                initial="enter"
                key={`${currentIndex}:${currentItem.resolvedSrc}`}
                src={currentItem.resolvedSrc}
                transition={{
                  duration: prefersReducedMotion
                    ? IMAGE_LIGHTBOX_REDUCED_MOTION_MS / 1000
                    : IMAGE_LIGHTBOX_GALLERY_SLIDE_MS / 1000,
                  ease: IMAGE_LIGHTBOX_GALLERY_EASE,
                }}
                variants={galleryImageVariants}
                onClick={handleImageClick}
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
              downloadImage(currentItem.src);
            }}
          >
            <Download className="h-4 w-4" />
          </button>
          <div
            aria-hidden="true"
            className="h-5 w-px shrink-0 bg-muted-foreground/15"
          />
          <ImageLightboxZoomControls
            markControlGesture={markControlGesture}
            setClampedZoom={setClampedZoom}
            setIsAdjustingZoom={setIsAdjustingZoom}
            updateZoom={updateZoom}
            zoom={zoom}
          />
          <ImageGalleryStatus {...{ currentIndex, itemCount: items.length }} />
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

export const LinkPreviewImageLightbox =
  createLinkPreviewImageLightbox(ImageZoomOverlay);

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
              trigger: triggerRef.current,
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
    copyImageToClipboard(copySrc);
  }, []);

  const handleDownload = React.useCallback(
    (downloadSrc: string | undefined) => {
      setMenu(null);
      downloadImage(downloadSrc);
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

export function createMarkdownComponents(
  interactive = true,
  mediaInset = false,
  blockCode = false,
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
      onOpenChannel,
      onOpenEntityLink,
      onOpenMessageLink,
      onImportSnapshotFromUrl,
      relayOrigin,
      resolveChannelReferences,
      snapshotSharedBy,
    } = useMarkdownRuntime();
    if (!interactive) {
      return <span className="font-medium text-current">{children}</span>;
    }
    if (hasBlockMedia(React.Children.toArray(children))) {
      return <>{children}</>;
    }

    const label = getReactNodeText(children);

    const audioAttachment = renderAudioMessageAttachment(
      href ? imetaByUrl?.get(href) : undefined,
      href,
      label,
      href && isRelayDownloadable(href, relayOrigin ?? undefined)
        ? href
        : undefined,
    );
    if (audioAttachment) return audioAttachment;

    // Classify verified agent/team snapshots before generic files.
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

    // Render non-media imeta links as download cards; media uses `img`.
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

    // Keep Buzz channel/message navigation in-app.
    if (href) {
      if (parseChannelLink(href).ok) {
        return (
          <ChannelDeepLinkAnchor
            {...props}
            href={href}
            interactive={interactive}
          >
            {children}
          </ChannelDeepLinkAnchor>
        );
      }
      const messageLinkTarget = resolveMessageLinkRenderTarget({
        href,
        label,
      });
      if (messageLinkTarget.kind !== "none") {
        if (messageLinkTarget.kind === "pill") {
          return (
            <MessageLinkPill
              channels={channels}
              interactive={interactive}
              link={messageLinkTarget.link}
              onOpenChannel={onOpenChannel}
              onOpenMessageLink={onOpenMessageLink}
              resolveChannelReference={resolveChannelReferences}
            />
          );
        }

        return (
          <AuthoredDeepLinkAnchor
            channelId={messageLinkTarget.link.channelId}
            href={href}
            interactive={interactive}
            messageLink={messageLinkTarget.link}
          >
            {children}
          </AuthoredDeepLinkAnchor>
        );
      }
      // Malformed message deep links fall through to external handling.
    }

    // `buzz://pr|issue|repo|project?…` entity links navigate in-app;
    // malformed ones fall through to the default anchor. The provider-backed
    // component keeps metadata tooltips available for both raw chips and
    // authored Markdown labels.
    if (href) {
      const entityAnchor = React.createElement(
        EntityLinkAnchor,
        {
          href,
          onOpenEntityLink,
          relayOrigin,
          interactive,
          asChip: label === href,
        },
        children,
      );
      if (
        parseEntityLink(href).ok ||
        parseSupportedLinkPreview(href, relayOrigin)?.href.startsWith("buzz://")
      )
        return entityAnchor;
    }

    const supportedLinkPreview = href
      ? parseSupportedLinkPreview(href, relayOrigin)
      : null;
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
    span: function MarkdownSpan({ children, node: _node, ...props }) {
      const { leadingInlineContent } = useMarkdownRuntime();
      if ("data-leading-inline-content" in props) {
        return <>{leadingInlineContent}</>;
      }
      return <span {...props}>{children}</span>;
    },
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
    p: function MarkdownParagraph({ children }) {
      const { imetaByUrl } = useMarkdownRuntime();
      // Detect media-only paragraphs (images + <br> from remarkBreaks).
      // Multi-image: render as a compact, count-aware mosaic. Two images split
      // a row, three form a hero-and-stack triptych, and larger odd counts let
      // the final image span both columns.
      // Single media: render as a plain <div> to avoid invalid <p><div> nesting
      // (the img component returns block-level wrappers for lightbox/video).
      const childArray = React.Children.toArray(children);
      const { imageChildren } = classifyChildren(childArray);
      const hasAudioAttachment = childArray.some(
        (child) =>
          React.isValidElement<{ href?: string }>(child) &&
          typeof child.props.href === "string" &&
          isAudioAttachment(imetaByUrl?.get(child.props.href)),
      );

      if (isImageOnlyParagraph(childArray)) {
        return <ImageMosaic>{imageChildren}</ImageMosaic>;
      }

      if (hasBlockMedia(childArray) || hasAudioAttachment) {
        return <div>{children}</div>;
      }

      return <p>{children}</p>;
    },
    pre: ({ children }) => {
      if (!interactive && !blockCode) return <span>{children}</span>;
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
      <td className="min-w-24 border-t border-border/70 px-3 py-2 align-top">
        {children}
      </td>
    ),
    th: ({ children }) => (
      <th className="min-w-24 bg-muted/60 px-3 py-2 align-top font-semibold text-foreground">
        {children}
      </th>
    ),
    ul: ({ children }) => (
      <ul className={cn("list-disc", listClassName)}>{children}</ul>
    ),
    mention: ({ children }: { children?: React.ReactNode }) => (
      <MarkdownMention interactive={interactive}>{children}</MarkdownMention>
    ),
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
    "channel-deep-link": ({ children }: { children?: React.ReactNode }) => (
      <MarkdownChannelDeepLink interactive={interactive}>
        {children}
      </MarkdownChannelDeepLink>
    ),
    "channel-link": ({ children }: { children?: React.ReactNode }) => (
      <MarkdownChannelReference interactive={interactive}>
        {children}
      </MarkdownChannelReference>
    ),
    "entity-link": function MarkdownEntityLink({
      children,
    }: {
      children?: React.ReactNode;
    }) {
      const { onOpenEntityLink, relayOrigin } = useMarkdownRuntime();
      const href = String(children ?? "");
      if (!parseEntityLink(href).ok)
        return <span data-entity-link="">{href}</span>;
      return React.createElement(
        EntityLinkAnchor,
        { href, interactive, onOpenEntityLink, relayOrigin },
        href,
      );
    },
    "message-link": function MarkdownMessageLink({
      children,
    }: {
      children?: React.ReactNode;
    }) {
      const runtime = useMarkdownRuntime();
      const { channels, onOpenChannel, onOpenMessageLink } = runtime;
      const href = String(children ?? "");
      const parsed = parseMessageLink(href);
      if (!parsed.ok) {
        // Malformed link: render the raw URL rather than a misleading pill.
        return <span data-message-link="">{href}</span>;
      }
      return (
        <MessageLinkPill
          channels={channels}
          interactive={interactive}
          link={parsed.value}
          onOpenChannel={onOpenChannel}
          onOpenMessageLink={onOpenMessageLink}
          resolveChannelReference={runtime.resolveChannelReferences}
        />
      );
    },
  } as Components;
}

/**
 * The component map only varies by the four boolean render flags, so at most
 * sixteen instances ever exist. Module-stable maps mean cached markdown
 * element trees (see ./markdown/nodeCache.ts) never embed per-mount closures.
 */
const MARKDOWN_COMPONENT_SCHEMA_VERSION = "8";
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
  leadingInlineContent: boolean,
  mediaInset: boolean,
  blockCode: boolean,
): MarkdownComponentSet {
  const variant = `${MARKDOWN_COMPONENT_SCHEMA_VERSION}:${interactive ? "i" : ""}${leadingInlineContent ? "l" : ""}${mediaInset ? "m" : ""}${blockCode ? "c" : ""}`;
  let entry = markdownComponentsByVariant.get(variant);
  if (!entry) {
    entry = {
      components: createMarkdownComponents(interactive, mediaInset, blockCode),
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
  hardLineBreaks = true,
  imetaByUrl,
  interactive = true,
  blockCode = false,
  agentMentionPubkeysByName,
  leadingInlineContent,
  mediaInset = false,
  messageId,
  linkPreviewsSuppressed = false,
  linkPreviewTags,
  onRemoveLinkPreviewsForEveryone,
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
  const onOpenEntityLink = useOpenEntityLink();
  const onOpenMessageLink = React.useCallback(
    (link: ParsedMessageLink) => {
      // Always route through `goChannel` with `messageId` set: the navigation
      // boundary guards every message-targeting caller before URL mutation.
      // `useAnchoredScroll` + `getEventById` backfill, and works for
      void goChannel(link.channelId, {
        messageId: link.messageId,
        threadRootId: link.threadRootId,
      });
    },
    [goChannel],
  );
  const relayOrigin = useRelayOrigin();
  const resolvedLinkPreviews = useMessageLinkPreviews({
    content,
    interactive,
    linkPreviewTags,
    linkPreviewsSuppressed,
    relayOrigin,
  });
  const configNudge = React.useMemo(
    () => computeConfigNudge(content, interactive, configNudgeAuthorPubkey),
    [content, interactive, configNudgeAuthorPubkey],
  );
  const runtime = React.useMemo<MarkdownRuntime>(
    () => ({
      agentMentionPubkeysByName,
      channels,
      imetaByUrl,
      leadingInlineContent,
      mentionPubkeysByName,
      onOpenChannel,
      onOpenEntityLink,
      onOpenMessageLink,
      relayOrigin,
      resolveChannelReferences: true,
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
      leadingInlineContent,
      mentionPubkeysByName,
      onOpenChannel,
      onOpenEntityLink,
      onOpenMessageLink,
      relayOrigin,
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

  // When a config-nudge suppresses the prose (selectProseOrNudge returns
  // null), skip the parse entirely — it would be thrown away unrendered.
  const hasLeadingInlineContent = leadingInlineContent != null;
  const componentSet = getMarkdownComponents(
    interactive,
    hasLeadingInlineContent,
    mediaInset,
    blockCode,
  );
  const markdownNode =
    configNudge === null
      ? renderCachedMarkdown({
          channelNames,
          components: componentSet.components,
          content: processedContent,
          customEmoji,
          hardLineBreaks,
          leadingInlineContent: hasLeadingInlineContent,
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
          "max-w-none wrap-anywhere text-message font-normal tracking-normal text-foreground",
          "[&>*:first-child]:mt-0 [&>*:last-child]:mb-0",
          "[&>*+*]:mt-3",
          "[&>p+p]:mt-conversation-paragraph [&>ol]:space-y-conversation-list [&>ul]:space-y-conversation-list",
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
              {selectNudgeLeadingContent(configNudge, leadingInlineContent)}
              <ConfigNudgeCard nudge={configNudge} />
            </AttachmentGroup>
          ) : null}
          <LinkPreviewList
            ImageLightbox={LinkPreviewImageLightbox}
            key={messageId}
            onRemoveForEveryone={onRemoveLinkPreviewsForEveryone}
            previews={resolvedLinkPreviews}
          />
        </VideoReviewMarkdownContext.Provider>
      </MarkdownRuntimeContext.Provider>
    </div>
  );
}

export const Markdown = React.memo(
  MarkdownInner,
  (prev, next) =>
    markdownPropsAreEqual(prev, next) &&
    prev.leadingInlineContent === next.leadingInlineContent,
);
Markdown.displayName = "Markdown";
export { SyntaxHighlightedCode } from "./markdown/CodeBlock";
