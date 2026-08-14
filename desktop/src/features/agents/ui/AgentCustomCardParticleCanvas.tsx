import * as React from "react";

import {
  createDefaultSettings,
  inverseParticleColor,
  type ParticleGridSettings,
} from "./customCardParticleGrid";

const CARD_DESIGN_WIDTH = 1227;
const CARD_DESIGN_HEIGHT = 1839;
const MAX_DEVICE_PIXEL_RATIO = 2;
const TARGET_FRAME_INTERVAL_MS = 1000 / 30;

type PointerPosition = {
  active: boolean;
  x: number;
  y: number;
};

function usePrefersReducedMotion() {
  const [shouldReduceMotion, setShouldReduceMotion] = React.useState(
    () =>
      typeof window !== "undefined" &&
      window.matchMedia("(prefers-reduced-motion: reduce)").matches,
  );

  React.useEffect(() => {
    const query = window.matchMedia("(prefers-reduced-motion: reduce)");
    const updatePreference = () => setShouldReduceMotion(query.matches);
    updatePreference();
    query.addEventListener("change", updatePreference);
    return () => query.removeEventListener("change", updatePreference);
  }, []);

  return shouldReduceMotion;
}

function clamp(value: number, min: number, max: number) {
  return Math.min(Math.max(value, min), max);
}

function rgbColor([red, green, blue]: [number, number, number]) {
  return `rgb(${Math.round(clamp(red, 0, 1) * 255)}, ${Math.round(
    clamp(green, 0, 1) * 255,
  )}, ${Math.round(clamp(blue, 0, 1) * 255)})`;
}

function readSurfaceBackground(
  canvas: HTMLCanvasElement,
  fallback: [number, number, number],
): [number, number, number] {
  const surface = canvas.closest<HTMLElement>(".agent-custom-card-waiting");
  if (!surface) return fallback;

  const channels =
    getComputedStyle(surface).backgroundColor.match(/-?\d*\.?\d+/g);
  if (!channels || channels.length < 3) return fallback;

  const [red, green, blue] = channels.map(Number);
  if (![red, green, blue].every(Number.isFinite)) return fallback;
  return [
    clamp(red ?? 0, 0, 255) / 255,
    clamp(green ?? 0, 0, 255) / 255,
    clamp(blue ?? 0, 0, 255) / 255,
  ];
}

function thinkingTarget(
  designX: number,
  designY: number,
  elapsedSeconds: number,
  settings: ParticleGridSettings,
) {
  const spatialX = designX * settings.noiseScale;
  const spatialY = designY * settings.noiseScale;
  const time = elapsedSeconds * settings.noiseSpeed;
  const field =
    Math.sin(spatialX * 5.2 + time * 2.2 + Math.sin(spatialY * 2.7 - time)) +
    Math.sin(spatialY * 4.1 - time * 1.7 + Math.cos(spatialX * 3.3 + time)) *
      0.68 +
    Math.sin((spatialX + spatialY) * 6.4 + time * 1.2) * 0.32;
  return Math.sin(field * 1.9) * 0.5 + 0.5;
}

function drawParticleGrid({
  canvas,
  context,
  elapsedSeconds,
  highlights,
  isFirstFrame,
  pointer,
  settings,
  surfaceBackground,
}: {
  canvas: HTMLCanvasElement;
  context: CanvasRenderingContext2D;
  elapsedSeconds: number;
  highlights: Float32Array;
  isFirstFrame: boolean;
  pointer: PointerPosition;
  settings: ParticleGridSettings;
  surfaceBackground: [number, number, number];
}) {
  const width = canvas.clientWidth;
  const height = canvas.clientHeight;
  if (width <= 0 || height <= 0) return;

  const dpr = Math.min(window.devicePixelRatio || 1, MAX_DEVICE_PIXEL_RATIO);
  const pixelWidth = Math.max(1, Math.round(width * dpr));
  const pixelHeight = Math.max(1, Math.round(height * dpr));
  if (canvas.width !== pixelWidth || canvas.height !== pixelHeight) {
    canvas.width = pixelWidth;
    canvas.height = pixelHeight;
  }
  context.setTransform(dpr, 0, 0, dpr, 0, 0);
  context.fillStyle = rgbColor(surfaceBackground);
  context.fillRect(0, 0, width, height);

  const spacing = settings.gridSize;
  const columns = Math.ceil(width / spacing) + 1;
  const rows = Math.ceil(height / spacing) + 1;
  const particleCount = columns * rows;
  if (highlights.length !== particleCount) return;

  const touchRadius = settings.touchRadius * (width / CARD_DESIGN_WIDTH);
  const dotColor = rgbColor(inverseParticleColor(surfaceBackground));
  const thinkingStrength = settings.mode === 2 ? settings.thinkingFade : 0;
  context.fillStyle = dotColor;

  for (let row = 0; row < rows; row += 1) {
    for (let column = 0; column < columns; column += 1) {
      const index = row * columns + column;
      const x = column * spacing - spacing / 2;
      const y = row * spacing - spacing / 2;
      const designX = (x / width) * CARD_DESIGN_WIDTH;
      const designY = (y / height) * CARD_DESIGN_HEIGHT;
      let target =
        thinkingTarget(designX, designY, elapsedSeconds, settings) *
        thinkingStrength;

      if (pointer.active) {
        const distance = Math.hypot(x - pointer.x, y - pointer.y);
        const falloff = clamp(1 - distance / touchRadius, 0, 1);
        target = Math.max(target, falloff * falloff * settings.touchStrength);
      }

      const previous = highlights[index] ?? 0;
      const highlight = isFirstFrame
        ? target
        : previous + (target - previous) * 0.07;
      highlights[index] = highlight;

      const shaped = clamp(highlight, 0, 1) ** settings.charGamma;
      const opacity =
        (settings.idleOpacity +
          (settings.baseOpacity - settings.idleOpacity) * shaped) *
        settings.gridOpacity;
      const radius = spacing * settings.charScale * (0.034 + shaped * 0.421);

      context.globalAlpha = clamp(opacity, 0, 1);
      context.beginPath();
      context.arc(x, y, radius, 0, Math.PI * 2);
      context.fill();
    }
  }

  context.globalAlpha = 1;
  canvas.dataset.ready = "true";
}

