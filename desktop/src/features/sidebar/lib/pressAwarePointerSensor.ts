import { PointerSensor } from "@dnd-kit/core";
import type { PointerSensorOptions } from "@dnd-kit/core";
import type { PointerEvent as ReactPointerEvent } from "react";

/**
 * A {@link PointerSensor} that refuses a `pointerdown` reporting no button
 * held, instead of arming a drag it will never see released.
 *
 * dnd-kit starts a drag from pointer travel alone and never re-reads the button
 * state, so a `pointerup` the page does not receive leaves the sensor armed:
 * the next cursor move begins a drag nobody asked for, and the click that move
 * belonged to is swallowed, because dnd-kit installs a capture-phase `click`
 * blocker as soon as a drag activates.
 *
 * The macOS webview does exactly that with tap-to-click. A tap is recognised
 * after the finger has already lifted, so `pointerdown` arrives with
 * `buttons: 0` and the matching `pointerup` is deferred — measured at 356ms to
 * over 1.2s, and sometimes not before the next input at all. A deliberate
 * press-and-hold reports `buttons: 1` throughout, which is what makes the two
 * separable at the one moment that decides everything: over 38 measured
 * gestures, every `buttons: 0` press was a click and every `buttons: 1` press
 * was a real drag, with nothing in between.
 *
 * So the fix is to not start. dnd-kit only instantiates a sensor when the
 * activator returns exactly `true` (`@dnd-kit/core` 6.3.1,
 * `bindActivatorToSensorInstantiator`), and reads `activators` off the sensor
 * class, so declining here means no session, no click blocker, and nothing to
 * unwind. The base handler is delegated to for everything else — it is what
 * rejects non-primary and non-left presses, and what fires `onActivation`.
 */
export class PressAwarePointerSensor extends PointerSensor {
  static activators = [
    {
      eventName: "onPointerDown" as const,
      handler: (
        event: ReactPointerEvent,
        options: PointerSensorOptions,
      ): boolean =>
        event.nativeEvent.buttons !== 0 &&
        PointerSensor.activators[0].handler(event, options),
    },
  ];
}
