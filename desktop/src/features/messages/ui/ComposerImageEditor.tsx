import * as React from "react";
import { Loader2, Redo2, Undo2 } from "lucide-react";

import { fetchMediaBytes } from "@/shared/api/tauriMedia";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";

type EditorPoint = { x: number; y: number };

/** A committed pen stroke, in natural-image pixel coordinates. */
type EditorStroke = {
  color: string;
  points: EditorPoint[];
  /** Line width in natural-image pixels (already scaled from CSS px). */
  width: number;
};

const PEN_COLORS = [
  { label: "Red", value: "#ef4444" },
  { label: "Yellow", value: "#f59e0b" },
  { label: "Green", value: "#22c55e" },
  { label: "Blue", value: "#3b82f6" },
  { label: "White", value: "#ffffff" },
  { label: "Black", value: "#111111" },
] as const;

/** Pen stroke width range, in CSS pixels: five whole-pixel slider stops. */
const PEN_WIDTH_MIN_CSS = 4;
const PEN_WIDTH_MAX_CSS = 12;
const PEN_WIDTH_STEP_CSS = 2;
const PEN_WIDTH_DEFAULT_CSS = 6;

function drawStroke(ctx: CanvasRenderingContext2D, stroke: EditorStroke) {
  const [first, ...rest] = stroke.points;
  if (!first) return;
  ctx.strokeStyle = stroke.color;
  ctx.fillStyle = stroke.color;
  ctx.lineWidth = stroke.width;
  ctx.lineCap = "round";
  ctx.lineJoin = "round";
  if (rest.length === 0) {
    // Single click — leave a dot instead of an invisible zero-length line.
    ctx.beginPath();
    ctx.arc(first.x, first.y, stroke.width / 2, 0, Math.PI * 2);
    ctx.fill();
    return;
  }
  ctx.beginPath();
  ctx.moveTo(first.x, first.y);
  for (const point of rest) ctx.lineTo(point.x, point.y);
  ctx.stroke();
}

function drawSegment(
  ctx: CanvasRenderingContext2D,
  from: EditorPoint,
  to: EditorPoint,
  stroke: EditorStroke,
) {
  ctx.strokeStyle = stroke.color;
  ctx.lineWidth = stroke.width;
  ctx.lineCap = "round";
  ctx.lineJoin = "round";
  ctx.beginPath();
  ctx.moveTo(from.x, from.y);
  ctx.lineTo(to.x, to.y);
  ctx.stroke();
}

/**
 * Composite the source image and strokes into a PNG at natural resolution.
 *
 * The source bytes are fetched over Tauri IPC and wrapped in a `blob:` URL.
 * Blob URLs are same-origin, so the canvas stays un-tainted and `toBlob`
 * works without any CORS involvement (the media proxy sends no CORS
 * headers, and cross-origin `crossOrigin="anonymous"` loads would need
 * them).
 */
async function renderAnnotatedPng(
  sourceUrl: string,
  sourceType: string,
  strokes: EditorStroke[],
): Promise<Uint8Array> {
  const bytes = await fetchMediaBytes(sourceUrl);
  // The explicit type matters: blob: image decoding is not content-sniffed
  // for all formats, so an untyped blob may fail to decode.
  const sourceBlob = new Blob([bytes], { type: sourceType });
  const blobUrl = URL.createObjectURL(sourceBlob);
  try {
    const image = new Image();
    image.src = blobUrl;
    await image.decode();

    const canvas = document.createElement("canvas");
    canvas.width = image.naturalWidth;
    canvas.height = image.naturalHeight;
    const ctx = canvas.getContext("2d");
    if (!ctx) throw new Error("Canvas 2D context unavailable");
    ctx.drawImage(image, 0, 0);
    for (const stroke of strokes) drawStroke(ctx, stroke);

    const blob = await new Promise<Blob | null>((resolve) => {
      canvas.toBlob(resolve, "image/png");
    });
    if (!blob) throw new Error("PNG encoding failed");
    return new Uint8Array(await blob.arrayBuffer());
  } finally {
    URL.revokeObjectURL(blobUrl);
  }
}

