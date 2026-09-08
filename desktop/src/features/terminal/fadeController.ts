export const REVEAL_DURATION_MS = 360;
export const CONCEAL_DURATION_MS = 210;
export const FADE_EASING = "cubic-bezier(.2,.8,.2,1)";

type Direction = "reveal" | "conceal";

type AnimationLike = {
  currentTime: CSSNumberish | null;
  playbackRate: number;
  play(): void;
  reverse(): void;
  cancel(): void;
  finished: Promise<unknown>;
};

type SurfaceLike = {
  style: Pick<CSSStyleDeclaration, "opacity" | "willChange" | "removeProperty">;
  animate(
    keyframes: Keyframe[] | PropertyIndexedKeyframes | null,
    options?: number | KeyframeAnimationOptions,
  ): AnimationLike;
};

/** One reversible opacity timeline; toggles never queue a second animation. */
export class FadeController {
  #surface: SurfaceLike;
  #animation: AnimationLike | null = null;
  #direction: Direction = "conceal";
  #token = 0;

  constructor(surface: SurfaceLike) {
    this.#surface = surface;
  }

  /**
   * The fade owns `will-change` only for as long as it is animating. The app
   * surface also carries an authored `will-change` for the huddle drawer
   * transition, and an inline declaration outranks the stylesheet — so writing
   * the literal `"auto"` does not "clear" the hint, it permanently replaces
   * the drawer's. Removing the inline property hands the element back to CSS.
   */
  #releaseHint(): void {
    this.#surface.style.removeProperty("will-change");
  }

  settle(direction: Direction): void {
    if (this.#animation) {
      this.#token += 1;
      this.#animation.cancel();
      this.#animation = null;
    }
    this.#direction = direction;
    this.#surface.style.opacity = direction === "reveal" ? "0" : "1";
    this.#releaseHint();
  }

  toggle(reducedMotion: boolean): Direction {
    const next = this.#direction === "reveal" ? "conceal" : "reveal";
    if (reducedMotion) {
      this.settle(next);
      return next;
    }
    this.#direction = next;
    this.#surface.style.willChange = "opacity";
    if (!this.#animation) {
      this.#animation = this.#surface.animate(
        [{ opacity: 1 }, { opacity: 0 }],
        {
          duration: REVEAL_DURATION_MS,
          easing: FADE_EASING,
          fill: "both",
        },
      );
      this.#animation.playbackRate = 1;
      if (next === "conceal") {
        this.#animation.currentTime = REVEAL_DURATION_MS;
        this.#animation.playbackRate = REVEAL_DURATION_MS / CONCEAL_DURATION_MS;
        this.#animation.reverse();
      }
    } else if (next === "reveal") {
      this.#animation.playbackRate = 1;
      this.#animation.play();
    } else {
      this.#animation.playbackRate = REVEAL_DURATION_MS / CONCEAL_DURATION_MS;
      this.#animation.reverse();
    }
    const animation = this.#animation;
    const token = ++this.#token;
    void animation.finished
      .then(() => {
        if (this.#animation === animation && this.#token === token) {
          this.#releaseHint();
        }
      })
      .catch(() => {
        // Reversal/cancellation rejects `finished`; the live timeline owns state.
      });
    return next;
  }
}
