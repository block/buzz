import { motion, useIsPresent, useReducedMotion } from "motion/react";
import * as React from "react";

import {
  claimCoverDrawerFocus,
  hasCoverDrawerFocusClaim,
} from "@/features/channels/lib/coverDrawerFocusSlot";
import {
  COVER_DRAWER_SLIVER_WIDTH_PX,
  COVER_DRAWER_TRAVEL_PX,
} from "@/features/channels/lib/coverDrawerLayout";
import { cn } from "@/shared/lib/cn";

type CoverDrawerProps = {
  /** Accessible name for the drawer surface itself. */
  ariaLabel: string;
  children: React.ReactNode;
  onClose: () => void;
  /**
   * Whether the drawer claims Escape for itself, ahead of anything inside it.
   *
   * Claiming it means a single press always leaves, even from a nested control
   * that would otherwise handle the key. Leave this off when the drawer's own
   * content already closes on Escape through `useEscapeKey`, which yields to
   * nested controls that mark the event handled. Defaults to claiming.
   */
  ownsEscape?: boolean;
  /**
   * Whether content inside the drawer currently owns Escape ahead of the
   * drawer's own claim.
   *
   * Only meaningful while `ownsEscape` is set. A capture-phase claim runs before
   * anything inside the drawer, so a drawer that unconditionally closes on
   * Escape would dismiss the whole surface out from under an in-progress edit
   * instead of letting that edit cancel first — losing the draft. Setting this
   * yields the press to the drawer's own subtree for exactly that case; presses
   * from outside the drawer still close it, so it cannot be wedged open.
   */
  escapeYieldsToContent?: boolean;
  /** Accessible name for the scrim, which is the click target back to the channel. */
  scrimLabel: string;
  /**
   * Test id of the drawer surface. The overlay and scrim derive theirs from it
   * (`-overlay`, `-scrim`) so one id names the whole presentation.
   */
  testId: string;
};

/**
 * Scrim over the channel content area behind a cover drawer.
 *
 * Veil, not shadow, and no blur: the channel fades toward the surface colour
 * rather than being darkened. A black wash is a multiply — it scales text and
 * background down together, so dark-on-light text keeps its contrast ratio and
 * stays readable at any opacity short of a solid bar. Fading toward
 * `background` instead compresses text against the surface in both themes,
 * which is what pushes the sliver back to colour and shape. Matches the shared
 * header backdrop's `bg-background/80` vocabulary, a touch heavier because this
 * one has to defeat body text rather than sit over a gap.
 */
const COVER_SCRIM_CLASS = "bg-background/75 dark:bg-background/80";

/**
 * Hover eases the veil one step in both themes.
 *
 * Feedback that the sliver is a target — deliberately not a peek: one step is
 * enough to register as interactive without making the channel readable.
 */
const COVER_SCRIM_HOVER_CLASS =
  "hover:bg-background/65 dark:hover:bg-background/70";

/** Arrive and settle. The iOS sheet curve, shared with `buzz-side-panel-enter`. */
const ENTER_EASE = [0.32, 0.72, 0, 1] as const;

/**
 * Leave immediately. Shares the enter's fast-start shape rather than the
 * conventional accelerating ease-in for exits.
 *
 * The "exits accelerate away" rule assumes the whole travel is visible; an
 * ease-in spends its opening frames barely moving and pays that back at the end.
 * Here the tail is hidden under the opacity fade, so acceleration buys nothing
 * and those opening frames are the entire perception of responsiveness — a
 * dismissal that hasn't visibly moved 40ms in reads as hesitation regardless of
 * its total duration. Decisiveness comes from the duration below instead.
 */
const EXIT_EASE = ENTER_EASE;

const SCRIM_ENTER_SECONDS = 0.2;

/**
 * Slightly ahead of the drawer's exit, and deliberately so.
 *
 * A scrim that outlasts the drawer leaves the channel dimmed with nothing on top
 * of it, which reads as lag at the exact moment the user has committed to
 * leaving. Undimming first hands the channel back the instant it is asked for.
 */
const SCRIM_EXIT_SECONDS = 0.12;

/**
 * Enter: opacity front-loaded, transform long.
 *
 * The two channels animate over deliberately different windows, and that
 * asymmetry is the whole point. Short travel *requires* an opacity fade — an
 * opaque surface this large appearing 120px off its mark with no fade is a hard
 * cut, not a slide. But pairing both properties on one timing function (as a
 * single CSS keyframe must) welds them together for the full duration, and since
 * opacity covers 100% of its range while transform covers ~3% of the drawer's
 * width, the fade is what the eye reads. Resolving opacity in the first ~90ms
 * leaves the remaining ~190ms as pure travel: the fade is over before it
 * registers, and what's perceived is sliding.
 *
 * It also keeps the drawer's own entrance from exposing its contents' load
 * order. Anything arriving late (replies resolving, media decoding) lands on an
 * already-opaque surface and reads as "the panel is loading" rather than the UI
 * assembling itself.
 */
