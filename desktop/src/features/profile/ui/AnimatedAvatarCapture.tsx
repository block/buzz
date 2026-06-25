import { Camera, Video } from "lucide-react";
import { motion } from "motion/react";
import * as React from "react";
import { createPortal } from "react-dom";

import {
  type AnimatedAvatarRecording,
  ANIMATED_AVATAR_FRAME_DELAY_MS,
  ANIMATED_AVATAR_SIZE,
  type AvatarCameraDevice,
  type AvatarComposition,
  buildPingPongAvatarFrames,
  composeAvatarFrame,
  composeAvatarFrames,
  createAvatarFrameBitmaps,
  DEFAULT_PERSON_OFFSET_X,
  DEFAULT_PERSON_OFFSET_Y,
  DEFAULT_PERSON_OUTLINE,
  DEFAULT_PERSON_SCALE,
  DEFAULT_SHAPE_OFFSET_X,
  DEFAULT_SHAPE_OFFSET_Y,
  DEFAULT_SHAPE_SCALE,
  encodeAvatarAnimation,
  MAX_PERSON_SCALE,
  MAX_SHAPE_SCALE,
  MIN_PERSON_SCALE,
  MIN_SHAPE_SCALE,
  listAvatarCameras,
  openAvatarCamera,
  preloadAvatarSegmenter,
  recordAnimatedAvatarFrames,
  renderAvatarPosterPng,
  stopAvatarCamera,
} from "@/features/profile/lib/animatedAvatarCapture";
import { AnimatedAvatarBackdropPanel } from "@/features/profile/ui/AnimatedAvatarBackdropPanel";
import { AnimatedAvatarCameraPicker } from "@/features/profile/ui/AnimatedAvatarCameraPicker";
import {
  AvatarFilmstripPicker,
  AvatarFramingSlider,
  AvatarOutlineToggle,
} from "@/features/profile/ui/AnimatedAvatarControls";
import {
  buildFilmstripFrames,
  cameraLabelsAreAvailable,
  type CapturePhase,
  type CameraSource,
  clampFrameIndex,
  clampOffset,
  defaultPersonScaleForSource,
  ENTRANCE_TRANSITION,
  PERSON_SIZE_TIP,
  preferredCameraDevice,
  randomBackdropColor,
  RECORD_SECONDS,
} from "@/features/profile/ui/AnimatedAvatarCapture.helpers";
import {
  AnimatedAvatarReviewNav,
  type ReviewSection,
} from "@/features/profile/ui/AnimatedAvatarReviewNav";
import { AvatarCustomColorPanel } from "@/features/profile/ui/AvatarCustomColorPanel";
import {
  AVATAR_COLORS,
  hexToHsv,
  hsvToHex,
  normalizeHue,
} from "@/features/profile/ui/ProfileAvatarEditor.utils";
import { uploadMediaBytes } from "@/shared/api/tauri";
import { buildAnimatedAvatarUrl } from "@/shared/lib/animatedAvatar";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import { Spinner } from "@/shared/ui/spinner";

type AnimatedAvatarCaptureProps = {
  disabled?: boolean;
  testIdPrefix: string;
  /** Receives the composed animated avatar URL after upload. */
  onApply: (avatarUrl: string) => void;
  /**
   * Host element inside the page's main avatar preview. When provided, the
   * camera feed, recording ring, and composed preview render there (via a
   * portal) instead of inside the tab — so edits show exactly where the
   * avatar will live. Pair with `onPreviewActiveChange` so the host can
   * hide its regular preview while the capture content is showing.
   */
  previewContainer?: HTMLElement | null;
  onPreviewActiveChange?: (active: boolean) => void;
  onPreviewCaptionChange?: (caption: string | null) => void;
  onApplyPendingChange?: (isPending: boolean) => void;
  onCustomColorPickerOpenChange?: (isOpen: boolean) => void;
  /**
   * Receives the current apply function (or null when there is nothing to
   * apply) so the host's Done button can upload-and-apply in one step.
   */
  registerApply?: (apply: (() => Promise<boolean>) | null) => void;
  /** Show the in-tab "Use as avatar" button (hosts without a Done button). */
  showApplyButton?: boolean;
};

