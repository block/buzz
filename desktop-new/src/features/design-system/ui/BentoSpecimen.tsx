import { BentoWorkspace } from "@/shared/ui/BentoWorkspace";
import { Section } from "./primitives";

/**
 * The workspace, with deliberately empty panels.
 *
 * Nothing is inside them because the subject here is the *arrangement* — the
 * drag, the drop targets, the gutter, the tab strip — and panel content would
 * only make that harder to judge. The gradient is not decoration either: a
 * panel's edge, shadow, and separation are only legible on the backdrop it
 * actually sits on.
 */
export function BentoSpecimen() {
  return (
    <>
      <Section
        title="Arrangement"
        description="Drag a header toward a panel edge. The highlight shows where it will land above, below, left, or right. Resize continuously in the gaps. Navigation stays at the upper left, with room for a panel below. Highlights appear only where both panels fit; stacking is off."
      >
        <div className="bento-stage">
          <BentoWorkspace
            anchoredPanelId="navigation"
            panels={[
              { id: "navigation", title: "Navigation" },
              { id: "one", title: "One" },
              { id: "two", title: "Two" },
              { id: "three", title: "Three" },
            ]}
          />
        </div>
      </Section>
    </>
  );
}
