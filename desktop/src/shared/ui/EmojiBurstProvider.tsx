import * as React from "react";

type BurstPoint = {
  x: number;
  y: number;
};

type EmojiBurstOrigin =
  | Element
  | {
      clientX?: number;
      clientY?: number;
      currentTarget?: EventTarget | null;
      target?: EventTarget | null;
    }
  | null
  | undefined;

type EmojiBurstPayload = {
  emoji: string;
  emojiUrl?: string | null;
  senderName?: string | null;
};

type EmojiBurstContextValue = {
  burstEmoji: (emoji: string, origin?: EmojiBurstOrigin) => void;
  burstHuddleReaction: (
    reaction: string | EmojiBurstPayload,
    origin?: EmojiBurstOrigin,
  ) => void;
  celebrateWithEmojiFloatBurst: () => void;
};

type Particle = {
  x: number;
  y: number;
  xv: number;
  yv: number;
  rotation: number;
  spin: number;
  scale: number;
  opacity: number;
  life: number;
  maxLife: number;
  emoji: string;
  emojiUrl: string | null;
  label: string | null;
  fontSize: number;
  radius: number;
  gravity: number;
};

const NOOP_CONTEXT: EmojiBurstContextValue = {
  burstEmoji: () => {},
  burstHuddleReaction: () => {},
  celebrateWithEmojiFloatBurst: () => {},
};

const EmojiBurstContext = React.createContext<EmojiBurstContextValue | null>(
  null,
);

const MAX_ACTIVE = 760;
const MAX_DPR = 2;
const EMOJI_CACHE_PX = 64;
const EMOJI_CACHE_SCALE = 2;
// Keep the label's old reference scale so larger sprites do not shift its offset.
const EMOJI_LABEL_REFERENCE_SCALE = 1.5;
const PICKER_PARTICLES_PER_BURST = 5;
const PICKER_PARTICLE_LIFE_FRAMES = 108;
const HUDDLE_PARTICLES_PER_BURST = 14;
const HUDDLE_PARTICLE_LIFE_FRAMES = 118;
const CELEBRATION_PARTICLE_COUNT = 102;
const HEART_PARTICLE_EMOJIS = [
  "❤️",
  "🩷",
  "🧡",
  "💛",
  "💚",
  "🩵",
  "💙",
  "💜",
  "🤎",
  "🖤",
  "🩶",
  "🤍",
  "❤️‍🔥",
  "❤️‍🩹",
  "💖",
  "💕",
  "💗",
  "💓",
  "💞",
  "💘",
  "💝",
  "❣️",
  "♥️",
];
const POSITIVE_REACTION_PARTICLE_EMOJIS = [
  "👍",
  "👏",
  "🙌",
  "🙏",
  "💯",
  "🔥",
  "✨",
  "⭐",
  "🌟",
  "🎉",
  "🎊",
  "🥳",
  "🚀",
  "💪",
];
const POSITIVE_FACE_PARTICLE_EMOJIS = [
  "😀",
  "😃",
  "😄",
  "😁",
  "😊",
  "😇",
  "🙂",
  "😍",
  "🤩",
  "🥰",
  "😘",
  "😋",
  "😎",
  "😂",
  "🤣",
];

export const POSITIVE_EMOJI_PARTICLES = [
  ...HEART_PARTICLE_EMOJIS,
  ...POSITIVE_REACTION_PARTICLE_EMOJIS,
  ...POSITIVE_FACE_PARTICLE_EMOJIS,
];

const CELEBRATION_EMOJIS = POSITIVE_EMOJI_PARTICLES;
const POSITIVE_EMOJI_PARTICLE_SET = new Set(POSITIVE_EMOJI_PARTICLES);

const emojiCanvasCache = new Map<string, HTMLCanvasElement>();
const emojiImageCanvasCache = new Map<
  string,
  { canvas: HTMLCanvasElement | null }
>();

function isElement(value: unknown): value is Element {
  return typeof Element !== "undefined" && value instanceof Element;
}

function elementCenter(element: Element): BurstPoint | null {
  const rect = element.getBoundingClientRect();
  if (!rect.width && !rect.height) return null;
  return {
    x: rect.left + rect.width / 2,
    y: rect.top + rect.height / 2,
  };
}

function pointFromOrigin(origin: EmojiBurstOrigin): BurstPoint | null {
  if (!origin) return null;
  if (isElement(origin)) return elementCenter(origin);

  if (
    typeof origin.clientX === "number" &&
    Number.isFinite(origin.clientX) &&
    typeof origin.clientY === "number" &&
    Number.isFinite(origin.clientY) &&
    (origin.clientX !== 0 || origin.clientY !== 0)
  ) {
    return { x: origin.clientX, y: origin.clientY };
  }

  const target = origin.currentTarget ?? origin.target;
  return isElement(target) ? elementCenter(target) : null;
}