export function AnimatedAvatarCapture({
  disabled = false,
  testIdPrefix,
  onApply,
  onApplyPendingChange,
  onCustomColorPickerOpenChange,
  previewContainer = null,
  onPreviewActiveChange,
  onPreviewCaptionChange,
  registerApply,
  showApplyButton = true,
}: AnimatedAvatarCaptureProps) {
  const [phase, setPhase] = React.useState<CapturePhase>("idle");
  const [errorMessage, setErrorMessage] = React.useState<string | null>(null);
  const [cameraDevices, setCameraDevices] = React.useState<
    AvatarCameraDevice[]
  >([]);
  const [selectedCameraSource, setSelectedCameraSource] =
    React.useState<CameraSource | null>(null);
  const [selectedCameraId, setSelectedCameraId] = React.useState<string | null>(
    null,
  );
  const [recordProgress, setRecordProgress] = React.useState(0);
  const [recording, setRecording] =
    React.useState<AnimatedAvatarRecording | null>(null);
  const [bitmaps, setBitmaps] = React.useState<ImageBitmap[]>([]);
  const [filmstripFrames, setFilmstripFrames] = React.useState<string[]>([]);
  const [posterIndex, setPosterIndex] = React.useState(0);
  const [backdropColor, setBackdropColor] = React.useState<string | null>(
    randomBackdropColor,
  );
  const [personOffset, setPersonOffset] = React.useState({
    x: DEFAULT_PERSON_OFFSET_X,
    y: DEFAULT_PERSON_OFFSET_Y,
  });
  const [personScale, setPersonScale] = React.useState(DEFAULT_PERSON_SCALE);
  const [personOutline, setPersonOutline] = React.useState(
    DEFAULT_PERSON_OUTLINE,
  );
  const [shapeOffset, setShapeOffset] = React.useState({
    x: DEFAULT_SHAPE_OFFSET_X,
    y: DEFAULT_SHAPE_OFFSET_Y,
  });
  const [shapeScale, setShapeScale] = React.useState(DEFAULT_SHAPE_SCALE);
  const [activeSection, setActiveSection] =
    React.useState<ReviewSection>("person");
  const [isPreviewPlaying, setIsPreviewPlaying] = React.useState(false);
  const [isDraggingPerson, setIsDraggingPerson] = React.useState(false);
  const [isSaving, setIsSaving] = React.useState(false);

  // Drag, arrow keys, and the size slider adjust whichever framing section
  // is open; the color/poster sections leave the person active.
  const editTarget = activeSection === "shape" ? "shape" : "person";
  const activeOffset = editTarget === "shape" ? shapeOffset : personOffset;
  const setActiveOffset =
    editTarget === "shape" ? setShapeOffset : setPersonOffset;
  const activeScale = editTarget === "shape" ? shapeScale : personScale;
  const setActiveScale =
    editTarget === "shape" ? setShapeScale : setPersonScale;
  const activeScaleMin =
    editTarget === "shape" ? MIN_SHAPE_SCALE : MIN_PERSON_SCALE;
  const activeScaleMax =
    editTarget === "shape" ? MAX_SHAPE_SCALE : MAX_PERSON_SCALE;
  const activeScaleReset =
    editTarget === "shape" ? DEFAULT_SHAPE_SCALE : DEFAULT_PERSON_SCALE;

  const resetActiveFraming = React.useCallback(() => {
    if (activeSection === "shape") {
      setShapeOffset({ x: DEFAULT_SHAPE_OFFSET_X, y: DEFAULT_SHAPE_OFFSET_Y });
      setShapeScale(DEFAULT_SHAPE_SCALE);
      return;
    }
    setPersonOffset({
      x: DEFAULT_PERSON_OFFSET_X,
      y: DEFAULT_PERSON_OFFSET_Y,
    });
    setPersonScale(DEFAULT_PERSON_SCALE);
  }, [activeSection]);

  const resetAllFraming = React.useCallback(() => {
    setPersonOffset({
      x: DEFAULT_PERSON_OFFSET_X,
      y: DEFAULT_PERSON_OFFSET_Y,
    });
    setPersonScale(DEFAULT_PERSON_SCALE);
    setShapeOffset({ x: DEFAULT_SHAPE_OFFSET_X, y: DEFAULT_SHAPE_OFFSET_Y });
    setShapeScale(DEFAULT_SHAPE_SCALE);
  }, []);

  // Custom backdrop color picker (shared HSV panel).
  const [isCustomPickerOpen, setIsCustomPickerOpen] = React.useState(false);
  const [customHue, setCustomHue] = React.useState(210);
  const [customSaturation, setCustomSaturation] = React.useState(80);
  const [customValue, setCustomValue] = React.useState(90);
  const customColorDraft = React.useMemo(
    () => hsvToHex(customHue, customSaturation, customValue),
    [customHue, customSaturation, customValue],
  );
  const isCustomPickerVisible = isCustomPickerOpen && phase === "review";
  const visibleBackdropColor = isCustomPickerVisible
    ? customColorDraft
    : backdropColor;

  const composition = React.useMemo<AvatarComposition>(
    () => ({
      backdropColor: visibleBackdropColor,
      offsetX: personOffset.x,
      offsetY: personOffset.y,
      personOutline,
      scale: personScale,
      shapeOffsetX: shapeOffset.x,
      shapeOffsetY: shapeOffset.y,
      shapeScale,
    }),
    [
      visibleBackdropColor,
      personOffset,
      personOutline,
      personScale,
      shapeOffset,
      shapeScale,
    ],
  );

  const videoRef = React.useRef<HTMLVideoElement | null>(null);
  const previewCanvasRef = React.useRef<HTMLCanvasElement | null>(null);
  const streamRef = React.useRef<MediaStream | null>(null);
  const activeStreamSourceRef = React.useRef<CameraSource | null>(null);
  const recordAbortRef = React.useRef<AbortController | null>(null);
  const dragStateRef = React.useRef<{
    pointerId: number;
    startClientX: number;
    startClientY: number;
    baseOffsetX: number;
    baseOffsetY: number;
  } | null>(null);

  const releaseCamera = React.useCallback(() => {
    if (streamRef.current) {
      stopAvatarCamera(streamRef.current);
      streamRef.current = null;
    }
    activeStreamSourceRef.current = null;
    if (videoRef.current) {
      videoRef.current.srcObject = null;
    }
  }, []);

  const releaseBitmaps = React.useCallback(() => {
    setBitmaps((previous) => {
      for (const bitmap of previous) {
        bitmap.close();
      }
      return [];
    });
  }, []);

  const refreshCameraDevices = React.useCallback(async () => {
    try {
      const devices = await listAvatarCameras();
      setCameraDevices(devices);
      setSelectedCameraId((current) => {
        if (current && devices.some((device) => device.deviceId === current)) {
          return current;
        }
        return selectedCameraSource
          ? (preferredCameraDevice(devices, selectedCameraSource)?.deviceId ??
              null)
          : null;
      });
    } catch {
      setCameraDevices([]);
      setSelectedCameraId(null);
    }
  }, [selectedCameraSource]);

  React.useEffect(() => {
    return () => {
      recordAbortRef.current?.abort();
      releaseCamera();
    };
  }, [releaseCamera]);

  // Close bitmaps on unmount only — phase changes manage them explicitly.
  React.useEffect(() => releaseBitmaps, [releaseBitmaps]);

  React.useEffect(() => {
    void refreshCameraDevices();
    const mediaDevices = navigator.mediaDevices;
    if (!mediaDevices?.addEventListener) {
      return;
    }
    const handleDeviceChange = () => {
      void refreshCameraDevices();
    };
    mediaDevices.addEventListener("devicechange", handleDeviceChange);
    return () => {
      mediaDevices.removeEventListener("devicechange", handleDeviceChange);
    };
  }, [refreshCameraDevices]);

  // Review preview mirrors the final avatar: the selected poster frame is
  // shown as a still, and hovering plays the animation.
  React.useEffect(() => {
    if (phase !== "review" || bitmaps.length === 0) {
      return;
    }
    const canvas = previewCanvasRef.current;
    const context = canvas?.getContext("2d");
    if (!context) {
      return;
    }

    if (!isPreviewPlaying) {
      const poster = bitmaps[Math.min(posterIndex, bitmaps.length - 1)];
      if (poster) {
        composeAvatarFrame(context, poster, composition);
      }
      return;
    }

    const previewFrames = buildPingPongAvatarFrames(bitmaps);
    let frameIndex = 0;
    let lastDrawn = 0;
    let animationFrame = 0;
    const tick = (now: number) => {
      if (now - lastDrawn >= ANIMATED_AVATAR_FRAME_DELAY_MS) {
        const bitmap = previewFrames[frameIndex];
        if (bitmap) {
          composeAvatarFrame(context, bitmap, composition);
        }
        frameIndex = (frameIndex + 1) % previewFrames.length;
        lastDrawn = now;
      }
      animationFrame = requestAnimationFrame(tick);
    };
    animationFrame = requestAnimationFrame(tick);

    return () => cancelAnimationFrame(animationFrame);
  }, [phase, bitmaps, composition, isPreviewPlaying, posterIndex]);

  // Filmstrip frames track the composition. Debounced so dragging or
  // scrubbing the size slider doesn't re-render the strip per tick.
  React.useEffect(() => {
    if (phase !== "review" || bitmaps.length === 0) {
      setFilmstripFrames([]);
      return;
    }
    const handle = setTimeout(() => {
      setFilmstripFrames(buildFilmstripFrames(bitmaps, composition));
    }, 200);
    return () => clearTimeout(handle);
  }, [phase, bitmaps, composition]);

  const startCamera = React.useCallback(
    async (cameraId = selectedCameraId, source = selectedCameraSource) => {
      setErrorMessage(null);
      setPhase("starting");
      releaseCamera();
      // Warm the segmentation model while the user lines up their shot.
      preloadAvatarSegmenter();
      try {
        let stream = await openAvatarCamera(cameraId || null);
        let resolvedCameraId = cameraId ?? null;
        if (source === "iphone" && !cameraId) {
          const devicesAfterPermission = await listAvatarCameras().catch(
            () => [],
          );
          setCameraDevices(devicesAfterPermission);
          const preferredIphone = preferredCameraDevice(
            devicesAfterPermission,
            "iphone",
          );
          if (!preferredIphone) {
            stopAvatarCamera(stream);
            setSelectedCameraSource(null);
            setSelectedCameraId(null);
            setErrorMessage(
              "Could not find an iPhone camera. Make sure Continuity Camera is available, then try again.",
            );
            setPhase("idle");
            return;
          }

          resolvedCameraId = preferredIphone.deviceId;
          const activeDeviceId =
            stream.getVideoTracks()[0]?.getSettings().deviceId ?? null;
          if (activeDeviceId !== preferredIphone.deviceId) {
            stopAvatarCamera(stream);
            stream = await openAvatarCamera(preferredIphone.deviceId);
          }
        }
        streamRef.current = stream;
        activeStreamSourceRef.current = source;
        setSelectedCameraId(resolvedCameraId);
        const video = videoRef.current;
        if (!video) {
          stopAvatarCamera(stream);
          activeStreamSourceRef.current = null;
          setPhase("idle");
          return;
        }
        video.srcObject = stream;
        await video.play();
        setPhase("live");
        void refreshCameraDevices();
      } catch {
        releaseCamera();
        setErrorMessage(
          "Could not access the camera. Check Buzz's camera permission and try again.",
        );
        setPhase("idle");
      }
    },
    [
      refreshCameraDevices,
      releaseCamera,
      selectedCameraId,
      selectedCameraSource,
    ],
  );

  const selectCameraSource = React.useCallback(
    (source: CameraSource) => {
      const cameraId =
        preferredCameraDevice(cameraDevices, source)?.deviceId ?? null;
      const hasDeviceLabels = cameraLabelsAreAvailable(cameraDevices);
      if (source === "iphone" && !cameraId && hasDeviceLabels) {
        return;
      }
      setSelectedCameraSource(source);
      setSelectedCameraId(cameraId);
      if (phase === "idle" || phase === "live") {
        void startCamera(cameraId, source);
      }
    },
    [cameraDevices, phase, startCamera],
  );

  const record = React.useCallback(async () => {
    const video = videoRef.current;
    if (!video || phase !== "live") {
      return;
    }

    setErrorMessage(null);
    setRecordProgress(0);
    setPhase("recording");
    const abort = new AbortController();
    recordAbortRef.current = abort;

    try {
      const captured = await recordAnimatedAvatarFrames(video, {
        onProgress: setRecordProgress,
        signal: abort.signal,
      });
      const recordingSource =
        activeStreamSourceRef.current ?? selectedCameraSource;
      setPhase("processing");
      releaseCamera();
      const nextBitmaps = await createAvatarFrameBitmaps(captured.frames);
      releaseBitmaps();
      setBitmaps(nextBitmaps);
      setPosterIndex(0);
      setRecording(captured);
      setPersonOffset({
        x: DEFAULT_PERSON_OFFSET_X,
        y: DEFAULT_PERSON_OFFSET_Y,
      });
      setPersonScale(defaultPersonScaleForSource(recordingSource));
      setPhase("review");
    } catch (error) {
      if (error instanceof DOMException && error.name === "AbortError") {
        return;
      }
      setErrorMessage(
        error instanceof Error ? error.message : "Recording failed. Try again.",
      );
      releaseCamera();
      setPhase("idle");
    } finally {
      recordAbortRef.current = null;
    }
  }, [phase, releaseBitmaps, releaseCamera, selectedCameraSource]);

  const retake = React.useCallback(() => {
    setRecording(null);
    setFilmstripFrames([]);
    setIsCustomPickerOpen(false);
    setIsPreviewPlaying(false);
    setActiveSection("person");
    resetAllFraming();
    releaseBitmaps();
    void startCamera(selectedCameraId, selectedCameraSource);
  }, [
    releaseBitmaps,
    resetAllFraming,
    selectedCameraId,
    selectedCameraSource,
    startCamera,
  ]);

  const apply = React.useCallback(async (): Promise<boolean> => {
    if (bitmaps.length === 0 || isSaving) {
      return false;
    }

    setIsSaving(true);
    setErrorMessage(null);
    try {
      const composed = composeAvatarFrames(bitmaps, composition);
      const posterFrame = composed[posterIndex] ?? composed[0];
      if (!posterFrame) {
        throw new Error("No frames were recorded.");
      }
      const animationBytes = encodeAvatarAnimation(composed);
      const posterBytes = await renderAvatarPosterPng(posterFrame);
      const [animationUpload, posterUpload] = await Promise.all([
        uploadMediaBytes([...animationBytes], "animated-avatar.png"),
        uploadMediaBytes([...posterBytes], "animated-avatar-poster.png"),
      ]);
      if (
        !animationUpload.type.startsWith("image/") ||
        !posterUpload.type.startsWith("image/")
      ) {
        setErrorMessage("The relay rejected the recording. Try again.");
        return false;
      }
      onApply(buildAnimatedAvatarUrl(posterUpload.url, animationUpload.url));
      return true;
    } catch (error) {
      setErrorMessage(
        error instanceof Error
          ? error.message
          : "Could not upload the animated avatar.",
      );
      return false;
    } finally {
      setIsSaving(false);
    }
  }, [bitmaps, composition, isSaving, onApply, posterIndex]);

  // Hand the host's Done button the current apply function whenever a
  // recording is ready to upload.
  React.useEffect(() => {
    if (!registerApply) {
      return;
    }
    registerApply(phase === "review" && bitmaps.length > 0 ? apply : null);
    return () => registerApply(null);
  }, [apply, bitmaps.length, phase, registerApply]);

  const openCustomPicker = React.useCallback(() => {
    const baseColor = hexToHsv(backdropColor ?? customColorDraft);
    setCustomHue(normalizeHue(baseColor.hue));
    setCustomSaturation(baseColor.saturation);
    setCustomValue(baseColor.value);
    setIsCustomPickerOpen(true);
  }, [backdropColor, customColorDraft]);

  const isCameraVisible = phase === "live" || phase === "recording";
  const isCameraStageVisible =
    phase === "starting" || phase === "live" || phase === "recording";
  const computerCamera = React.useMemo(
    () => preferredCameraDevice(cameraDevices, "computer"),
    [cameraDevices],
  );
  const iphoneCamera = React.useMemo(
    () => preferredCameraDevice(cameraDevices, "iphone"),
    [cameraDevices],
  );
  const hasCameraLabels = cameraLabelsAreAvailable(cameraDevices);
  const activeCameraSource = selectedCameraSource;
  const isCustomBackdropSelected =
    backdropColor !== null &&
    !AVATAR_COLORS.some(
      (color) => color.toUpperCase() === backdropColor.toUpperCase(),
    );
  const isFramingSection =
    activeSection === "person" || activeSection === "shape";

  // With a host preview container, the camera feed / recording ring /
  // composed preview render inside the page's main avatar preview (via a
  // portal) so edits show exactly where the avatar will live.
  const usePortal = previewContainer !== null;
  const reviewWarning =
    phase === "review" && recording && !recording.backgroundRemoved
      ? "Background removal model couldn't be loaded, so the background was kept. Retake while online to remove it."
      : null;
  const captureHelpText =
    phase === "idle"
      ? null
      : phase === "starting"
        ? null
        : phase === "live"
          ? "Line up your shot."
          : phase === "recording"
            ? "Recording... hold still-ish."
            : phase === "processing"
              ? "Cutting you out of the background..."
              : null;
  const previewCaption =
    usePortal && (phase === "live" || phase === "recording")
      ? captureHelpText
      : usePortal && phase === "review"
        ? "Hover to play"
        : null;
  const inlineCaptureHelpText =
    usePortal && (phase === "live" || phase === "recording")
      ? null
      : captureHelpText;
  const showCaptureCard = !usePortal && phase !== "review";
  const showCameraPicker = ["idle", "starting", "live"].includes(phase);

  React.useEffect(() => {
    onPreviewActiveChange?.(usePortal);
    return () => onPreviewActiveChange?.(false);
  }, [onPreviewActiveChange, usePortal]);

  React.useLayoutEffect(() => {
    onApplyPendingChange?.(isSaving);
    return () => onApplyPendingChange?.(false);
  }, [isSaving, onApplyPendingChange]);

  React.useEffect(() => {
    onPreviewCaptionChange?.(previewCaption);

    return () => onPreviewCaptionChange?.(null);
  }, [onPreviewCaptionChange, previewCaption]);

  React.useLayoutEffect(() => {
    onCustomColorPickerOpenChange?.(isCustomPickerVisible);

    return () => {
      onCustomColorPickerOpenChange?.(false);
    };
  }, [isCustomPickerVisible, onCustomColorPickerOpenChange]);

  const stageContent = (
    <div className={cn("relative", usePortal ? "h-full w-full" : "h-44 w-44")}>
      {/* Live camera preview — kept mounted so the stream can attach. */}
      <div
        className={cn(
          "h-full w-full overflow-hidden rounded-full bg-background/60 shadow-inner",
          !isCameraStageVisible && "hidden",
        )}
      >
        <video
          autoPlay
          className={cn(
            "h-full w-full -scale-x-100 object-cover",
            !isCameraVisible && "opacity-0",
          )}
          data-testid={`${testIdPrefix}-animated-preview`}
          muted
          playsInline
          ref={videoRef}
        />
      </div>

      {/* Recording timer: a stroke that sweeps around the capture circle. */}
      {phase === "recording" ? (
        <svg
          aria-hidden="true"
          className="pointer-events-none absolute -inset-1 h-[calc(100%+8px)] w-[calc(100%+8px)] -rotate-90"
          data-testid={`${testIdPrefix}-animated-timer-ring`}
          viewBox="0 0 100 100"
        >
          <circle
            className="stroke-foreground"
            cx="50"
            cy="50"
            fill="none"
            pathLength={100}
            r="48"
            strokeDasharray={100}
            strokeDashoffset={100 - recordProgress * 100}
            strokeLinecap="round"
            strokeWidth={2.5}
          />
        </svg>
      ) : null}

      {/* Review preview: circle-cropped like the real avatar, shows the
              selected poster frame as a still, and plays the composed
              animation on hover — exactly how the avatar behaves in the app.
              Dragging repositions the active framing target. */}
      <div
        aria-label="Avatar preview — drag or use arrow keys to position"
        className={cn(
          // Transparent like the real avatar container — only a faint
          // ring marks the circular crop boundary. pointer-events-auto
          // re-enables interaction inside the host's inert overlay when
          // portaled.
          "pointer-events-auto h-full w-full touch-none overflow-hidden rounded-full ring-1 ring-foreground/10 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
          isDraggingPerson ? "cursor-grabbing" : "cursor-grab",
          phase !== "review" && "hidden",
        )}
        onKeyDown={(event) => {
          if (phase !== "review" || disabled || isSaving) {
            return;
          }
          const step = event.shiftKey ? 16 : 4;
          const moves: Record<string, [number, number]> = {
            ArrowDown: [0, step],
            ArrowLeft: [-step, 0],
            ArrowRight: [step, 0],
            ArrowUp: [0, -step],
          };
          const move = moves[event.key];
          if (!move) {
            return;
          }
          event.preventDefault();
          setActiveOffset((previous) => ({
            x: clampOffset(previous.x + move[0]),
            y: clampOffset(previous.y + move[1]),
          }));
        }}
        onMouseEnter={() => setIsPreviewPlaying(true)}
        onMouseLeave={() => setIsPreviewPlaying(false)}
        onPointerCancel={() => {
          dragStateRef.current = null;
          setIsDraggingPerson(false);
        }}
        onPointerDown={(event) => {
          if (phase !== "review" || disabled || isSaving) {
            return;
          }
          event.preventDefault();
          event.currentTarget.setPointerCapture(event.pointerId);
          dragStateRef.current = {
            baseOffsetX: activeOffset.x,
            baseOffsetY: activeOffset.y,
            pointerId: event.pointerId,
            startClientX: event.clientX,
            startClientY: event.clientY,
          };
          setIsDraggingPerson(true);
        }}
        onPointerMove={(event) => {
          const drag = dragStateRef.current;
          if (!drag || drag.pointerId !== event.pointerId) {
            return;
          }
          const rect = event.currentTarget.getBoundingClientRect();
          const toFrame = ANIMATED_AVATAR_SIZE / Math.max(rect.width, 1);
          setActiveOffset({
            x: clampOffset(
              drag.baseOffsetX + (event.clientX - drag.startClientX) * toFrame,
            ),
            y: clampOffset(
              drag.baseOffsetY + (event.clientY - drag.startClientY) * toFrame,
            ),
          });
        }}
        onPointerUp={() => {
          dragStateRef.current = null;
          setIsDraggingPerson(false);
        }}
        role="application"
        tabIndex={phase === "review" ? 0 : -1}
      >
        <canvas
          className="pointer-events-none h-full w-full"
          data-playing={isPreviewPlaying ? "true" : undefined}
          data-testid={`${testIdPrefix}-animated-review-preview`}
          height={ANIMATED_AVATAR_SIZE}
          ref={previewCanvasRef}
          width={ANIMATED_AVATAR_SIZE}
        />
      </div>

      {phase === "idle" ? (
        <div className="grid h-full w-full place-items-center rounded-full border-2 border-dashed border-border bg-background text-primary shadow-xs">
          <Camera className="h-10 w-10" />
        </div>
      ) : phase === "starting" ? (
        <div className="absolute inset-0 grid place-items-center rounded-full bg-background/70 text-center shadow-inner">
          <div className="grid justify-items-center gap-2 px-4">
            <Spinner aria-label="Starting camera" className="h-4 w-4" />
            <span className="text-xs font-medium text-muted-foreground">
              Starting camera
            </span>
          </div>
        </div>
      ) : phase === "processing" ? (
        <div className="grid h-full w-full place-items-center rounded-full bg-background/60 shadow-inner">
          <Spinner aria-label="Processing recording" className="h-6 w-6" />
        </div>
      ) : null}
    </div>
  );

  return (
    <div
      className={cn(
        "relative grid content-start",
        phase === "review" ? "gap-7 pb-9 pt-2" : "gap-4 pb-5",
        phase === "review" && !showApplyButton && "mb-5",
        isCustomPickerVisible && "min-h-[504px]",
      )}
      data-testid={`${testIdPrefix}-animated`}
    >
      {usePortal && previewContainer
        ? createPortal(stageContent, previewContainer)
        : null}

      {showCaptureCard ? (
        <div className="relative grid place-items-center rounded-xl bg-muted px-4 py-6">
          {usePortal ? null : stageContent}

          {inlineCaptureHelpText ? (
            <p
              className={cn(
                "text-center text-sm text-muted-foreground",
                !usePortal && "mt-4",
              )}
            >
              {inlineCaptureHelpText}
            </p>
          ) : null}

          {reviewWarning ? (
            <p className="mt-2 text-center text-xs text-muted-foreground">
              {reviewWarning}
            </p>
          ) : null}
        </div>
      ) : null}

      {!showCaptureCard && reviewWarning ? (
        <p className="rounded-xl bg-muted px-4 py-3 text-center text-xs text-muted-foreground">
          {reviewWarning}
        </p>
      ) : null}

      {phase === "review" ? (
        <AnimatedAvatarReviewNav
          activeSection={activeSection}
          disabled={disabled}
          isSaving={isSaving}
          onRetake={retake}
          onSectionChange={setActiveSection}
          testIdPrefix={testIdPrefix}
        />
      ) : null}

      {phase === "review" && isFramingSection ? (
        <div className="flex items-start gap-2">
          <AvatarFramingSlider
            disabled={disabled || isSaving}
            max={Math.round(activeScaleMax * 100)}
            min={Math.round(activeScaleMin * 100)}
            onChange={(value) => {
              if (activeSection === "shape" && value > 0) {
                setBackdropColor((current) => current ?? randomBackdropColor());
              }
              setActiveScale(value / 100);
            }}
            onReset={resetActiveFraming}
            resetValue={Math.round(activeScaleReset * 100)}
            resetTestId={`${testIdPrefix}-animated-reset-framing`}
            testId={`${testIdPrefix}-animated-size`}
            tipText={activeSection === "person" ? PERSON_SIZE_TIP : null}
            value={Math.round(activeScale * 100)}
          />
          {activeSection === "person" ? (
            <AvatarOutlineToggle
              disabled={disabled || isSaving}
              enabled={personOutline}
              onChange={setPersonOutline}
              testIdPrefix={testIdPrefix}
            />
          ) : null}
        </div>
      ) : null}

      {phase === "review" && activeSection === "color" ? (
        <AnimatedAvatarBackdropPanel
          backdropColor={backdropColor}
          disabled={disabled}
          isCustomBackdropSelected={isCustomBackdropSelected}
          isSaving={isSaving}
          onOpenCustomPicker={openCustomPicker}
          onSelectColor={setBackdropColor}
          testIdPrefix={testIdPrefix}
        />
      ) : null}

      {phase === "review" && activeSection === "poster" ? (
        <AvatarFilmstripPicker
          disabled={disabled || isSaving}
          frameCount={bitmaps.length}
          frames={filmstripFrames}
          helpTestId={`${testIdPrefix}-animated-review-help`}
          helpText="Pick the still shown before hover."
          onSelectFrame={(index) =>
            setPosterIndex(clampFrameIndex(index, bitmaps.length))
          }
          selectedFrame={posterIndex}
          testIdPrefix={testIdPrefix}
        />
      ) : null}

      {showCameraPicker ? (
        <div className="grid gap-4">
          <AnimatedAvatarCameraPicker
            activeCameraSource={activeCameraSource}
            computerDisabled={cameraDevices.length > 0 && !computerCamera}
            disabled={disabled || phase === "starting"}
            iphoneDisabled={
              cameraDevices.length > 0 && !iphoneCamera && hasCameraLabels
            }
            onSelectSource={selectCameraSource}
            testIdPrefix={testIdPrefix}
          />
          {usePortal && inlineCaptureHelpText ? (
            <p className="px-1 text-center text-sm text-muted-foreground">
              {inlineCaptureHelpText}
            </p>
          ) : null}
          <div className="h-14 pt-2">
            {phase === "live" ? (
              <Button
                asChild
                className="h-12 w-full rounded-xl"
                data-testid={`${testIdPrefix}-animated-record`}
                disabled={disabled}
                onClick={() => void record()}
                type="button"
              >
                <motion.button
                  animate={{ opacity: 1 }}
                  initial={{ opacity: 0 }}
                  transition={ENTRANCE_TRANSITION}
                >
                  <Video aria-hidden="true" className="mr-2 h-4 w-4" />
                  Record {RECORD_SECONDS} seconds
                </motion.button>
              </Button>
            ) : null}
          </div>
        </div>
      ) : usePortal && inlineCaptureHelpText ? (
        <p className="px-1 text-center text-sm text-muted-foreground">
          {inlineCaptureHelpText}
        </p>
      ) : null}

      {phase === "review" && showApplyButton ? (
        <Button
          className="h-12 w-full rounded-xl"
          data-testid={`${testIdPrefix}-animated-apply`}
          disabled={disabled || isSaving}
          onClick={() => void apply()}
          type="button"
        >
          {isSaving ? (
            <Spinner
              aria-label="Uploading animated avatar"
              className="h-4 w-4 border-2"
            />
          ) : (
            "Use as avatar"
          )}
        </Button>
      ) : null}

      {errorMessage ? (
        <p
          className="rounded-xl border border-destructive/30 bg-destructive/10 px-4 py-3 text-sm font-medium text-destructive"
          data-testid={`${testIdPrefix}-animated-error`}
          role="alert"
        >
          {errorMessage}
        </p>
      ) : null}

      <AvatarCustomColorPanel
        colorDraft={customColorDraft}
        hue={customHue}
        onCommit={() => {
          setBackdropColor(customColorDraft);
          setIsCustomPickerOpen(false);
        }}
        onHueChange={setCustomHue}
        onSaturationValueChange={(nextSaturation, nextValue) => {
          setCustomSaturation(nextSaturation);
          setCustomValue(nextValue);
        }}
        saturation={customSaturation}
        className="h-[504px]"
        testIdPrefix={`${testIdPrefix}-animated`}
        value={customValue}
        visible={isCustomPickerVisible}
      />
    </div>
  );
}
