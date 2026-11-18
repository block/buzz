import { Input } from "@/shared/ui/input";
import { FieldLabel } from "./workflowFormPrimitives";
import {
  DEFAULT_DURATION_SECONDS,
  DURATION_SLIDER_STOPS,
  durationSliderIndex,
  formatDurationSeconds,
  parseDurationSeconds,
} from "./workflowDuration";

export function WorkflowDurationField({
  disabled,
  fallbackSeconds = DEFAULT_DURATION_SECONDS,
  hideLabel = false,
  id,
  label = "Duration",
  onChange,
  placeholder = "1s",
  value,
}: {
  disabled?: boolean;
  fallbackSeconds?: number;
  hideLabel?: boolean;
  id: string;
  label?: string;
  onChange: (value: string) => void;
  placeholder?: string;
  value: string;
}) {
  const parsedSeconds = parseDurationSeconds(value);
  const sliderIndex = durationSliderIndex(
    Math.max(DURATION_SLIDER_STOPS[0], parsedSeconds ?? fallbackSeconds),
  );
  const progress = (sliderIndex / (DURATION_SLIDER_STOPS.length - 1)) * 100;
  const sliderSeconds = DURATION_SLIDER_STOPS[sliderIndex];

  return (
    <div className="space-y-1.5">
      {hideLabel ? (
        <label className="sr-only" htmlFor={id}>
          {label}
        </label>
      ) : (
        <FieldLabel htmlFor={id}>{label}</FieldLabel>
      )}
      <div className="flex items-center gap-4">
        <div className="relative flex h-9 min-w-0 flex-1 items-center">
          <div
            aria-hidden="true"
            className="pointer-events-none absolute inset-x-0 h-1.5 overflow-hidden rounded-full bg-muted"
          >
            <div
              className="h-full rounded-full bg-primary"
              style={{ width: `${progress}%` }}
            />
          </div>
          <input
            aria-label={`${label} slider`}
            aria-valuetext={formatDurationSeconds(sliderSeconds)}
            className="absolute inset-x-0 h-9 w-full cursor-pointer appearance-none bg-transparent disabled:cursor-not-allowed disabled:opacity-50 [&::-moz-range-thumb]:h-4 [&::-moz-range-thumb]:w-4 [&::-moz-range-thumb]:rounded-full [&::-moz-range-thumb]:border-2 [&::-moz-range-thumb]:border-primary [&::-moz-range-thumb]:bg-background [&::-moz-range-thumb]:shadow-sm [&::-moz-range-track]:bg-transparent [&::-webkit-slider-runnable-track]:h-1.5 [&::-webkit-slider-runnable-track]:bg-transparent [&::-webkit-slider-thumb]:mt-[-0.3125rem] [&::-webkit-slider-thumb]:h-4 [&::-webkit-slider-thumb]:w-4 [&::-webkit-slider-thumb]:appearance-none [&::-webkit-slider-thumb]:rounded-full [&::-webkit-slider-thumb]:border-2 [&::-webkit-slider-thumb]:border-primary [&::-webkit-slider-thumb]:bg-background [&::-webkit-slider-thumb]:shadow-sm focus-visible:outline-hidden focus-visible:[&::-moz-range-thumb]:ring-2 focus-visible:[&::-moz-range-thumb]:ring-ring focus-visible:[&::-moz-range-thumb]:ring-offset-2 focus-visible:[&::-webkit-slider-thumb]:ring-2 focus-visible:[&::-webkit-slider-thumb]:ring-ring focus-visible:[&::-webkit-slider-thumb]:ring-offset-2"
            disabled={disabled}
            max={DURATION_SLIDER_STOPS.length - 1}
            min={0}
            onChange={(event) => {
              const seconds = DURATION_SLIDER_STOPS[Number(event.target.value)];
              onChange(formatDurationSeconds(seconds));
            }}
            type="range"
            value={sliderIndex}
          />
        </div>
        <Input
          autoCapitalize="off"
          autoCorrect="off"
          className="w-28 shrink-0 text-center tabular-nums"
          disabled={disabled}
          id={id}
          onBlur={() => {
            if (parsedSeconds !== null) {
              onChange(
                formatDurationSeconds(
                  Math.max(DURATION_SLIDER_STOPS[0], parsedSeconds),
                ),
              );
            }
          }}
          onChange={(event) => onChange(event.target.value)}
          placeholder={placeholder}
          spellCheck={false}
          value={value}
        />
      </div>
    </div>
  );
}