function resizeCanvas(canvas: HTMLCanvasElement) {
  const dpr = Math.min(window.devicePixelRatio || 1, MAX_DPR);
  const width = window.innerWidth;
  const height = window.innerHeight;
  const targetWidth = Math.round(width * dpr);
  const targetHeight = Math.round(height * dpr);

  if (canvas.width === targetWidth && canvas.height === targetHeight) {
    return;
  }

  canvas.width = targetWidth;
  canvas.height = targetHeight;
  canvas.style.width = `${width}px`;
  canvas.style.height = `${height}px`;
}

function getEmojiCanvas(emoji: string): HTMLCanvasElement {
  const dpr = Math.min(window.devicePixelRatio || 1, MAX_DPR);
  const cacheKey = `${emoji}:${dpr}:${EMOJI_CACHE_SCALE}`;
  const existing = emojiCanvasCache.get(cacheKey);
  if (existing) return existing;

  const fontSize = Math.ceil(EMOJI_CACHE_PX * dpr);
  const size = Math.ceil(fontSize * EMOJI_CACHE_SCALE);
  const canvas = document.createElement("canvas");
  canvas.width = size;
  canvas.height = size;

  const context = canvas.getContext("2d");
  if (!context) return canvas;

  context.textAlign = "center";
  context.textBaseline = "middle";
  context.font = `${fontSize}px "Apple Color Emoji", "Segoe UI Emoji", "Noto Color Emoji", sans-serif`;
  context.fillText(emoji, size / 2, size / 2);

  emojiCanvasCache.set(cacheKey, canvas);
  return canvas;
}

function getEmojiImageCanvas(src: string): HTMLCanvasElement | null {
  const dpr = Math.min(window.devicePixelRatio || 1, MAX_DPR);
  const cacheKey = `${src}:${dpr}:${EMOJI_CACHE_SCALE}`;
  const existing = emojiImageCanvasCache.get(cacheKey);
  if (existing) return existing.canvas;

  const record: { canvas: HTMLCanvasElement | null } = {
    canvas: null,
  };
  emojiImageCanvasCache.set(cacheKey, record);

  const image = new Image();
  image.decoding = "async";
  image.onload = () => {
    const size = Math.ceil(EMOJI_CACHE_PX * dpr * EMOJI_CACHE_SCALE);
    const canvas = document.createElement("canvas");
    canvas.width = size;
    canvas.height = size;
    const context = canvas.getContext("2d");
    if (!context) {
      return;
    }

    const sourceWidth = image.naturalWidth || size;
    const sourceHeight = image.naturalHeight || size;
    const scale = Math.min(size / sourceWidth, size / sourceHeight);
    const width = sourceWidth * scale;
    const height = sourceHeight * scale;
    context.drawImage(
      image,
      (size - width) / 2,
      (size - height) / 2,
      width,
      height,
    );
    record.canvas = canvas;
  };
  image.src = src;

  return null;
}

function roundedRectPath(
  context: CanvasRenderingContext2D,
  x: number,
  y: number,
  width: number,
  height: number,
  radius: number,
) {
  const cappedRadius = Math.min(radius, width / 2, height / 2);
  context.beginPath();
  context.moveTo(x + cappedRadius, y);
  context.lineTo(x + width - cappedRadius, y);
  context.quadraticCurveTo(x + width, y, x + width, y + cappedRadius);
  context.lineTo(x + width, y + height - cappedRadius);
  context.quadraticCurveTo(
    x + width,
    y + height,
    x + width - cappedRadius,
    y + height,
  );
  context.lineTo(x + cappedRadius, y + height);
  context.quadraticCurveTo(x, y + height, x, y + height - cappedRadius);
  context.lineTo(x, y + cappedRadius);
  context.quadraticCurveTo(x, y, x + cappedRadius, y);
  context.closePath();
}

function fitParticleLabel(
  context: CanvasRenderingContext2D,
  label: string,
  maxWidth: number,
) {
  if (context.measureText(label).width <= maxWidth) return label;

  const ellipsis = "…";
  let fitted = label;
  while (
    fitted.length > 1 &&
    context.measureText(`${fitted}${ellipsis}`).width > maxWidth
  ) {
    fitted = fitted.slice(0, -1).trimEnd();
  }

  return `${fitted}${ellipsis}`;
}

