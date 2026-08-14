import { AgentCustomCardParticleCanvas } from "./AgentCustomCardParticleCanvas";

/**
 * Quiet generative placeholder used while the custom-card prompt is being
 * written and while the existing card generator is running. The shared card
 * root supplies the exact SVG-derived outer clipping.
 */
export function AgentCustomCardDraftPreview({
  isCreating,
}: {
  isCreating: boolean;
}) {
  return (
    <div className="flex w-full justify-center">
      <div
        className="agent-trading-card-preview__stage w-full max-w-[16.5rem]"
        data-testid="agent-custom-card-waiting-stage"
      >
        <div
          aria-label="Custom agent card canvas"
          className="agent-trading-card-preview__tilt-plane"
          data-creating={isCreating}
          data-testid="agent-custom-card-waiting-preview"
          role="img"
        >
          <div className="agent-trading-card-preview agent-custom-card-waiting">
            <AgentCustomCardParticleCanvas />
          </div>
        </div>
      </div>
    </div>
  );
}
