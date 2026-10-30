import { type CSSProperties, useEffect, useId, useRef, useState } from "react";
import { type Bee, spawnBees, stepBees } from "./bees";

// Two-tone bee palette, matching the design: ink + buzz-blue. Bees are tinted
// by masking the bee silhouette (bee.svg's alpha) over a solid background-color,
// so a single asset yields any number of colors.
const DEFAULT_PALETTE = ["#231e1e", "#D7E7F5"];

// Base footprint of one bee (px). Natural ratio is 466:309 ≈ 1.5. Every bee
// renders at this size × the field's sizeScale — no per-bee size variance, so
// the whole swarm reads as one consistent scale (per the designer note).
const BEE_W = 46;
const BEE_H = 31;

// The bee art (bee.svg) points UP — body is a vertical capsule with the eyes at
// the top. atan2(vy,vx) measures motion from the +x axis, so to aim the art at
// the travel direction we add 90°.
const ROT_OFFSET = 90;
// Below this speed (px/s) we hold the last facing rather than snapping to a
// meaningless angle when a bee is nearly stationary.
const ROT_MIN_SPEED = 6;
const INTRO_MS = 1050;
const TAU = Math.PI * 2;

// One compound path replaces the per-bee SVG mask. Safari struggles to animate
// 94 masked hero sprites while scrolling. The wing lobes stop at the body edge,
// so the even-odd subpaths preserve the eyes and stripes without masking.
const BEE_SPRITE_PATH = [
  "M128 70.3 A91.7 91.7 0 1 0 128 238.7 Z",
  "M338 70.3 A91.7 91.7 0 1 1 338 238.7 Z",
  "M162 0 H304 A34 34 0 0 1 338 34 V275 A34 34 0 0 1 304 309 H162 A34 34 0 0 1 128 275 V34 A34 34 0 0 1 162 0 Z",
  "M220.3 84.4 A27 27 0 1 1 166.3 84.4 A27 27 0 1 1 220.3 84.4 Z",
  "M303 84.4 A27 27 0 1 1 249 84.4 A27 27 0 1 1 303 84.4 Z",
  "M171.3 157.2 H298.2 A5 5 0 0 1 303.2 162.2 V190.5 A5 5 0 0 1 298.2 195.5 H171.3 A5 5 0 0 1 166.3 190.5 V162.2 A5 5 0 0 1 171.3 157.2 Z",
  "M171.9 235.1 H298.1 A5 5 0 0 1 303.1 240.1 V267.7 A5 5 0 0 1 298.1 272.7 H171.9 A5 5 0 0 1 166.9 267.7 V240.1 A5 5 0 0 1 171.9 235.1 Z",
].join(" ");

// Deterministic per-index tint so SSR and client agree (no Math.random). The
// sine-hash seed is independent of the bee's position seed (R2 in bees.ts) so
// same-color bees don't line up into diagonal streaks. Rotation and size are no
// longer per-bee (velocity-driven rotation + uniform size), so this is tint-only.
const hash = (i: number, seed: number) => {
  const v = Math.sin((i + 1) * seed) * 43758.5453;
  return v - Math.floor(v); // fractional part → [0, 1)
};
function tintOf(i: number, paletteLen: number) {
  return Math.floor(hash(i, 12.9898) * paletteLen);
}
function flapDelayOf(i: number) {
  return `${-(hash(i, 78.233) * 5.2).toFixed(2)}s`;
}

function useReducedMotion() {
  const [reduced, setReduced] = useState(
    () => window.matchMedia("(prefers-reduced-motion: reduce)").matches,
  );
  useEffect(() => {
    const media = window.matchMedia("(prefers-reduced-motion: reduce)");
    const update = () => setReduced(media.matches);
    media.addEventListener("change", update);
    return () => media.removeEventListener("change", update);
  }, []);
  return reduced;
}