type ComposerImageEditorProps = {
  alt: string;
  /** Resolved (proxy-rewritten) image URL, for display. */
  src: string;
  /** Original relay media URL — export fetches its bytes over IPC. */
  sourceUrl: string;
  /** MIME type of the source image (from the blob descriptor). */
  sourceType: string;
  onCancel: () => void;
  /** Upload the annotated PNG; rejection keeps the editor open. */
  onSave: (bytes: Uint8Array) => Promise<void>;
};

/**
 * Freehand drawing mode for a composer image attachment: the image at
 * lightbox size with a canvas overlay, plus a pen toolbar (color, stroke
 * width, undo, clear, cancel, save). Strokes are stored in natural-image
 * coordinates so the exported PNG matches what's on screen.
 */
export function ComposerImageEditor({
  alt,
  src,
  sourceUrl,
  sourceType,
  onCancel,
  onSave,
}: ComposerImageEditorProps) {
  const canvasRef = React.useRef<HTMLCanvasElement>(null);
  const activeStrokeRef = React.useRef<EditorStroke | null>(null);
  // Committed strokes plus the undone strokes available for redo. Kept in
  // one state object so undo/redo move strokes between stacks atomically.
  const [history, setHistory] = React.useState<{
    strokes: EditorStroke[];
    undone: EditorStroke[];
  }>({ strokes: [], undone: [] });
  const strokes = history.strokes;
  const [activeColor, setActiveColor] = React.useState<string>(
    PEN_COLORS[0].value,
  );
  const [activeWidthCss, setActiveWidthCss] = React.useState<number>(
    PEN_WIDTH_DEFAULT_CSS,
  );
  const [naturalSize, setNaturalSize] = React.useState<{
    height: number;
    width: number;
  } | null>(null);
  const [saving, setSaving] = React.useState(false);
  const [saveError, setSaveError] = React.useState<string | null>(null);

  const handleImageLoad = React.useCallback(
    (event: React.SyntheticEvent<HTMLImageElement>) => {
      const { naturalHeight, naturalWidth } = event.currentTarget;
      if (naturalWidth > 0 && naturalHeight > 0) {
        setNaturalSize({ height: naturalHeight, width: naturalWidth });
      }
    },
    [],
  );

  // Redraw committed strokes whenever they change (undo/clear/commit).
  // Live segments are drawn imperatively during pointermove for latency.
  React.useEffect(() => {
    const canvas = canvasRef.current;
    const ctx = canvas?.getContext("2d");
    if (!canvas || !ctx) return;
    ctx.clearRect(0, 0, canvas.width, canvas.height);
    for (const stroke of strokes) drawStroke(ctx, stroke);
  }, [strokes]);

  const undo = React.useCallback(() => {
    setHistory((prev) => {
      const last = prev.strokes[prev.strokes.length - 1];
      if (!last) return prev;
      return {
        strokes: prev.strokes.slice(0, -1),
        undone: [...prev.undone, last],
      };
    });
  }, []);

  const redo = React.useCallback(() => {
    setHistory((prev) => {
      const last = prev.undone[prev.undone.length - 1];
      if (!last) return prev;
      return {
        strokes: [...prev.strokes, last],
        undone: prev.undone.slice(0, -1),
      };
    });
  }, []);

  React.useEffect(() => {
    function onKeyDown(event: KeyboardEvent) {
      const isModZ =
        (event.metaKey || event.ctrlKey) &&
        !event.altKey &&
        event.key.toLowerCase() === "z";
      if (!isModZ) return;
      event.preventDefault();
      if (event.shiftKey) {
        redo();
      } else {
        undo();
      }
    }
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [redo, undo]);

  const toNaturalPoint = React.useCallback(
    (event: React.PointerEvent<HTMLCanvasElement>): EditorPoint | null => {
      const canvas = canvasRef.current;
      if (!canvas) return null;
      const rect = canvas.getBoundingClientRect();
      if (rect.width === 0 || rect.height === 0) return null;
      return {
        x: ((event.clientX - rect.left) / rect.width) * canvas.width,
        y: ((event.clientY - rect.top) / rect.height) * canvas.height,
      };
    },
    [],
  );

  const handlePointerDown = React.useCallback(
    (event: React.PointerEvent<HTMLCanvasElement>) => {
      if (event.button !== 0 || saving) return;
      const canvas = canvasRef.current;
      const point = toNaturalPoint(event);
      if (!canvas || !point) return;
      canvas.setPointerCapture(event.pointerId);
      const rect = canvas.getBoundingClientRect();
      const stroke: EditorStroke = {
        color: activeColor,
        points: [point],
        // Scale the chosen CSS width into natural pixels so the on-screen
        // preview matches the exported PNG exactly.
        width: Math.max(1, activeWidthCss * (canvas.width / rect.width)),
      };
      activeStrokeRef.current = stroke;
      const ctx = canvas.getContext("2d");
      if (ctx) drawStroke(ctx, stroke);
    },
    [activeColor, activeWidthCss, saving, toNaturalPoint],
  );

  const handlePointerMove = React.useCallback(
    (event: React.PointerEvent<HTMLCanvasElement>) => {
      const stroke = activeStrokeRef.current;
      const canvas = canvasRef.current;
      if (!stroke || !canvas) return;
      const point = toNaturalPoint(event);
      if (!point) return;
      const previous = stroke.points[stroke.points.length - 1];
      stroke.points.push(point);
      const ctx = canvas.getContext("2d");
      if (ctx && previous) drawSegment(ctx, previous, point, stroke);
    },
    [toNaturalPoint],
  );

  const commitActiveStroke = React.useCallback(() => {
    const stroke = activeStrokeRef.current;
    if (!stroke) return;
    activeStrokeRef.current = null;
    // A new stroke invalidates the redo stack, matching editor conventions.
    setHistory((prev) => ({ strokes: [...prev.strokes, stroke], undone: [] }));
  }, []);

  const handleSave = React.useCallback(async () => {
    if (saving || strokes.length === 0) return;
    setSaving(true);
    setSaveError(null);
    try {
      const bytes = await renderAnnotatedPng(sourceUrl, sourceType, strokes);
      await onSave(bytes);
      // On success the parent closes the lightbox and unmounts this component.
    } catch {
      setSaveError("Could not save the drawing. Please try again.");
      setSaving(false);
    }
  }, [onSave, saving, sourceType, sourceUrl, strokes]);

  const hasStrokes = strokes.length > 0;

  // The native cursor is hidden over the canvas; this DOM dot follows the
  // pointer instead. Unlike a `cursor: url(...)` image, an element sized in
  // CSS pixels is guaranteed to match the on-screen stroke width exactly.
  // Positioned imperatively during pointermove to avoid re-rendering.
  const brushPreviewRef = React.useRef<HTMLDivElement>(null);

  const moveBrushPreview = React.useCallback(
    (event: React.PointerEvent<HTMLCanvasElement>) => {
      const preview = brushPreviewRef.current;
      const rect = canvasRef.current?.getBoundingClientRect();
      if (!preview || !rect) return;
      preview.style.opacity = "1";
      preview.style.transform = `translate(${event.clientX - rect.left}px, ${event.clientY - rect.top}px) translate(-50%, -50%)`;
    },
    [],
  );

  const hideBrushPreview = React.useCallback(() => {
    const preview = brushPreviewRef.current;
    if (preview) preview.style.opacity = "0";
  }, []);

  return (
    <div className="relative z-10 flex max-h-full max-w-full flex-col items-center gap-3">
      <div className="relative">
        <img
          alt={alt}
          className="pointer-events-none max-h-[75vh] max-w-[85vw] select-none rounded-lg object-contain"
          draggable={false}
          onLoad={handleImageLoad}
          src={src}
        />
        {naturalSize ? (
          <>
            <canvas
              aria-label="Drawing canvas"
              className="absolute inset-0 h-full w-full cursor-none touch-none rounded-lg"
              data-testid="composer-image-editor-canvas"
              height={naturalSize.height}
              onPointerCancel={commitActiveStroke}
              onPointerDown={handlePointerDown}
              onPointerEnter={moveBrushPreview}
              onPointerLeave={hideBrushPreview}
              onPointerMove={(event) => {
                handlePointerMove(event);
                moveBrushPreview(event);
              }}
              onPointerUp={commitActiveStroke}
              ref={canvasRef}
              width={naturalSize.width}
            />
            <div
              aria-hidden
              className="pointer-events-none absolute left-0 top-0 rounded-full opacity-0 ring-1 ring-white/60"
              ref={brushPreviewRef}
              style={{
                backgroundColor: activeColor,
                height: `${activeWidthCss}px`,
                width: `${activeWidthCss}px`,
              }}
            />
          </>
        ) : null}
      </div>

      <div className="fixed right-4 top-4 z-20 flex items-center gap-3">
        <div
          className="flex items-center gap-3 animate-in fade-in slide-in-from-right-12 duration-300"
          data-testid="composer-image-editor-toolbar"
        >
          <input
            aria-label="Stroke width"
            className="h-1 w-12 cursor-pointer appearance-none rounded-full bg-white/25 [&::-webkit-slider-thumb]:h-3 [&::-webkit-slider-thumb]:w-3 [&::-webkit-slider-thumb]:appearance-none [&::-webkit-slider-thumb]:rounded-full [&::-webkit-slider-thumb]:bg-white"
            max={PEN_WIDTH_MAX_CSS}
            min={PEN_WIDTH_MIN_CSS}
            onChange={(event) => setActiveWidthCss(Number(event.target.value))}
            step={PEN_WIDTH_STEP_CSS}
            type="range"
            value={activeWidthCss}
          />

          <div className="flex items-center gap-1.5">
            {PEN_COLORS.map((color) => (
              <button
                aria-label={`${color.label} pen`}
                aria-pressed={activeColor === color.value}
                className={cn(
                  "flex h-5 w-5 items-center justify-center rounded-full transition-transform",
                  activeColor === color.value && "scale-110 ring-2 ring-white",
                )}
                key={color.value}
                onClick={() => setActiveColor(color.value)}
                type="button"
              >
                <span
                  className={cn(
                    "rounded-full transition-[height,width]",
                    color.label === "Black" && "ring-1 ring-white/30",
                  )}
                  style={{
                    backgroundColor: color.value,
                    height: `${activeWidthCss}px`,
                    width: `${activeWidthCss}px`,
                  }}
                />
              </button>
            ))}
          </div>

          <Tooltip disableHoverableContent>
            <TooltipTrigger asChild>
              <button
                aria-label="Undo last stroke"
                className="flex h-7 w-7 items-center justify-center rounded-full text-white transition-colors hover:bg-white/10 disabled:opacity-40 disabled:hover:bg-transparent"
                disabled={!hasStrokes}
                onClick={undo}
                type="button"
              >
                <Undo2 className="h-4 w-4" />
              </button>
            </TooltipTrigger>
            <TooltipContent>Undo (⌘Z)</TooltipContent>
          </Tooltip>
          <Tooltip disableHoverableContent>
            <TooltipTrigger asChild>
              <button
                aria-label="Redo stroke"
                className="flex h-7 w-7 items-center justify-center rounded-full text-white transition-colors hover:bg-white/10 disabled:opacity-40 disabled:hover:bg-transparent"
                disabled={history.undone.length === 0}
                onClick={redo}
                type="button"
              >
                <Redo2 className="h-4 w-4" />
              </button>
            </TooltipTrigger>
            <TooltipContent>Redo (⇧⌘Z)</TooltipContent>
          </Tooltip>
        </div>

        <Button
          className="text-white hover:bg-white/10 hover:text-white"
          disabled={saving}
          onClick={onCancel}
          size="sm"
          type="button"
          variant="ghost"
        >
          Cancel
        </Button>
        <Button
          data-testid="composer-image-editor-save"
          disabled={saving || !hasStrokes}
          onClick={() => void handleSave()}
          size="sm"
          type="button"
        >
          {saving ? <Loader2 className="animate-spin" /> : null}
          Save
        </Button>
      </div>

      {saveError ? (
        <p className="text-xs text-red-300" role="alert">
          {saveError}
        </p>
      ) : null}
    </div>
  );
}
