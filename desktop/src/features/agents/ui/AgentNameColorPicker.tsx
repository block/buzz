import {
  AGENT_NAME_COLOR_IDS,
  getAgentNameColorStyle,
} from "@/shared/lib/agentNameColors";
import { cn } from "@/shared/lib/cn";

type AgentNameColorPickerProps = {
  disabled?: boolean;
  onChange: (nameColor: string | null) => void;
  value: string | null;
};

export function AgentNameColorPicker({
  disabled = false,
  onChange,
  value,
}: AgentNameColorPickerProps) {
  return (
    <div className="space-y-1.5">
      <label
        className="text-sm font-medium text-foreground"
        htmlFor="persona-name-color"
      >
        Name color
      </label>
      <div
        className="grid grid-cols-9 justify-items-center gap-1.5 rounded-lg bg-muted p-3"
        id="persona-name-color"
        role="radiogroup"
      >
        {/* biome-ignore lint/a11y/useSemanticElements: radio-styled swatch button, matches AgentCreationPreview's swatch grid pattern */}
        <button
          aria-checked={value === null}
          aria-label="No color"
          className={cn(
            "relative flex h-6 w-6 items-center justify-center rounded-full border border-dashed border-muted-foreground/50 text-muted-foreground transition-transform duration-150 ease-out hover:scale-[1.15] focus-visible:scale-[1.15] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
          )}
          disabled={disabled}
          onClick={() => onChange(null)}
          role="radio"
          type="button"
        >
          {value === null ? (
            <span className="h-1.5 w-1.5 rounded-full bg-current" />
          ) : null}
        </button>
        {AGENT_NAME_COLOR_IDS.map((id) => {
          const isSelected = value === id;
          return (
            // biome-ignore lint/a11y/useSemanticElements: radio-styled swatch button, matches AgentCreationPreview's swatch grid pattern
            <button
              aria-checked={isSelected}
              aria-label={`Use ${id}`}
              className={cn(
                "relative h-6 w-6 rounded-full border border-border transition-transform duration-150 ease-out hover:scale-[1.15] focus-visible:scale-[1.15] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
              )}
              disabled={disabled}
              key={id}
              onClick={() => onChange(id)}
              role="radio"
              style={{ background: getAgentNameColorStyle(id).color }}
              type="button"
            >
              {isSelected ? (
                <span className="absolute inset-0.5 rounded-full border-2 border-background" />
              ) : null}
            </button>
          );
        })}
      </div>
    </div>
  );
}
