import * as React from "react";

type Hsl = {
  h: number;
  s: number;
  l: number;
};

type Point = {
  x: number;
  y: number;
};

type Particle = {
  originX: number;
  originY: number;
  vx: number;
  vy: number;
  size: number;
  lifetime: number;
  respawn: number;
  phase: number;
  lightness: number;
  shape: "circle" | "square";
};

type ParticleScene = {
  accent: Hsl;
  cssHeight: number;
  cssWidth: number;
  dpr: number;
  particles: Particle[];
};

type SpoilerParticlesProps = {
  active: boolean;
  contentRef: React.RefObject<HTMLElement | null>;
};

type SpoilerParticleCanvasOptions = {
  canvas: HTMLCanvasElement;
  content: HTMLElement;
  syncCanvas?: () => boolean;
};

const DEFAULT_ACCENT: Hsl = { h: 0, s: 0, l: 100 };
const DEFAULT_DENSITY = 0.12;
const DEFAULT_FPS = 24;
const DEFAULT_GAP = 6;
const GAP_RATIO = 8;
const MAX_PARTICLES = 5000;
const RANDOM_SEED = 4011505;
const V_MIN = 2;
const V_MAX = 12;

export function SpoilerParticles({
  active,
  contentRef,
}: SpoilerParticlesProps) {
  const canvasRef = React.useRef<HTMLCanvasElement | null>(null);

  React.useEffect(() => {
    if (!active) return;
    const canvas = canvasRef.current;
    const content = contentRef.current;
    if (!canvas || !content) return;

    return mountSpoilerParticleCanvas({ canvas, content });
  }, [active, contentRef]);

  return <canvas className="buzz-spoiler__particles" ref={canvasRef} />;
}

export function mountSpoilerParticleCanvas({
  canvas,
  content,
  syncCanvas,
}: SpoilerParticleCanvasOptions): () => void {
  const context = canvas.getContext("2d");
  if (!context) return () => {};

  let animationFrame = 0;
  let lastFrameTime = 0;
  let scene: ParticleScene | null = null;
  let sceneIsStale = true;
  let visible = true;
  let reducedMotion = window.matchMedia(
    "(prefers-reduced-motion: reduce)",
  ).matches;

  const ensureScene = () => {
    if (syncCanvas && !syncCanvas()) {
      context.clearRect(0, 0, canvas.width, canvas.height);
      return null;
    }

    const rect = canvas.getBoundingClientRect();
    const dpr = Math.max(1, window.devicePixelRatio || 1);
    const cssWidth = Math.max(1, rect.width);
    const cssHeight = Math.max(1, rect.height);

    if (
      !scene ||
      sceneIsStale ||
      scene.cssWidth !== cssWidth ||
      scene.cssHeight !== cssHeight ||
      scene.dpr !== dpr
    ) {
      scene = buildScene(canvas, content, cssWidth, cssHeight, dpr);
      sceneIsStale = false;
    }

    return scene;
  };

  const draw = (timeMs: number) => {
    const currentScene = ensureScene();
    if (!currentScene) return;

    drawParticles(context, currentScene, reducedMotion ? 0 : timeMs / 1000);
  };

  const stop = () => {
    if (animationFrame) {
      window.cancelAnimationFrame(animationFrame);
      animationFrame = 0;
    }
  };

  const frame = (timeMs: number) => {
    if (lastFrameTime === 0 || timeMs - lastFrameTime >= 1000 / DEFAULT_FPS) {
      draw(timeMs);
      lastFrameTime = timeMs;
    }
    animationFrame = window.requestAnimationFrame(frame);
  };

  const start = () => {
    if (!visible || animationFrame) return;
    if (reducedMotion) {
      draw(0);
      return;
    }
    animationFrame = window.requestAnimationFrame(frame);
  };

  const resizeObserver = new ResizeObserver(() => {
    sceneIsStale = true;
    draw(performance.now());
  });
  resizeObserver.observe(content);

  const contentObserver = new MutationObserver(() => {
    sceneIsStale = true;
    draw(performance.now());
  });
  contentObserver.observe(content, {
    attributes: true,
    characterData: true,
    childList: true,
    subtree: true,
  });

  const themeObserver = new MutationObserver(() => {
    sceneIsStale = true;
    draw(performance.now());
  });
  themeObserver.observe(content.ownerDocument.documentElement, {
    attributeFilter: ["class", "style"],
    attributes: true,
  });

  const intersectionObserver = new IntersectionObserver(
    ([entry]) => {
      visible = entry?.isIntersecting ?? true;
      if (visible) {
        start();
      } else {
        stop();
      }
    },
    { rootMargin: "120px" },
  );
  intersectionObserver.observe(content);

  const motionQuery = window.matchMedia("(prefers-reduced-motion: reduce)");
  const handleMotionChange = () => {
    reducedMotion = motionQuery.matches;
    stop();
    start();
  };
  motionQuery.addEventListener("change", handleMotionChange);

  draw(performance.now());
  start();

  return () => {
    stop();
    resizeObserver.disconnect();
    contentObserver.disconnect();
    themeObserver.disconnect();
    intersectionObserver.disconnect();
    motionQuery.removeEventListener("change", handleMotionChange);
  };
}