export function AgentCustomCardParticleCanvas() {
  const canvasRef = React.useRef<HTMLCanvasElement>(null);
  const pointerRef = React.useRef<PointerPosition>({
    active: false,
    x: 0,
    y: 0,
  });
  const settingsRef = React.useRef(createDefaultSettings());
  const shouldReduceMotion = usePrefersReducedMotion();
  const settings = settingsRef.current;

  React.useEffect(() => {
    const canvas = canvasRef.current;
    const context = canvas?.getContext("2d");
    if (!canvas || !context) return;

    let animationFrame = 0;
    let highlights = new Float32Array(0);
    let isFirstFrame = true;
    let isVisible = true;
    let lastFrameTime = -TARGET_FRAME_INTERVAL_MS;
    let surfaceBackground = readSurfaceBackground(
      canvas,
      settings.backgroundColor,
    );

    const rebuildGrid = () => {
      const columns = Math.ceil(canvas.clientWidth / settings.gridSize) + 1;
      const rows = Math.ceil(canvas.clientHeight / settings.gridSize) + 1;
      highlights = new Float32Array(Math.max(0, columns * rows));
      isFirstFrame = true;
    };

    const render = (time: number) => {
      drawParticleGrid({
        canvas,
        context,
        elapsedSeconds: time / 1000,
        highlights,
        isFirstFrame,
        pointer: pointerRef.current,
        settings,
        surfaceBackground,
      });
      canvas.dataset.backgroundColor = surfaceBackground.join(",");
      canvas.dataset.dotColor =
        inverseParticleColor(surfaceBackground).join(",");
      isFirstFrame = false;
    };

    const tick = (time: number) => {
      if (isVisible && time - lastFrameTime >= TARGET_FRAME_INTERVAL_MS) {
        render(time);
        lastFrameTime = time;
      }
      animationFrame = requestAnimationFrame(tick);
    };

    const resizeObserver = new ResizeObserver(() => {
      rebuildGrid();
      render(0);
    });
    resizeObserver.observe(canvas);

    const intersectionObserver = new IntersectionObserver(([entry]) => {
      isVisible = entry?.isIntersecting ?? true;
    });
    intersectionObserver.observe(canvas);

    const themeObserver = new MutationObserver(() => {
      surfaceBackground = readSurfaceBackground(
        canvas,
        settings.backgroundColor,
      );
      render(performance.now());
    });
    themeObserver.observe(document.documentElement, {
      attributeFilter: ["class", "style"],
      attributes: true,
    });

    rebuildGrid();
    render(0);
    if (!shouldReduceMotion) animationFrame = requestAnimationFrame(tick);

    return () => {
      cancelAnimationFrame(animationFrame);
      intersectionObserver.disconnect();
      resizeObserver.disconnect();
      themeObserver.disconnect();
    };
  }, [settings, shouldReduceMotion]);

  const updatePointer = React.useCallback(
    (event: React.PointerEvent<HTMLCanvasElement>) => {
      const rect = event.currentTarget.getBoundingClientRect();
      pointerRef.current = {
        active: !shouldReduceMotion,
        x: event.clientX - rect.left,
        y: event.clientY - rect.top,
      };
    },
    [shouldReduceMotion],
  );

  const resetPointer = React.useCallback(() => {
    pointerRef.current = { active: false, x: 0, y: 0 };
  }, []);

  return (
    <canvas
      aria-hidden
      className="agent-custom-card-waiting__particle-grid"
      data-animated={shouldReduceMotion ? "false" : "true"}
      data-char-set={settings.charSet}
      data-grid-size={settings.gridSize}
      data-mode={settings.mode}
      data-object-reveal-style={settings.objectRevealStyle}
      data-testid="agent-custom-card-particle-grid"
      onPointerLeave={resetPointer}
      onPointerMove={updatePointer}
      ref={canvasRef}
    />
  );
}
