// Sidebar drag sensors: mouse/pen drags exactly as before (PointerSensor, 6px), while touch needs a long press so a swipe scrolls
// the panel instead of grabbing the row (iOS/iPad). Touch pointers are ignored by the pointer sensor and handled by TouchSensor.
import { PointerSensor, TouchSensor, useSensor } from "@dnd-kit/core";
import type { PointerEvent as ReactPointerEvent } from "react";

export class NonTouchPointerSensor extends PointerSensor {
  static override activators = [
    {
      eventName: "onPointerDown" as const,
      handler: (event: ReactPointerEvent, options: Parameters<(typeof PointerSensor.activators)[0]["handler"]>[1]) =>
        event.nativeEvent.pointerType !== "touch" && PointerSensor.activators[0].handler(event, options),
    },
  ];
}

/** Long-press before a touch drag starts; moving more than `tolerance` px first lets the scroll win. */
export const TOUCH_DRAG_DELAY_MS = 350;
export const TOUCH_DRAG_TOLERANCE_PX = 8;

export function useSidebarDragSensors() {
  return [
    useSensor(NonTouchPointerSensor, { activationConstraint: { distance: 6 } }),
    useSensor(TouchSensor, { activationConstraint: { delay: TOUCH_DRAG_DELAY_MS, tolerance: TOUCH_DRAG_TOLERANCE_PX } }),
  ] as const;
}