function buildScene(
  canvas: HTMLCanvasElement,
  content: HTMLElement,
  cssWidth: number,
  cssHeight: number,
  dpr: number,
): ParticleScene | null {
  const pixelWidth = Math.max(1, Math.ceil(cssWidth * dpr));
  const pixelHeight = Math.max(1, Math.ceil(cssHeight * dpr));

  if (canvas.width !== pixelWidth) canvas.width = pixelWidth;
  if (canvas.height !== pixelHeight) canvas.height = pixelHeight;

  const accent = readAccent(canvas);
  const maskPoints = buildWordMaskPoints(content, canvas, cssWidth, cssHeight);
  const points =
    maskPoints.length > 0
      ? maskPoints
      : buildFallbackPoints(cssWidth, cssHeight);

  const particleCount = Math.round(
    Math.min(
      MAX_PARTICLES,
      Math.max(16, DEFAULT_DENSITY * cssWidth * cssHeight),
    ),
  );
  const rand = lcgrand(RANDOM_SEED);
  const ldir = accent.l > 50 ? -1 : 1;
  const particles: Particle[] = Array.from({ length: particleCount }, () => {
    const point = points[Math.floor(rand(0, points.length))] ?? {
      x: rand(0, cssWidth),
      y: rand(0, cssHeight),
    };
    const speed = rand(V_MIN, V_MAX);
    const angle = rand(0, Math.PI * 2);
    const lifetime = rand(0.3, 1.5);
    const respawn = rand(0, 1);

    return {
      originX: point.x + rand(-0.35, 0.35),
      originY: point.y + rand(-0.35, 0.35),
      vx: speed * Math.cos(angle),
      vy: speed * Math.sin(angle),
      size: rand(1, 1 + (dpr > 1 ? 0.5 : 0)),
      lifetime,
      respawn,
      phase: rand(0, lifetime + respawn),
      lightness: clamp(accent.l + ldir * rand(0, 30), 0, 100),
      shape: rand() > 0.5 ? "square" : "circle",
    };
  });

  return {
    accent,
    cssHeight,
    cssWidth,
    dpr,
    particles,
  };
}

function buildWordMaskPoints(
  content: HTMLElement,
  canvas: HTMLCanvasElement,
  cssWidth: number,
  cssHeight: number,
): Point[] {
  const canvasRect = canvas.getBoundingClientRect();
  const document = content.ownerDocument;
  const range = document.createRange();
  const points: Point[] = [];
  const walker = document.createTreeWalker(content, NodeFilter.SHOW_TEXT, {
    acceptNode(node) {
      return node.textContent?.trim()
        ? NodeFilter.FILTER_ACCEPT
        : NodeFilter.FILTER_REJECT;
    },
  });

  while (walker.nextNode()) {
    const node = walker.currentNode;
    const text = node.textContent ?? "";
    const words = text.matchAll(/\S+/g);

    for (const word of words) {
      const start = word.index ?? 0;
      const end = start + word[0].length;
      range.setStart(node, start);
      range.setEnd(node, end);

      for (const rect of range.getClientRects()) {
        addRectPoints(points, {
          x: rect.left - canvasRect.left,
          y: rect.top - canvasRect.top,
          width: rect.width,
          height: rect.height,
        });
      }
    }
  }

  range.detach();

  for (const media of content.querySelectorAll("img, video")) {
    const rect = media.getBoundingClientRect();
    addRectPoints(points, {
      height: rect.height,
      width: rect.width,
      x: rect.left - canvasRect.left,
      y: rect.top - canvasRect.top,
    });
  }

  return points.filter(
    (point) =>
      point.x >= 0 &&
      point.y >= 0 &&
      point.x <= cssWidth &&
      point.y <= cssHeight,
  );
}

function addRectPoints(
  points: Point[],
  rect: { x: number; y: number; width: number; height: number },
) {
  if (rect.width <= 0 || rect.height <= 0) return;

  const verticalGap = Math.min(DEFAULT_GAP, rect.height / GAP_RATIO);
  const yStart = rect.y + verticalGap;
  const yEnd = rect.y + Math.max(verticalGap, rect.height - verticalGap);
  const step = 1;

  for (let y = yStart; y <= yEnd; y += step) {
    for (let x = rect.x; x <= rect.x + rect.width; x += step) {
      points.push({ x, y });
    }
  }
}