export function BeeField({
  count = 24,
  repel = true,
  palette = DEFAULT_PALETTE,
  sizeScale = 1,
  bleed = 0,
  zIndex,
  clip = true,
}: {
  count?: number;
  repel?: boolean;
  palette?: string[];
  // Global multiplier on every bee's rendered size. The footer packs many more
  // bees, so it renders them smaller (sizeScale < 1) to match the dense field.
  sizeScale?: number;
  // Extra simulation space beyond the visible field. Bees are still clipped by
  // the section, but they can partially leave the viewport instead of banking
  // exactly at the edge.
  bleed?: number;
  // Stacking order of the field. Sections pass a value to place the swarm above
  // or below the wordmark — the hero runs two fields (behind + in front of the
  // "Buzz" text) for depth.
  zIndex?: number;
  // When false, the field doesn't clip its bees at its own bounds — paired with a
  // non-clipping parent section, bees drift past the edge (up to `bleed`px) so the
  // swarm bridges into the neighbouring section instead of hard-cropping.
  clip?: boolean;
}) {
  const maskPrefix = useId().replace(/[^a-zA-Z0-9_-]/g, "");
  const wrap = useRef<HTMLDivElement>(null);
  const reduce = useReducedMotion();
  // SSR + initial client render = count, matching the statically-exported HTML
  // (no hydration mismatch). The effect then halves it on narrow screens. This
  // is the single source of truth for both rendered bees and spawned bees.
  const [n, setN] = useState(count);
  useEffect(() => {
    const updateCount = () => {
      setN(window.innerWidth < 640 ? Math.ceil(count / 2) : count);
    };
    const raf = requestAnimationFrame(updateCount);
    window.addEventListener("resize", updateCount);
    return () => {
      cancelAnimationFrame(raf);
      window.removeEventListener("resize", updateCount);
    };
  }, [count]);

  // Per-bee render data, recomputed when count or palette changes.
  const renderedBees = Array.from({ length: n }, (_, i) => ({
    key: `${maskPrefix}-bee-${i}`,
    tint: tintOf(i, palette.length),
    index: i,
  }));

  useEffect(() => {
    const el = wrap.current;
    if (!el) return;
    // Source the bee nodes directly from the committed DOM (the rendered spans
    // are el's children) rather than from ref callbacks — ref timing relative to
    // this effect is fragile, and getting it wrong leaves every bee stacked at
    // the origin instead of spread across the field.
    const beeNodes = Array.from(el.children) as HTMLElement[];
    const bleedPx = Math.max(0, bleed);
    const w = el.clientWidth + bleedPx * 2,
      h = el.clientHeight + bleedPx * 2;
    let bees: Bee[] = spawnBees(n, w, h, bleedPx);
    const introFrom = bees.map((b, i) => {
      const angle = b.seed * TAU + i * 0.37;
      return {
        x: w / 2 + Math.cos(angle) * (w * 0.7 + bleedPx + BEE_W),
        y: h / 2 + Math.sin(angle) * (h * 0.7 + bleedPx + BEE_H),
      };
    });
    // Smoothed facing angle (deg) per bee, so bees bank toward their heading
    // instead of snapping. Seeded from the initial wander heading.
    const rot = bees.map((b) => (b.heading * 180) / Math.PI + ROT_OFFSET);
    let pointer: { x: number; y: number } | null = null;
    // Listen on WINDOW, not `el`: the wrapper is pointer-events:none (so it never
    // blocks clicks on the content beneath it), which means pointer events never
    // reach it — listening on `el` left repel permanently dead. We translate the
    // global cursor into field-local coordinates via the live bounding rect.
    const onMove = (e: PointerEvent) => {
      const r = el.getBoundingClientRect();
      pointer = {
        x: e.clientX - r.left + bleedPx,
        y: e.clientY - r.top + bleedPx,
      };
    };
    const onLeave = () => {
      pointer = null;
    };
    if (repel && !reduce) {
      window.addEventListener("pointermove", onMove, { passive: true });
      // Clear when the cursor leaves the document entirely.
      document.addEventListener("pointerleave", onLeave);
    }
    const paint = (introProgress = 1) =>
      beeNodes.forEach((node, i) => {
        const b = bees[i];
        if (node && b) {
          const easedIntro = 1 - (1 - introProgress) ** 3;
          const from = introFrom[i];
          const displayX = from ? from.x + (b.x - from.x) * easedIntro : b.x;
          const displayY = from ? from.y + (b.y - from.y) * easedIntro : b.y;
          node.style.transform = `translate(${displayX - bleedPx}px, ${displayY - bleedPx}px) rotate(${rot[i]}deg) scale(${sizeScale})`;
        }
      });
    if (reduce) {
      paint();
      return;
    } // freeze
    paint(0);
    let last = performance.now();
    const t0 = last;
    let raf = 0;
    const loop = (t: number) => {
      const dt = Math.min(0.05, (t - last) / 1000);
      last = t;
      // Elapsed seconds drive each bee's independent wander heading.
      bees = stepBees(bees, pointer, dt, w, h, repel, (t - t0) / 1000, bleedPx);
      // Rotate each bee toward its travel direction (shortest-arc lerp).
      bees.forEach((b, i) => {
        const speed = Math.hypot(b.vx, b.vy);
        if (speed > ROT_MIN_SPEED) {
          const target = (Math.atan2(b.vy, b.vx) * 180) / Math.PI + ROT_OFFSET;
          const diff = ((target - rot[i] + 540) % 360) - 180;
          rot[i] += diff * 0.18;
        }
      });
      paint(Math.min(1, (t - t0) / INTRO_MS));
      raf = requestAnimationFrame(loop);
    };
    raf = requestAnimationFrame(loop);
    return () => {
      cancelAnimationFrame(raf);
      window.removeEventListener("pointermove", onMove);
      document.removeEventListener("pointerleave", onLeave);
    };
  }, [n, repel, reduce, sizeScale, bleed]);

  return (
    <div
      ref={wrap}
      aria-hidden
      style={{
        position: "absolute",
        inset: 0,
        overflow: clip ? "hidden" : "visible",
        pointerEvents: "none",
        zIndex,
      }}
    >
      {renderedBees.map(({ key, tint, index }) => (
        <span
          key={key}
          style={{
            position: "absolute",
            top: 0,
            left: 0,
            width: BEE_W,
            height: BEE_H,
            color: palette[tint],
            transformOrigin: "center",
            willChange: "transform",
          }}
        >
          <svg
            className="bee-sprite"
            viewBox="0 0 466 309"
            aria-hidden
            width={BEE_W}
            height={BEE_H}
            style={
              {
                display: "block",
                overflow: "visible",
                "--flap-delay": flapDelayOf(index),
              } as CSSProperties
            }
            focusable="false"
          >
            <title>Animated bee</title>
            <path
              d={BEE_SPRITE_PATH}
              fill="currentColor"
              fillRule="evenodd"
              clipRule="evenodd"
            />
          </svg>
        </span>
      ))}
    </div>
  );
}
