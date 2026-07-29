import type { DevComposerMode } from "@/features/dev-mode/lib/useDevComposerModes";
import { cn } from "@/shared/lib/cn";

/**
 * The composer's Tab-cycled target, rendered like a message's "to Name"
 * direction line rather than a pill: sits at the top of the chat box, name in
 * the agent's chat color.
 */
export function DevComposerModeLine({
  mode,
  busy,
  agentColor,
  className,
}: {
  mode: DevComposerMode;
  busy: boolean;
  agentColor: string | null;
  className?: string;
}) {
  return (
    <div
      className={cn(
        "select-none text-xs leading-4 text-muted-foreground/60",
        className,
      )}
      data-testid="dev-mode-pill"
    >
      {busy ? (
        "working…"
      ) : mode.kind === "agent" ? (
        <>
          to{" "}
          <span style={agentColor ? { color: agentColor } : undefined}>
            {mode.target.name}
          </span>
        </>
      ) : (
        "chat"
      )}
    </div>
  );
}
