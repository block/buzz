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

type EmojiBurstContextValue = {
  burstEmoji: (emoji: string, origin?: EmojiBurstOrigin) => void;
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
  fontSize: number;
  radius: number;
  gravity: number;
};

const NOOP_CONTEXT: EmojiBurstContextValue = {
  burstEmoji: () => {},
  celebrateWithEmojiFloatBurst: () => {},
};

const EmojiBurstContext = React.createContext<EmojiBurstContextValue | null>(
  null,
);

const MAX_ACTIVE = 760;
const MAX_DPR = 2;
const EMOJI_CACHE_PX = 64;
const PICKER_PARTICLES_PER_BURST = 5;
const PICKER_PARTICLE_LIFE_FRAMES = 108;
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
  const cacheKey = `${emoji}:${dpr}`;
  const existing = emojiCanvasCache.get(cacheKey);
  if (existing) return existing;

  const fontSize = Math.ceil(EMOJI_CACHE_PX * dpr);
  const size = Math.ceil(fontSize * 1.5);
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

function spawnPickerEmojiBurst(
  particles: Particle[],
  point: BurstPoint,
  emoji: string,
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
      emoji,
      fontSize: 18 + Math.ceil(Math.random() * 24),
      radius: 0,
      gravity: -(0.018 + Math.random() * 0.018),
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

        const emojiCanvas = getEmojiCanvas(particle.emoji);
        const drawSize = particle.fontSize * particle.scale * 1.5;
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
      const trimmedEmoji = emoji.trim();
      if (!trimmedEmoji || reducedMotionRef.current) return;

      spawnPickerEmojiBurst(
        particlesRef.current,
        pointFromOrigin(origin) ?? viewportCenter(),
        trimmedEmoji,
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
    () => ({ burstEmoji, celebrateWithEmojiFloatBurst }),
    [burstEmoji, celebrateWithEmojiFloatBurst],
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