function drawParticleLabel(
  context: CanvasRenderingContext2D,
  particle: Particle,
  dpr: number,
  drawSize: number,
) {
  const label = particle.label?.trim();
  if (!label) return;

  context.setTransform(dpr, 0, 0, dpr, 0, 0);
  context.font =
    '500 11px Inter, ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif';
  context.textAlign = "center";
  context.textBaseline = "middle";

  const maxTextWidth = 120;
  const text = fitParticleLabel(context, label, maxTextWidth);
  const width = Math.min(
    144,
    Math.max(42, context.measureText(text).width + 20),
  );
  const height = 20;
  const x = particle.x - width / 2;
  const y = particle.y + drawSize * 0.32 - height / 2;

  roundedRectPath(context, x, y, width, height, height / 2);
  context.fillStyle = "rgba(0, 0, 0, 0.72)";
  context.fill();
  context.fillStyle = "rgb(255, 255, 255)";
  context.fillText(text, particle.x, y + height / 2 + 0.5, width - 12);
}

function updateParticle(particle: Particle): boolean {
  particle.life -= 1;
  particle.rotation += particle.spin;
  particle.yv += particle.gravity;
  particle.xv *= 0.965;
  particle.yv *= 0.998;
  particle.x += particle.xv;
  particle.y += particle.yv;
  particle.scale += (1 - particle.scale) * 0.28;
  particle.radius = particle.fontSize * particle.scale * 0.42;

  const lifeRatio = particle.life / particle.maxLife;
  if (lifeRatio < 0.24) {
    particle.opacity = Math.max(0, lifeRatio / 0.24);
  }

  return particle.life > 0 && particle.opacity > 0.02;
}

function resolveCollisions(particles: Particle[]) {
  for (let i = 0; i < particles.length; i += 1) {
    for (let j = i + 1; j < particles.length; j += 1) {
      const a = particles[i];
      const b = particles[j];
      const dx = b.x - a.x;
      const dy = b.y - a.y;
      const distanceSquared = dx * dx + dy * dy;
      const minDistance = a.radius + b.radius;

      if (
        distanceSquared >= minDistance * minDistance ||
        distanceSquared < 0.01
      ) {
        continue;
      }

      const distance = Math.sqrt(distanceSquared);
      const nx = dx / distance;
      const ny = dy / distance;
      const separation = (minDistance - distance) * 0.5;

      a.x -= nx * separation;
      a.y -= ny * separation;
      b.x += nx * separation;
      b.y += ny * separation;

      const dvx = a.xv - b.xv;
      const dvy = a.yv - b.yv;
      const velocityAlongNormal = dvx * nx + dvy * ny;
      if (velocityAlongNormal <= 0) continue;

      const impulse = velocityAlongNormal * 0.34;
      a.xv -= impulse * nx;
      a.yv -= impulse * ny;
      b.xv += impulse * nx;
      b.yv += impulse * ny;
    }
  }
}

function viewportCenter(): BurstPoint {
  return {
    x: window.innerWidth / 2,
    y: window.innerHeight / 2,
  };
}

function huddleReactionOrigin(): BurstPoint {
  return {
    x: 58,
    y: Math.max(64, window.innerHeight - 58),
  };
}

function normalizeBurstPayload(
  reaction: string | EmojiBurstPayload,
): EmojiBurstPayload | null {
  if (typeof reaction === "string") {
    const emoji = reaction.trim();
    return emoji ? { emoji } : null;
  }

  const emoji = reaction.emoji.trim();
  if (!emoji) return null;

  return {
    emoji,
    emojiUrl: reaction.emojiUrl?.trim() || null,
    senderName: reaction.senderName?.trim() || null,
  };
}

function spawnPickerEmojiBurst(
  particles: Particle[],
  point: BurstPoint,
  reaction: EmojiBurstPayload,
) {
  if (particles.length + PICKER_PARTICLES_PER_BURST > MAX_ACTIVE) return;

  for (let i = 0; i < PICKER_PARTICLES_PER_BURST; i += 1) {
    const horizontalDrift = (Math.random() - 0.5) * 4.4;
    const initialLift = 2.1 + Math.random() * 2.35;

    particles.push({
      x: point.x,
      y: point.y,
      xv: horizontalDrift,
      yv: -initialLift,
      rotation: (Math.random() - 0.5) * 22,
      spin: (Math.random() - 0.5) * 5.2,
      scale: 0.25,
      opacity: 1,
      life: PICKER_PARTICLE_LIFE_FRAMES,
      maxLife: PICKER_PARTICLE_LIFE_FRAMES,
      emoji: reaction.emoji,
      emojiUrl: reaction.emojiUrl ?? null,
      label: null,
      fontSize: 18 + Math.ceil(Math.random() * 24),
      radius: 0,
      gravity: -(0.018 + Math.random() * 0.018),
    });
  }
}

