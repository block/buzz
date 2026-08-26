import * as React from "react";

import { BuzzMark } from "@/shared/ui/buzz-logo/BuzzMark";
import { FlappingBee } from "@/shared/ui/buzz-logo/FlappingBee";

type Bee = {
  top: string;
  left: string;
  size: number;
  rotate: number;
  color: string;
};

const WHITE = "#FFFFFF";

// Orbit brand colors
const YELLOW = "#A713E8";
const ORBIT_BLUE = "#3D42D9";
const ORBIT_PURPLE = "#7C2BE2";
const ORBIT_PINK = "#D60FD9";

/**
 * Fixed scatter.
 *
 * Positions are deliberately spaced so logos don't initially
 * render on top of each other.
 *
 * We also use collision avoidance during animation below.
 */
const BEES: Bee[] = [
  // Top Left Area
  { top: "10%", left: "12%", size: 42, rotate: -10, color: WHITE },
  { top: "25%", left: "22%", size: 38, rotate: 8, color: ORBIT_PURPLE },

  // Top Right Area
  { top: "8%", left: "75%", size: 44, rotate: 12, color: ORBIT_BLUE },
  { top: "22%", left: "88%", size: 40, rotate: -6, color: WHITE },

  // Mid Left Area
  { top: "50%", left: "10%", size: 46, rotate: 15, color: ORBIT_PINK },

  // Mid Right Area
  { top: "48%", left: "85%", size: 42, rotate: -8, color: ORBIT_BLUE },

  // Bottom Left Area
  { top: "75%", left: "18%", size: 40, rotate: -12, color: YELLOW },
  { top: "88%", left: "8%", size: 44, rotate: 6, color: WHITE },

  // Bottom Right Area
  { top: "75%", left: "80%", size: 42, rotate: 10, color: ORBIT_PINK },
];

const REPEL_RADIUS = 180;
const REPEL_STRENGTH = 110;

/**
 * Keep wander fairly small.
 *
 * Large wandering is what makes otherwise well-spaced icons
 * crash into each other.
 */
const WANDER_X = 16;
const WANDER_Y = 13;

/**
 * Additional space between integration cards.
 */
const COLLISION_PADDING = 14;

const COLLISION_STRENGTH = 0.18;

