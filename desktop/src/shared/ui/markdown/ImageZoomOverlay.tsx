import * as React from "react";
import { createPortal } from "react-dom";
import { ChevronLeft, ChevronRight, Download } from "lucide-react";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";

import { cn } from "@/shared/lib/cn";
import { useSmoothCorners } from "@/shared/ui/smoothCorners";
import { copyImageToClipboard, downloadImage } from "./imageActions";
import { ImageGalleryStatus } from "./ImageGalleryStatus";
import { ImageLightboxZoomControls } from "./ImageLightboxZoomControls";
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
  imageLightboxCornerRadiiStyle,
  imageLightboxExpandedCornerRadii,
  getImageLightboxFocusableElements,
  imageLightboxReturnTargetForItem,
  imageLightboxStyle,
  imageLightboxTargetBox,
  imageLightboxTransform,
  imageLightboxZoomBox,
  imageLightboxZoomStateAtPoint,
  imageLightboxZoomStateAtZoom,
  normalizedWheelDeltaY,
} from "./imageLightbox";
import {
  MediaContextMenu,
  type MediaContextMenuPosition,
  useDismissMediaContextMenu,
} from "./MediaContextMenu";

type WebKitGestureLikeEvent = Event & {
  scale?: number;
};

export function ImageZoomOverlay({
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
