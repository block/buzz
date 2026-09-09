import { useId, useState, type ReactNode } from "react";
import { ArrowLeft, ArrowRight } from "lucide-react";
import { IconButton } from "./button";
/** A manually advanced carousel. No autoplay; every slide has a named position. */
export function Carousel({
  label,
  slides,
}: {
  label: string;
  slides: readonly ReactNode[];
}) {
  const [index, setIndex] = useState(0);
  const id = useId();
  const current = Math.min(index, Math.max(0, slides.length - 1));
  if (!slides.length) return null;
  return (
    <section
      className="bui-stack"
      aria-label={label}
      aria-roledescription="carousel"
    >
      {/* biome-ignore lint/a11y/useSemanticElements: WAI-ARIA carousel slides are groups, not form fieldsets. */}
      <div
        id={id}
        role="group"
        aria-roledescription="slide"
        aria-label={current + 1 + " of " + slides.length}
      >
        {slides[current]}
      </div>
      <div className="bui-inline">
        <IconButton
          aria-label="Previous slide"
          aria-controls={id}
          variant="outline"
          disabled={current === 0}
          onClick={() => setIndex(current - 1)}
        >
          <ArrowLeft aria-hidden="true" />
        </IconButton>
        <span aria-live="polite" className="bui-description">
          {current + 1} / {slides.length}
        </span>
        <IconButton
          aria-label="Next slide"
          aria-controls={id}
          variant="outline"
          disabled={current === slides.length - 1}
          onClick={() => setIndex(current + 1)}
        >
          <ArrowRight aria-hidden="true" />
        </IconButton>
      </div>
    </section>
  );
}