const ENTER_TRANSITION = {
  opacity: { duration: 0.09, ease: "linear" },
  x: { duration: 0.28, ease: ENTER_EASE },
} as const;

/**
 * Exit: half the enter's duration, opacity barely back-loaded.
 *
 * Opening and closing are not symmetric tasks. The enter has something to say —
 * it establishes where the panel came from and that the channel is still behind
 * it. The exit has nothing to say: attention has already left for the channel,
 * so its only job is to get out of the way without popping. That makes duration
 * the thing to spend, and 140ms is about the floor before the drawer reads as
 * vanishing rather than leaving.
 *
 * The opacity hold shrinks with it. Its purpose is to let the drawer commit to
 * moving before it dissolves, so it reads as sliding out — but at this duration a
 * hold proportional to the old one would eat half the animation. 20ms is enough
 * to register solidity in the first frame or two.
 */
const EXIT_TRANSITION = {
  opacity: { delay: 0.02, duration: 0.12, ease: "linear" },
  x: { duration: 0.14, ease: EXIT_EASE },
} as const;

/**
 * Reduced motion keeps a crossfade and drops the travel.
 *
 * Travel is the part that's motion; the fade is what makes appearing and
 * disappearing legible. With `x` pinned to 0 the front/back-loaded opacity
 * timings would read as dead air on a stationary surface, so both collapse to
 * one short symmetric fade.
 */
const REDUCED_MOTION_TRANSITION = { duration: 0.12, ease: "linear" } as const;

/**
 * Right-anchored drawer that overlays the channel content area.
 *
 * Presentation only — it knows nothing about what it covers the channel with.
 * The thread focus drawer and the agent activity drawer are both this surface
 * with different contents and different open conditions.
 *
 * Must be rendered inside `ChannelPane`'s relative layout root, and beneath an
 * `AnimatePresence` so the exit animation can run: everything here is absolutely
 * positioned against the channel content area, so the app sidebar is never
 * covered. The channel stays mounted underneath — a narrow scrim-dimmed sliver
 * of it remains visible for depth, and the whole scrim (sliver included) is one
 * tall click target back to the channel. Orientation lives in the drawer's own
 * header, where the eye already is — the sliver carries no label of its own.
 *
 * `z-41` places the drawer above the channel section (whose inner `isolate`
 * wrapper traps the timeline's z-50 pill, z-40 composer overlay, and z-50 drop
 * overlay) and the `z-30` shared header backdrop, while staying below the
 * global `z-45` top chrome. Setting z-index on the positioned container also
 * gives the drawer its own stacking context, so the panel chrome inside is
 * isolated.
 */
