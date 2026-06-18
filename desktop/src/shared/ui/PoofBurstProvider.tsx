import React, { type CSSProperties, useEffect, useRef, useState } from "react";

export const POOF_TRIGGER_CLASS = "buzz-poof-trigger";
export const POOF_ORIGIN_CLASS = "buzz-poof-origin";

export const POOF_DURATION_MS = 430;

const POOF_SOUND_URL = "/pow/plop.m4a";
const POOF_SIZE_SCALE = 0.6375;
const POOF_FRAMES = [
  { id: "poof-1", src: "/pow/poof1@3x.png" },
  { id: "poof-2", src: "/pow/poof2@3x.png" },
  { id: "poof-3", src: "/pow/poof3@3x.png" },
  { id: "poof-4", src: "/pow/poof4@3x.png" },
  { id: "poof-5", src: "/pow/poof5@3x.png" },
] as const;

let poofAudio: HTMLAudioElement | null = null;
let lastPointerDownTrigger: Element | null = null;

type PoofBurst = {
  id: number;
  size: number;
  x: number;
  y: number;
};

type PoofStyle = CSSProperties & {
  "--buzz-poof-size": string;
  "--buzz-poof-x": string;
  "--buzz-poof-y": string;
};

function getPoofOrigin(target: Element) {
  const origin = target.closest(`.${POOF_ORIGIN_CLASS}`) ?? target;
  const rect = origin.getBoundingClientRect();
  const baseSize = Math.min(Math.max(rect.width * 0.54, 104), 190);

  return {
    size: baseSize * POOF_SIZE_SCALE,
    x: rect.left + rect.width / 2,
    y: rect.top + rect.height / 2,
  };
}

function playPoofSound() {
  try {
    poofAudio ??= new Audio(POOF_SOUND_URL);
    poofAudio.volume = 0.34;
    poofAudio.currentTime = 0;
    poofAudio.play().catch(() => {
      // Best-effort — browsers can still block audio playback.
    });
  } catch {
    // Best-effort only: audio may be unavailable or blocked.
  }
}

export function PoofBurstProvider({ children }: { children: React.ReactNode }) {
  const [bursts, setBursts] = useState<PoofBurst[]>([]);
  const idRef = useRef(0);
  const timeoutIdsRef = useRef<number[]>([]);

  useEffect(() => {
    for (const frame of POOF_FRAMES) {
      const image = new Image();
      image.src = frame.src;
    }

    try {
      poofAudio ??= new Audio(POOF_SOUND_URL);
      poofAudio.preload = "auto";
      poofAudio.load();
    } catch {
      // Best-effort only.
    }
  }, []);

  useEffect(() => {
    function emitPoof(target: Element) {
      const id = idRef.current;
      idRef.current += 1;

      setBursts((current) => [
        ...current.slice(-5),
        { ...getPoofOrigin(target), id },
      ]);
      playPoofSound();

      const timeoutId = window.setTimeout(() => {
        setBursts((current) => current.filter((burst) => burst.id !== id));
      }, POOF_DURATION_MS);
      timeoutIdsRef.current.push(timeoutId);
    }

    function findTriggerTarget(event: Event) {
      return event.target instanceof Element
        ? event.target.closest(`.${POOF_TRIGGER_CLASS}`)
        : null;
    }

    function handleDocumentPointerDown(event: PointerEvent) {
      if (event.button !== 0) {
        return;
      }

      const target = findTriggerTarget(event);

      if (!target) {
        return;
      }

      lastPointerDownTrigger = target;
      window.setTimeout(() => {
        if (lastPointerDownTrigger === target) {
          lastPointerDownTrigger = null;
        }
      }, POOF_DURATION_MS);
      emitPoof(target);
    }

    function handleDocumentClick(event: MouseEvent) {
      const target =
        event.target instanceof Element
          ? event.target.closest(`.${POOF_TRIGGER_CLASS}`)
          : null;

      if (!target) {
        return;
      }

      if (lastPointerDownTrigger === target) {
        lastPointerDownTrigger = null;
        return;
      }

      emitPoof(target);
    }

    document.addEventListener("pointerdown", handleDocumentPointerDown, {
      capture: true,
    });
    document.addEventListener("click", handleDocumentClick, { capture: true });

    return () => {
      document.removeEventListener("pointerdown", handleDocumentPointerDown, {
        capture: true,
      });
      document.removeEventListener("click", handleDocumentClick, {
        capture: true,
      });
      for (const timeoutId of timeoutIdsRef.current) {
        window.clearTimeout(timeoutId);
      }
      timeoutIdsRef.current = [];
    };
  }, []);

  return (
    <>
      {children}
      <div aria-hidden="true" className="buzz-poof-layer">
        {bursts.map((burst) => (
          <div
            className="buzz-poof-burst"
            key={burst.id}
            style={
              {
                "--buzz-poof-size": `${burst.size}px`,
                "--buzz-poof-x": `${burst.x}px`,
                "--buzz-poof-y": `${burst.y}px`,
              } as PoofStyle
            }
          >
            {POOF_FRAMES.map((frame, index) => (
              <img
                alt=""
                className={`buzz-poof-frame buzz-poof-frame-${index + 1}`}
                decoding="async"
                draggable={false}
                key={`${burst.id}-${frame.id}`}
                src={frame.src}
              />
            ))}
          </div>
        ))}
      </div>
    </>
  );
}
