import * as React from "react";

import { ZorroHat } from "@/shared/ui/zorro-logo/ZorroHat";
import { SlashableHat } from "./SlashableHat";

type Hat = {
  top: string;
  left: string;
  size: number;
  rotate: number;
};

// Fixed perimeter lanes keep animated hats out of the onboarding content.
const HATS: Hat[] = [
  { top: "16%", left: "3%", size: 42, rotate: -12 },
  { top: "31%", left: "9%", size: 36, rotate: 18 },
  { top: "47%", left: "2%", size: 44, rotate: -20 },
  { top: "63%", left: "10%", size: 38, rotate: 14 },
  { top: "79%", left: "4%", size: 42, rotate: -8 },
  { top: "17%", left: "92%", size: 38, rotate: 16 },
  { top: "32%", left: "85%", size: 44, rotate: -14 },
  { top: "48%", left: "93%", size: 36, rotate: 10 },
  { top: "64%", left: "86%", size: 42, rotate: -18 },
  { top: "80%", left: "92%", size: 38, rotate: 12 },
  { top: "2%", left: "24%", size: 34, rotate: -10 },
  { top: "4%", left: "39%", size: 40, rotate: 16 },
  { top: "2%", left: "56%", size: 36, rotate: -20 },
  { top: "4%", left: "72%", size: 42, rotate: 8 },
  { top: "91%", left: "24%", size: 40, rotate: 14 },
  { top: "93%", left: "40%", size: 34, rotate: -16 },
  { top: "91%", left: "57%", size: 42, rotate: 20 },
  { top: "93%", left: "73%", size: 36, rotate: -8 },
];

// Autonomous wander: each hat drifts on its own smooth loop.
const WANDER_X = 10;
const WANDER_Y = 8;
const HIT_PADDING = 20;

export function LandingZees() {
  const fieldRef = React.useRef<HTMLDivElement>(null);
  const hatRefs = React.useRef<(HTMLSpanElement | null)[]>([]);
  const offsets = React.useRef(HATS.map(() => ({ x: 0, y: 0 })));

  React.useEffect(() => {
    const field = fieldRef.current;
    if (!field) return;

    let raf = 0;
    const start = performance.now();

    const tick = (now: number) => {
      const t = (now - start) / 1000;
      hatRefs.current.forEach((el, i) => {
        if (!el) return;
        const hat = HATS[i];
        // Per-hat wander: two incommensurate sine waves, phase-shifted by index.
        const phase = i * 1.7;
        const wx =
          Math.sin(t * (0.7 + (i % 5) * 0.13) + phase) * WANDER_X +
          Math.sin(t * 1.9 + phase * 2.1) * 3;
        const wy =
          Math.cos(t * (0.6 + (i % 7) * 0.11) + phase) * WANDER_Y +
          Math.cos(t * 2.3 + phase * 1.3) * 3;
        const target = { x: wx, y: wy };
        const cur = offsets.current[i];
        cur.x += (target.x - cur.x) * 0.12;
        cur.y += (target.y - cur.y) * 0.12;
        el.style.transform = `translate(${cur.x}px, ${cur.y}px) rotate(${hat.rotate}deg)`;
      });
      raf = requestAnimationFrame(tick);
    };

    const reduced = window.matchMedia("(prefers-reduced-motion: reduce)");
    if (!reduced.matches) {
      raf = requestAnimationFrame(tick);
    }
    return () => {
      if (raf) cancelAnimationFrame(raf);
    };
  }, []);

  return (
    <div
      ref={fieldRef}
      aria-hidden
      className="pointer-events-none absolute inset-0 overflow-hidden"
    >
      <span className="absolute left-6 top-12 block w-11">
        <ZorroHat className="h-auto w-full" />
      </span>
      {HATS.map((hat, i) => (
        <span
          key={`${hat.top}-${hat.left}`}
          className="zorro-zee-hitbox pointer-events-auto absolute block"
          data-testid={`landing-floating-hat-${i}`}
          style={{
            top: hat.top,
            left: hat.left,
            width: hat.size + HIT_PADDING * 2,
            height: hat.size + HIT_PADDING * 2,
            transform: `translate(-${HIT_PADDING}px, -${HIT_PADDING}px)`,
          }}
        >
          <SlashableHat
            ref={(el) => {
              hatRefs.current[i] = el;
            }}
            className="absolute block will-change-transform"
            style={{
              top: HIT_PADDING,
              left: HIT_PADDING,
              width: hat.size,
              transform: `rotate(${hat.rotate}deg)`,
              opacity: 0.78,
            }}
          />
        </span>
      ))}
    </div>
  );
}