export function CoverDrawer({
  ariaLabel,
  children,
  escapeYieldsToContent = false,
  onClose,
  ownsEscape = true,
  scrimLabel,
  testId,
}: CoverDrawerProps) {
  const prefersReducedMotion = useReducedMotion();
  /**
   * False from the moment `AnimatePresence` starts this drawer's exit.
   *
   * The covered slot belongs to the drawer that is arriving or settled, not to
   * one that is animating away, and this is the only signal that distinguishes
   * them — the focus slot cannot, because a drawer that never captures focus
   * (its content may take it instead) leaves the outgoing drawer's claim
   * current. See the Escape handler.
   */
  const isPresent = useIsPresent();
  const travelPx = prefersReducedMotion ? 0 : COVER_DRAWER_TRAVEL_PX;
  const drawerRef = React.useRef<HTMLDivElement>(null);
  const previousFocusRef = React.useRef<HTMLElement | null>(null);
  /**
   * Whether the opener has been captured for this drawer instance.
   *
   * Distinct from `previousFocusRef.current === null`, which is a legitimate
   * capture (nothing was focused) and must not be retried. See the capture
   * effect for why one attempt is all this gets.
   */
  const hasCapturedPreviousFocusRef = React.useRef(false);

  React.useEffect(() => {
    if (!ownsEscape) return;
    // Stand down for the whole exit: a drawer on its way out does not own the
    // covered slot, so the key belongs to whatever replaced it.
    //
    // `AnimatePresence` keeps a replaced drawer mounted through its exit
    // animation, so during a replacement two drawers have this listener
    // installed at once, and capture-phase listeners on the same target fire in
    // registration order — the outgoing one registered first, so it would
    // otherwise always win. It then consumes the press via
    // `stopImmediatePropagation`, which is invisible to the successor, and the
    // user has to press Escape twice to leave the drawer that just arrived.
    //
    // `useEscapeKey` carries the same guard for the same reason. This one is not
    // sufficient on its own: the agent activity drawer sets `ownsEscape={false}`
    // and routes the key through its panel, so on that path no code here runs
    // and it is the exiting *panel*'s `preventDefault` that swallows the press.
    //
    // Gating on presence rather than the focus slot is deliberate. The focus
    // slot is claimed only by a drawer that captures focus, and a successor
    // whose content takes focus instead never claims it — which leaves the
    // outgoing drawer's claim current and makes a slot check pass for exactly
    // the drawer that should stand down. Presence is the state that actually
    // distinguishes arriving from leaving.
    if (!isPresent) return;

    function handleEscape(event: KeyboardEvent) {
      if (event.key !== "Escape") return;
      // Yield to an in-progress edit inside the drawer: the capture-phase claim
      // runs first, so without this the press would dismiss the whole surface
      // and lose the draft instead of cancelling the edit. Scoped to the
      // drawer's own subtree, so a press from outside still closes it.
      const target = event.target;
      if (
        escapeYieldsToContent &&
        target instanceof Node &&
        drawerRef.current?.contains(target)
      ) {
        return;
      }
      event.preventDefault();
      event.stopImmediatePropagation();
      onClose();
    }

    window.addEventListener("keydown", handleEscape, { capture: true });
    return () => {
      window.removeEventListener("keydown", handleEscape, { capture: true });
    };
  }, [escapeYieldsToContent, isPresent, onClose, ownsEscape]);

  React.useLayoutEffect(() => {
    // Capture the opener exactly once per drawer instance.
    //
    // `React.StrictMode` replays effects in development as setup → cleanup →
    // setup, and by that second setup this drawer has already focused itself. An
    // unconditional read of `document.activeElement` would therefore record the
    // drawer as its own opener, and a real close would focus a node React has
    // since detached — leaving focus on `<body>`, keyboard-stranded. Refs survive
    // the replay, so a one-shot flag is enough. The flag is deliberately not
    // reset in cleanup: the only cleanup it would see before a real close is the
    // simulated one, which is exactly what it exists to ignore.
    //
    // Re-claiming the focus slot on the replayed setup is correct and stays as
    // is — that new generation is what makes the first cleanup's deferred
    // restore stand down.
    if (!hasCapturedPreviousFocusRef.current) {
      hasCapturedPreviousFocusRef.current = true;
      previousFocusRef.current =
        document.activeElement instanceof HTMLElement
          ? document.activeElement
          : null;
    }
    const focusClaim = claimCoverDrawerFocus();
    drawerRef.current?.focus({ preventScroll: true });

    return () => {
      const previousFocus = previousFocusRef.current;
      requestAnimationFrame(() => {
        // Deferred by a frame so the exit animation can start, which is exactly
        // long enough for a replacing drawer to mount and take focus. Restore
        // only while this drawer still holds the slot; otherwise the successor
        // owns focus and restoring would drop it into the inert channel.
        if (!hasCoverDrawerFocusClaim(focusClaim)) return;
        previousFocus?.focus({ preventScroll: true });
      });
    };
  }, []);

  return (
    <div className="absolute inset-0 z-41" data-testid={`${testId}-overlay`}>
      <motion.button
        animate={{ opacity: 1 }}
        aria-label={scrimLabel}
        className={cn(
          "absolute inset-0 cursor-pointer transition-colors duration-150",
          COVER_SCRIM_CLASS,
          COVER_SCRIM_HOVER_CLASS,
        )}
        data-testid={`${testId}-scrim`}
        exit={{
          opacity: 0,
          transition: prefersReducedMotion
            ? REDUCED_MOTION_TRANSITION
            : { duration: SCRIM_EXIT_SECONDS, ease: "linear" },
        }}
        initial={{ opacity: 0 }}
        onClick={onClose}
        transition={
          prefersReducedMotion
            ? REDUCED_MOTION_TRANSITION
            : { duration: SCRIM_ENTER_SECONDS, ease: "linear" }
        }
        type="button"
      />

      <motion.div
        animate={{ opacity: 1, x: 0 }}
        className={cn(
          // Left corners only, at the app content surface's own `rounded-2xl`:
          // the drawer is flush to that surface's right edge, so it is *clipped*
          // to its right corners rather than nesting inside them. Flush edges
          // share a radius — a smaller one here would put two radii on one
          // element. `shadow-panel-left` draws the left edge and its corners;
          // see the token for why a `border-l` cannot.
          // `outline-hidden`: the drawer is `tabIndex={-1}` and focuses itself
          // on open purely to land the keyboard inside it, so the focus ring
          // that would draw around the whole surface is noise, not affordance.
          "absolute inset-y-0 right-0 flex flex-col overflow-hidden rounded-l-2xl bg-background shadow-panel-left outline-hidden",
        )}
        aria-label={ariaLabel}
        data-testid={testId}
        ref={drawerRef}
        role="complementary"
        tabIndex={-1}
        exit={{
          opacity: 0,
          transition: prefersReducedMotion
            ? REDUCED_MOTION_TRANSITION
            : EXIT_TRANSITION,
          x: travelPx,
        }}
        initial={{ opacity: 0, x: travelPx }}
        style={{ left: COVER_DRAWER_SLIVER_WIDTH_PX }}
        transition={
          prefersReducedMotion ? REDUCED_MOTION_TRANSITION : ENTER_TRANSITION
        }
      >
        <div className="flex min-h-0 flex-1 flex-col">{children}</div>
      </motion.div>
    </div>
  );
}
