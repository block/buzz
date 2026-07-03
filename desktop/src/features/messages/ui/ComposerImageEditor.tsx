import * as React from "react";
import { Loader2, Trash2, Undo2 } from "lucide-react";

import { cn } from "@/shared/lib/cn";
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

const PEN_WIDTHS = [
  { cssPx: 3, dotClass: "h-1 w-1", label: "Thin" },
  { cssPx: 6, dotClass: "h-2 w-2", label: "Medium" },
  { cssPx: 12, dotClass: "h-3 w-3", label: "Thick" },
] as const;

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
 * Requires the media proxy to grant CORS (`crossOrigin="anonymous"`), else
 * the canvas is tainted and `toBlob` throws.
 */
async function renderAnnotatedPng(
  src: string,
  strokes: EditorStroke[],
): Promise<Uint8Array> {
  const image = new Image();
  image.crossOrigin = "anonymous";
  // Distinct cache key from the display <img> loads: relay media is cached
  // as `immutable` for a year, and WKWebView may hold pre-CORS-fix entries
  // (no access-control-allow-origin header) that would fail the CORS check
  // without ever refetching. The extra param guarantees this CORS-mode load
  // gets a response that carries the header; the relay ignores the query.
  image.src = `${src}${src.includes("?") ? "&" : "?"}cors=1`;
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
}

type ComposerImageEditorProps = {
  alt: string;
  /** Resolved (proxy-rewritten) image URL. */
  src: string;
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
  onCancel,
  onSave,
}: ComposerImageEditorProps) {
  const canvasRef = React.useRef<HTMLCanvasElement>(null);
  const activeStrokeRef = React.useRef<EditorStroke | null>(null);
  const [strokes, setStrokes] = React.useState<EditorStroke[]>([]);
  const [activeColor, setActiveColor] = React.useState<string>(
    PEN_COLORS[0].value,
  );
  const [activeWidthCss, setActiveWidthCss] = React.useState<number>(
    PEN_WIDTHS[1].cssPx,
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
    setStrokes((prev) => (prev.length > 0 ? prev.slice(0, -1) : prev));
  }, []);

  React.useEffect(() => {
    function onKeyDown(event: KeyboardEvent) {
      const isUndo =
        (event.metaKey || event.ctrlKey) &&
        !event.shiftKey &&
        !event.altKey &&
        event.key.toLowerCase() === "z";
      if (!isUndo) return;
      event.preventDefault();
      undo();
    }
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [undo]);

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
    setStrokes((prev) => [...prev, stroke]);
  }, []);

  const handleSave = React.useCallback(async () => {
    if (saving || strokes.length === 0) return;
    setSaving(true);
    setSaveError(null);
    try {
      const bytes = await renderAnnotatedPng(src, strokes);
      await onSave(bytes);
      // On success the parent leaves edit mode and unmounts this component.
    } catch {
      setSaveError("Could not save the drawing. Please try again.");
      setSaving(false);
    }
  }, [onSave, saving, src, strokes]);

  const hasStrokes = strokes.length > 0;

  return (
    <div className="relative z-10 flex max-h-full max-w-full flex-col items-center gap-3">
      <div className="relative">
        <img
          alt={alt}
          className="max-h-[75vh] max-w-[85vw] select-none rounded-lg object-contain"
          draggable={false}
          onLoad={handleImageLoad}
          src={src}
        />
        {naturalSize ? (
          <canvas
            aria-label="Drawing canvas"
            className="absolute inset-0 h-full w-full cursor-crosshair touch-none rounded-lg"
            data-testid="composer-image-editor-canvas"
            height={naturalSize.height}
            onPointerCancel={commitActiveStroke}
            onPointerDown={handlePointerDown}
            onPointerMove={handlePointerMove}
            onPointerUp={commitActiveStroke}
            ref={canvasRef}
            width={naturalSize.width}
          />
        ) : null}
      </div>

      <div
        className="flex flex-wrap items-center justify-center gap-3 rounded-full bg-black/60 px-4 py-2 backdrop-blur-sm"
        data-testid="composer-image-editor-toolbar"
      >
        <div className="flex items-center gap-1.5">
          {PEN_COLORS.map((color) => (
            <button
              aria-label={`${color.label} pen`}
              aria-pressed={activeColor === color.value}
              className={cn(
                "h-5 w-5 rounded-full border border-white/30 transition-transform",
                activeColor === color.value
                  ? "scale-110 ring-2 ring-white"
                  : "hover:scale-105",
              )}
              key={color.value}
              onClick={() => setActiveColor(color.value)}
              style={{ backgroundColor: color.value }}
              type="button"
            />
          ))}
        </div>

        <div className="h-5 w-px bg-white/20" />

        <div className="flex items-center gap-1">
          {PEN_WIDTHS.map((width) => (
            <button
              aria-label={`${width.label} stroke`}
              aria-pressed={activeWidthCss === width.cssPx}
              className={cn(
                "flex h-7 w-7 items-center justify-center rounded-full text-white transition-colors",
                activeWidthCss === width.cssPx
                  ? "bg-white/25"
                  : "hover:bg-white/10",
              )}
              key={width.label}
              onClick={() => setActiveWidthCss(width.cssPx)}
              type="button"
            >
              <span className={cn("rounded-full bg-current", width.dotClass)} />
            </button>
          ))}
        </div>

        <div className="h-5 w-px bg-white/20" />

        <Tooltip>
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
        <Tooltip>
          <TooltipTrigger asChild>
            <button
              aria-label="Clear all strokes"
              className="flex h-7 w-7 items-center justify-center rounded-full text-white transition-colors hover:bg-white/10 disabled:opacity-40 disabled:hover:bg-transparent"
              disabled={!hasStrokes}
              onClick={() => setStrokes([])}
              type="button"
            >
              <Trash2 className="h-4 w-4" />
            </button>
          </TooltipTrigger>
          <TooltipContent>Clear drawing</TooltipContent>
        </Tooltip>

        <div className="h-5 w-px bg-white/20" />

        <button
          className="rounded-full px-3 py-1 text-sm text-white/80 transition-colors hover:bg-white/10 hover:text-white"
          onClick={onCancel}
          type="button"
        >
          Cancel
        </button>
        <button
          className="flex items-center gap-1.5 rounded-full bg-white px-3 py-1 text-sm font-medium text-black transition-opacity hover:opacity-90 disabled:opacity-50"
          data-testid="composer-image-editor-save"
          disabled={saving || !hasStrokes}
          onClick={() => void handleSave()}
          type="button"
        >
          {saving ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : null}
          Save
        </button>
      </div>

      {saveError ? (
        <p className="text-xs text-red-300" role="alert">
          {saveError}
        </p>
      ) : null}
    </div>
  );
}