function spawnHuddleEmojiBurst(
  particles: Particle[],
  point: BurstPoint,
  reaction: EmojiBurstPayload,
) {
  if (particles.length + HUDDLE_PARTICLES_PER_BURST + 1 > MAX_ACTIVE) return;

  const heroLife = HUDDLE_PARTICLE_LIFE_FRAMES + 42;
  particles.push({
    x: point.x + 6,
    y: point.y - 4,
    xv: 2.6 + Math.random() * 0.8,
    yv: -(4.6 + Math.random() * 1.2),
    rotation: 0,
    spin: 0,
    scale: 0.48,
    opacity: 1,
    life: heroLife,
    maxLife: heroLife,
    emoji: reaction.emoji,
    emojiUrl: reaction.emojiUrl ?? null,
    label: reaction.senderName || null,
    fontSize: 87,
    radius: 0,
    gravity: 0.018,
  });

  for (let i = 0; i < HUDDLE_PARTICLES_PER_BURST; i += 1) {
    const lift = 3.2 + Math.random() * 4.1;
    const rightwardFan = 1.2 + Math.random() * 5.4;
    const life = HUDDLE_PARTICLE_LIFE_FRAMES + Math.floor(Math.random() * 28);

    particles.push({
      x: point.x + Math.random() * 12,
      y: point.y + (Math.random() - 0.5) * 12,
      xv: rightwardFan + (Math.random() - 0.5) * 1.1,
      yv: -lift,
      rotation: (Math.random() - 0.5) * 26,
      spin: (Math.random() - 0.5) * 4.2,
      scale: 0.24 + Math.random() * 0.12,
      opacity: 1,
      life,
      maxLife: life,
      emoji: reaction.emoji,
      emojiUrl: reaction.emojiUrl ?? null,
      label: null,
      fontSize: 18 + Math.ceil(Math.random() * 26),
      radius: 0,
      gravity: 0.012 + Math.random() * 0.018,
    });
  }
}

function spawnEmojiFloatBurst(particles: Particle[]) {
  const availableSlots = Math.max(0, MAX_ACTIVE - particles.length);
  const count = Math.min(CELEBRATION_PARTICLE_COUNT, availableSlots);
  if (count === 0) return;

  const centerX = window.innerWidth / 2;
  const launchBandWidth = window.innerWidth * 0.78;

  for (let i = 0; i < count; i += 1) {
    const x = centerX + (Math.random() - 0.5) * launchBandWidth;
    const y = window.innerHeight + 52 + Math.random() * 88;
    const life = 142 + Math.floor(Math.random() * 72);
    const fanDirection = (x - centerX) / Math.max(window.innerWidth / 2, 1);
    const emoji =
      CELEBRATION_EMOJIS[Math.floor(Math.random() * CELEBRATION_EMOJIS.length)];

    particles.push({
      x,
      y,
      xv:
        fanDirection * (4.2 + Math.random() * 3.4) +
        (Math.random() - 0.5) * 2.2,
      yv: -(5.4 + Math.random() * 4.8),
      rotation: (Math.random() - 0.5) * 70,
      spin: (Math.random() - 0.5) * 7.6,
      scale: 0.44 + Math.random() * 0.48,
      opacity: 1,
      life,
      maxLife: life,
      emoji,
      emojiUrl: null,
      label: null,
      fontSize: 22 + Math.ceil(Math.random() * 30),
      radius: 0,
      gravity: 0,
    });
  }
}