export function LandingBees() {
  const fieldRef = React.useRef<HTMLDivElement>(null);

  const beeRefs = React.useRef<
    (HTMLSpanElement | null)[]
  >([]);

  const pointer = React.useRef<{
    x: number;
    y: number;
  } | null>(null);

  const offsets = React.useRef(
    BEES.map(() => ({
      x: 0,
      y: 0,
    })),
  );

  React.useEffect(() => {
    const field = fieldRef.current;

    if (!field) return;

    let raf = 0;

    const start = performance.now();

    const tick = (now: number) => {
      const t = (now - start) / 1000;

      const rect = field.getBoundingClientRect();

      const p = pointer.current;

      /**
       * Calculate the desired positions first.
       */
      const targets = BEES.map((bee, i) => {
        const phase = i * 1.7;

        const wx =
          Math.sin(
            t * (0.55 + (i % 5) * 0.08) +
              phase,
          ) *
            WANDER_X +
          Math.sin(
            t * 1.3 + phase * 2.1,
          ) *
            3;

        const wy =
          Math.cos(
            t * (0.5 + (i % 7) * 0.07) +
              phase,
          ) *
            WANDER_Y +
          Math.cos(
            t * 1.5 + phase * 1.3,
          ) *
            3;

        let rx = 0;
        let ry = 0;

        /**
         * Mouse repulsion.
         */
        if (p) {
          const cx =
            rect.left +
            (rect.width *
              parseFloat(bee.left)) /
              100 +
            offsets.current[i].x;

          const cy =
            rect.top +
            (rect.height *
              parseFloat(bee.top)) /
              100 +
            offsets.current[i].y;

          const ox = cx - p.x;
          const oy = cy - p.y;

          const dist = Math.hypot(ox, oy);

          if (
            dist < REPEL_RADIUS &&
            dist > 0.01
          ) {
            const push =
              ((REPEL_RADIUS - dist) /
                REPEL_RADIUS) *
              REPEL_STRENGTH;

            rx = (ox / dist) * push;
            ry = (oy / dist) * push;
          }
        }

        return {
          x: wx + rx,
          y: wy + ry,
        };
      });

      /**
       * -----------------------------------------------------
       * LOGO-TO-LOGO COLLISION AVOIDANCE
       * -----------------------------------------------------
       *
       * Every pair is checked.
       *
       * If their visual bounds get too close, both particles
       * receive an equal push in opposite directions.
       */
      for (
        let i = 0;
        i < BEES.length;
        i++
      ) {
        for (
          let j = i + 1;
          j < BEES.length;
          j++
        ) {
          const beeA = BEES[i];
          const beeB = BEES[j];

          const ax =
            (rect.width *
              parseFloat(beeA.left)) /
              100 +
            targets[i].x;

          const ay =
            (rect.height *
              parseFloat(beeA.top)) /
              100 +
            targets[i].y;

          const bx =
            (rect.width *
              parseFloat(beeB.left)) /
              100 +
            targets[j].x;

          const by =
            (rect.height *
              parseFloat(beeB.top)) /
              100 +
            targets[j].y;

          const dx = ax - bx;
          const dy = ay - by;

          const distance =
            Math.hypot(dx, dy);

          const minimumDistance =
            beeA.size / 2 +
            beeB.size / 2 +
            COLLISION_PADDING;

          if (
            distance < minimumDistance &&
            distance > 0.01
          ) {
            const overlap =
              minimumDistance - distance;

            const nx = dx / distance;
            const ny = dy / distance;

            const push =
              overlap *
              COLLISION_STRENGTH;

            targets[i].x += nx * push;
            targets[i].y += ny * push;

            targets[j].x -= nx * push;
            targets[j].y -= ny * push;
          }
        }
      }

      /**
       * Apply final positions.
       */
      beeRefs.current.forEach(
        (el, i) => {
          if (!el) return;

          const bee = BEES[i];

          const target = targets[i];

          const cur =
            offsets.current[i];

          /**
           * Smooth movement instead of snapping.
           */
          cur.x +=
            (target.x - cur.x) *
            0.09;

          cur.y +=
            (target.y - cur.y) *
            0.09;

          el.style.transform = `
            translate3d(
              ${cur.x}px,
              ${cur.y}px,
              0
            )
            rotate(${bee.rotate}deg)
          `;
        },
      );

      raf =
        requestAnimationFrame(tick);
    };

    const onMove = (
      event: MouseEvent,
    ) => {
      pointer.current = {
        x: event.clientX,
        y: event.clientY,
      };
    };

    const onLeave = () => {
      pointer.current = null;
    };

    const reduced =
      window.matchMedia(
        "(prefers-reduced-motion: reduce)",
      );

    if (!reduced.matches) {
      raf =
        requestAnimationFrame(tick);

      window.addEventListener(
        "mousemove",
        onMove,
      );

      window.addEventListener(
        "mouseout",
        onLeave,
      );
    }

    return () => {
      window.removeEventListener(
        "mousemove",
        onMove,
      );

      window.removeEventListener(
        "mouseout",
        onLeave,
      );

      if (raf) {
        cancelAnimationFrame(raf);
      }
    };
  }, []);

  return (
    <div
      ref={fieldRef}
      aria-hidden
      className="
        pointer-events-none
        absolute
        inset-0
        overflow-hidden
      "
    >
      {/* =====================================================
          ORBIT LOGO
      ===================================================== */}

      <span
        className="
          absolute
          left-6
          top-12
          z-20
          block
          w-14
        "
      >
        <BuzzMark className="h-auto w-full" />
      </span>

      {/* =====================================================
          INTEGRATIONS
      ===================================================== */}

      {BEES.map((bee, i) => (
        <span
          key={`${bee.top}-${bee.left}`}
          ref={(el) => {
            beeRefs.current[i] = el;
          }}
          className="
            absolute
            block
            will-change-transform
          "
          style={{
            top: bee.top,
            left: bee.left,

            width: bee.size,
            height: bee.size,

            color: bee.color,

            transform: `rotate(${bee.rotate}deg)`,

            opacity: 0.92,

            /**
             * Important:
             *
             * Position coordinates represent the CENTER
             * of each integration rather than its top-left
             * corner.
             */
            marginLeft:
              -(bee.size / 2),

            marginTop:
              -(bee.size / 2),
          }}
        >
          <FlappingBee
            className="w-full"
            index={i}
          />
        </span>
      ))}
    </div>
  );
}