function buildFallbackPoints(cssWidth: number, cssHeight: number): Point[] {
  const points: Point[] = [];
  const verticalGap = Math.min(DEFAULT_GAP, cssHeight / GAP_RATIO);

  for (let y = verticalGap; y <= cssHeight - verticalGap; y += 1) {
    for (let x = 0; x <= cssWidth; x += 1) {
      points.push({ x, y });
    }
  }

  return points;
}

function drawParticles(
  context: CanvasRenderingContext2D,
  scene: ParticleScene,
  time: number,
) {
  context.setTransform(1, 0, 0, 1, 0, 0);
  context.clearRect(
    0,
    0,
    Math.ceil(scene.cssWidth * scene.dpr),
    Math.ceil(scene.cssHeight * scene.dpr),
  );
  context.setTransform(scene.dpr, 0, 0, scene.dpr, 0, 0);

  for (const particle of scene.particles) {
    const cycle = particle.lifetime + particle.respawn;
    const t = Math.min(particle.lifetime, (time + particle.phase) % cycle);
    const visibility = trapezoidalWave(particle.lifetime, 0.15, 0.3)(t);
    const alpha = clamp(1 - t / particle.lifetime, 0, 1);
    const size = particle.size * visibility;
    if (size <= 0) continue;

    const x = particle.originX + particle.vx * t;
    const y = particle.originY + particle.vy * t;

    context.fillStyle = `hsl(${scene.accent.h} ${scene.accent.s}% ${
      particle.lightness
    }% / ${Math.round(alpha * 100)}%)`;

    for (const [wrappedX, wrappedY] of cycleBounds(
      x,
      y,
      scene.cssWidth,
      scene.cssHeight,
      size / 2,
    )) {
      context.beginPath();
      if (particle.shape === "square") {
        context.rect(wrappedX, wrappedY, size, size);
      } else {
        context.arc(wrappedX, wrappedY, size / 2, 0, Math.PI * 2);
      }
      context.fill();
    }
  }
}

function readAccent(canvas: HTMLCanvasElement): Hsl {
  const parsed = parseRgb(window.getComputedStyle(canvas).color);
  return parsed ? rgbToHsl(parsed.r, parsed.g, parsed.b) : DEFAULT_ACCENT;
}

function parseRgb(value: string): { r: number; g: number; b: number } | null {
  const match = value.match(/rgba?\(\s*([\d.]+)[,\s]+([\d.]+)[,\s]+([\d.]+)/);
  if (!match) return null;
  return {
    b: Number(match[3]),
    g: Number(match[2]),
    r: Number(match[1]),
  };
}

function rgbToHsl(r: number, g: number, b: number): Hsl {
  const r1 = r / 255;
  const g1 = g / 255;
  const b1 = b / 255;
  const max = Math.max(r1, g1, b1);
  const min = Math.min(r1, g1, b1);
  const delta = max - min;
  const l = (max + min) / 2;

  if (delta === 0) {
    return { h: 0, s: 0, l: Math.round(l * 100) };
  }

  const s = delta / (1 - Math.abs(2 * l - 1));
  let h = 0;

  if (max === r1) {
    h = 60 * (((g1 - b1) / delta) % 6);
  } else if (max === g1) {
    h = 60 * ((b1 - r1) / delta + 2);
  } else {
    h = 60 * ((r1 - g1) / delta + 4);
  }

  return {
    h: Math.round((h + 360) % 360),
    l: Math.round(l * 100),
    s: Math.round(s * 100),
  };
}

function cycleBounds(
  x: number,
  y: number,
  width: number,
  height: number,
  radius: number,
): [number, number][] {
  const wrappedX = cycle(x, width);
  const wrappedY = cycle(y, height);

  return [
    [wrappedX, wrappedY],
    [mirror(wrappedX, width, radius), mirror(wrappedY, height, radius)],
  ];
}

function trapezoidalWave(length: number, a: number, b: number) {
  const stop = Math.max(a, length - b);

  return (time: number) => {
    if (time < a) return Math.max(0, time / a);
    if (time > stop) return Math.max(0, 1 - (time - stop) / (length - stop));
    return 1;
  };
}

function lcgrand(seed = 1) {
  let currentSeed = seed;
  return (a = 0, b = 1) => {
    currentSeed = Math.imul(214013, currentSeed) + 2531011;
    const next = (Math.imul(48271, currentSeed) & 0x7fffffff) / 0x7fffffff;
    return a + Math.abs(b - a) * next;
  };
}

function cycle(value: number, max: number): number {
  if (max <= 0) return 0;
  return ((value % max) + max) % max;
}

function mirror(value: number, max: number, radius: number): number {
  if (value < radius) return max + value;
  if (value > max - radius) return value - max;
  return value;
}

function clamp(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, value));
}