export function EmojiBurstProvider({
  children,
}: {
  children: React.ReactNode;
}) {
  const canvasRef = React.useRef<HTMLCanvasElement>(null);
  const contextRef = React.useRef<CanvasRenderingContext2D | null>(null);
  const particlesRef = React.useRef<Particle[]>([]);
  const animationFrameRef = React.useRef<number | null>(null);
  const reducedMotionRef = React.useRef(false);

  const startLoop = React.useCallback(() => {
    if (animationFrameRef.current !== null) return;

    const canvas = canvasRef.current;
    const context = contextRef.current;
    if (!canvas || !context) return;
    const activeCanvas = canvas;
    const activeContext = context;

    function frame() {
      const particles = particlesRef.current;
      const dpr = Math.min(window.devicePixelRatio || 1, MAX_DPR);

      for (let i = particles.length - 1; i >= 0; i -= 1) {
        if (!updateParticle(particles[i])) {
          particles[i] = particles[particles.length - 1];
          particles.pop();
        }
      }

      activeContext.setTransform(1, 0, 0, 1, 0, 0);
      activeContext.clearRect(0, 0, activeCanvas.width, activeCanvas.height);

      if (particles.length === 0) {
        activeContext.globalAlpha = 1;
        animationFrameRef.current = null;
        return;
      }

      resolveCollisions(particles);

      for (const particle of particles) {
        activeContext.globalAlpha = particle.opacity;

        const emojiCanvas =
          (particle.emojiUrl ? getEmojiImageCanvas(particle.emojiUrl) : null) ??
          getEmojiCanvas(particle.emoji);
        const drawSize = particle.fontSize * particle.scale * EMOJI_CACHE_SCALE;
        const labelReferenceSize =
          particle.fontSize * particle.scale * EMOJI_LABEL_REFERENCE_SCALE;
        const halfSize = drawSize / 2;
        const radians = (particle.rotation * Math.PI) / 180;
        const cos = Math.cos(radians) * dpr;
        const sin = Math.sin(radians) * dpr;

        activeContext.setTransform(
          cos,
          sin,
          -sin,
          cos,
          particle.x * dpr,
          particle.y * dpr,
        );
        activeContext.drawImage(
          emojiCanvas,
          -halfSize,
          -halfSize,
          drawSize,
          drawSize,
        );
        drawParticleLabel(activeContext, particle, dpr, labelReferenceSize);
      }

      activeContext.globalAlpha = 1;
      animationFrameRef.current = requestAnimationFrame(frame);
    }

    animationFrameRef.current = requestAnimationFrame(frame);
  }, []);

  React.useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;

    contextRef.current = canvas.getContext("2d");
    resizeCanvas(canvas);

    const handleResize = () => resizeCanvas(canvas);
    window.addEventListener("resize", handleResize);

    return () => {
      window.removeEventListener("resize", handleResize);
      if (animationFrameRef.current !== null) {
        cancelAnimationFrame(animationFrameRef.current);
        animationFrameRef.current = null;
      }
    };
  }, []);

  React.useEffect(() => {
    const mediaQuery = window.matchMedia("(prefers-reduced-motion: reduce)");
    const syncReducedMotion = () => {
      reducedMotionRef.current = mediaQuery.matches;
    };

    syncReducedMotion();
    mediaQuery.addEventListener("change", syncReducedMotion);

    return () => {
      mediaQuery.removeEventListener("change", syncReducedMotion);
    };
  }, []);

  const burstEmoji = React.useCallback(
    (emoji: string, origin?: EmojiBurstOrigin) => {
      const reaction = normalizeBurstPayload(emoji);
      if (!reaction || reducedMotionRef.current) return;

      spawnPickerEmojiBurst(
        particlesRef.current,
        pointFromOrigin(origin) ?? viewportCenter(),
        reaction,
      );
      startLoop();
    },
    [startLoop],
  );

  const burstHuddleReaction = React.useCallback(
    (input: string | EmojiBurstPayload, origin?: EmojiBurstOrigin) => {
      const reaction = normalizeBurstPayload(input);
      if (!reaction || reducedMotionRef.current) return;

      spawnHuddleEmojiBurst(
        particlesRef.current,
        pointFromOrigin(origin) ?? huddleReactionOrigin(),
        reaction,
      );
      startLoop();
    },
    [startLoop],
  );

  const celebrateWithEmojiFloatBurst = React.useCallback(() => {
    if (reducedMotionRef.current) return;

    spawnEmojiFloatBurst(particlesRef.current);
    startLoop();
  }, [startLoop]);

  const value = React.useMemo<EmojiBurstContextValue>(
    () => ({ burstEmoji, burstHuddleReaction, celebrateWithEmojiFloatBurst }),
    [burstEmoji, burstHuddleReaction, celebrateWithEmojiFloatBurst],
  );

  return (
    <EmojiBurstContext.Provider value={value}>
      {children}
      <div
        aria-hidden="true"
        className="pointer-events-none fixed inset-0 select-none"
        style={{ contain: "strict", zIndex: 2147483000 }}
      >
        <canvas className="block" ref={canvasRef} />
      </div>
    </EmojiBurstContext.Provider>
  );
}

export function useEmojiBurst(): EmojiBurstContextValue {
  return React.useContext(EmojiBurstContext) ?? NOOP_CONTEXT;
}

export function isPositiveEmojiParticle(emoji: string): boolean {
  return POSITIVE_EMOJI_PARTICLE_SET.has(emoji);
}
