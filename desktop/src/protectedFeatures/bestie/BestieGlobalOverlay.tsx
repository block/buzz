import * as React from "react";
import { LayoutGroup, motion } from "motion/react";
import { createPortal } from "react-dom";

import { Bloom } from "./BloomMenu";
import { BestiePopover, BestieTriggerVisual } from "./BestiePopover";
import { useBestie } from "./useBestie";

type Point = { x: number; y: number };
type Placement = {
  anchor: "start" | "end";
  direction: "top" | "bottom";
};
type DragBounds = {
  maxX: number;
  maxY: number;
  minX: number;
  minY: number;
};
const AVATAR_SIZE = 48;
const EDGE_INSET = 16;
const BESTIE_AVATAR_LAYOUT_ID = "bestie-floating-bloom-avatar";

function clampPoint(point: Point): Point {
  return {
    x: Math.min(
      Math.max(EDGE_INSET, point.x),
      Math.max(EDGE_INSET, window.innerWidth - AVATAR_SIZE - EDGE_INSET),
    ),
    y: Math.min(
      Math.max(EDGE_INSET, point.y),
      Math.max(EDGE_INSET, window.innerHeight - AVATAR_SIZE - EDGE_INSET),
    ),
  };
}

function initialPoint(): Point {
  return clampPoint({ x: window.innerWidth - AVATAR_SIZE - 20, y: 52 });
}

export function BestieGlobalOverlay() {
  const bestie = useBestie();
  const [open, setOpen] = React.useState(false);
  const [placement, setPlacement] = React.useState<Placement | null>(null);
  const [shareAvatarLayout, setShareAvatarLayout] = React.useState(true);
  const [dragging, setDragging] = React.useState(false);
  const [position, setPosition] = React.useState<Point>(initialPoint);
  const dragRef = React.useRef<
    | {
        bounds: DragBounds;
        moved: boolean;
        open: boolean;
        origin: Point;
        pointerId: number;
        start: Point;
      }
    | undefined
  >(undefined);
  const suppressClickRef = React.useRef(false);

  React.useEffect(() => {
    const handleResize = () => setPosition((current) => clampPoint(current));
    window.addEventListener("resize", handleResize);
    return () => window.removeEventListener("resize", handleResize);
  }, []);

  const currentPlacement: Placement = {
    direction:
      position.y > (window.innerHeight - AVATAR_SIZE) / 2 ? "top" : "bottom",
    anchor:
      position.x > (window.innerWidth - AVATAR_SIZE) / 2 ? "end" : "start",
  };
  const activePlacement = open && placement ? placement : currentPlacement;

  const updateOpen = (nextOpen: boolean) => {
    setShareAvatarLayout(nextOpen);
    if (nextOpen) setPlacement(currentPlacement);
    setOpen(nextOpen);
  };

  return createPortal(
    <div
      className="pointer-events-auto fixed z-[200]"
      data-testid="bestie-floating-avatar"
      onPointerCancel={(event) => {
        const drag = dragRef.current;
        if (!drag || drag.pointerId !== event.pointerId) return;
        dragRef.current = undefined;
        setDragging(false);
        if (event.currentTarget.hasPointerCapture(event.pointerId)) {
          event.currentTarget.releasePointerCapture(event.pointerId);
        }
      }}
      onPointerDown={(event) => {
        if (event.button !== 0) return;
        const target = event.target as HTMLElement;
        if (
          open &&
          (!target.closest("[data-bestie-drag-handle]") ||
            target.closest("button, input, textarea, a"))
        ) {
          return;
        }
        const surface = open
          ? event.currentTarget
              .querySelector('[data-testid="bestie-bloom-content"]')
              ?.getBoundingClientRect()
          : event.currentTarget.getBoundingClientRect();
        if (!surface) return;
        event.preventDefault();
        event.currentTarget.setPointerCapture(event.pointerId);
        setDragging(true);
        setShareAvatarLayout(false);
        dragRef.current = {
          bounds: {
            maxX: window.innerWidth - EDGE_INSET - surface.right,
            maxY: window.innerHeight - EDGE_INSET - surface.bottom,
            minX: EDGE_INSET - surface.left,
            minY: EDGE_INSET - surface.top,
          },
          open,
          pointerId: event.pointerId,
          origin: position,
          start: { x: event.clientX, y: event.clientY },
          moved: false,
        };
      }}
      onPointerMove={(event) => {
        const drag = dragRef.current;
        if (!drag || drag.pointerId !== event.pointerId) return;
        const rawX = event.clientX - drag.start.x;
        const rawY = event.clientY - drag.start.y;
        const dx = Math.min(drag.bounds.maxX, Math.max(drag.bounds.minX, rawX));
        const dy = Math.min(drag.bounds.maxY, Math.max(drag.bounds.minY, rawY));
        if (Math.hypot(dx, dy) > 4) drag.moved = true;
        setPosition(
          clampPoint({ x: drag.origin.x + dx, y: drag.origin.y + dy }),
        );
      }}
      onPointerUp={(event) => {
        const drag = dragRef.current;
        if (!drag || drag.pointerId !== event.pointerId) return;
        if (!drag.open) suppressClickRef.current = drag.moved;
        dragRef.current = undefined;
        setDragging(false);
        if (event.currentTarget.hasPointerCapture(event.pointerId)) {
          event.currentTarget.releasePointerCapture(event.pointerId);
        }
        if (!drag.open && !drag.moved) updateOpen(true);
      }}
      style={{
        left: 0,
        top: 0,
        transform: `translate3d(${position.x}px, ${position.y}px, 0)`,
        willChange: "transform",
      }}
    >
      <LayoutGroup id="bestie-floating-bloom">
        <Bloom.Root
          anchor={activePlacement.anchor}
          direction={activePlacement.direction}
          onOpenChange={(nextOpen) => {
            if (nextOpen && suppressClickRef.current) {
              suppressClickRef.current = false;
              return;
            }
            updateOpen(nextOpen);
          }}
          open={open}
        >
          <Bloom.Container
            buttonSize={AVATAR_SIZE}
            className="border border-border/70 bg-popover text-popover-foreground ring-1 ring-foreground/5"
            edgeDraggable
            menuRadius={20}
            menuWidth={320}
            motionDisabled={dragging}
            onMorphAnimationComplete={(isOpen) => {
              if (isOpen) setShareAvatarLayout(false);
            }}
          >
            <Bloom.Trigger
              ariaLabel="Open Bestie"
              className="flex h-12 w-12 touch-none select-none items-center justify-center rounded-full border-0 bg-transparent p-0 focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring [&_img]:pointer-events-none"
            >
              <motion.div
                key={shareAvatarLayout ? "shared-avatar" : "static-avatar"}
                layoutId={
                  shareAvatarLayout ? BESTIE_AVATAR_LAYOUT_ID : undefined
                }
              >
                <BestieTriggerVisual
                  agent={bestie.assignedAgent}
                  imageDraggable={false}
                />
              </motion.div>
            </Bloom.Trigger>
            <Bloom.Content ariaLabel="Bestie" className="p-4">
              <BestiePopover
                avatarLayoutId={
                  shareAvatarLayout ? BESTIE_AVATAR_LAYOUT_ID : undefined
                }
                onRequestClose={() => updateOpen(false)}
              />
            </Bloom.Content>
          </Bloom.Container>
        </Bloom.Root>
      </LayoutGroup>
    </div>,
    document.body,
  );
}
